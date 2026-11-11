//! Header detection and structure-aware length measurement for carved formats.

use crate::io::Src;
use crate::util::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Riff,
    Tiff,
    Bmff,
}

impl Kind {
    /// User-facing type group used for filtering carving.
    pub fn group(&self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
            Kind::Gif => "gif",
            Kind::Bmp => "bmp",
            Kind::Riff => "webp", // refined after measure; webp/avi share the group check below
            Kind::Tiff => "raw",
            Kind::Bmff => "mp4",
        }
    }
}

pub struct Carved {
    pub len: u64,
    pub ext: &'static str,
    pub prefix: &'static str,
    pub created: Option<i64>,
    pub complete: bool,
}

pub fn detect(b: &[u8]) -> Option<Kind> {
    match b[0] {
        0xFF if b[1] == 0xD8 && b[2] == 0xFF && (b[3] >= 0xC0) => Some(Kind::Jpeg),
        0x89 if &b[1..8] == b"PNG\r\n\x1a\n" => Some(Kind::Png),
        b'G' if &b[..6] == b"GIF89a" || &b[..6] == b"GIF87a" => Some(Kind::Gif),
        b'B' if b[1] == b'M' && le32(b, 6) == 0 && matches!(le32(b, 14), 12 | 40 | 52 | 56 | 108 | 124) => Some(Kind::Bmp),
        b'R' if &b[..4] == b"RIFF" && (&b[8..12] == b"WEBP" || &b[8..12] == b"AVI ") => Some(Kind::Riff),
        b'I' if &b[..4] == b"II*\0" => Some(Kind::Tiff),
        b'M' if &b[..4] == b"MM\0*" => Some(Kind::Tiff),
        0 if &b[4..8] == b"ftyp" && (8..=512).contains(&be32(b, 0)) && b[8..12].iter().all(|c| c.is_ascii_graphic() || *c == b' ') => Some(Kind::Bmff),
        _ => None,
    }
}

/// Windowed reader over a source.
struct Win<'a> {
    src: &'a Src,
    base: u64,
    buf: Vec<u8>,
    start: u64,
}

impl<'a> Win<'a> {
    fn new(src: &'a Src, base: u64) -> Self {
        Win { src, base, buf: Vec::new(), start: u64::MAX }
    }
    /// Slice at relative offset `o` of length `n` (n <= 1 MiB).
    fn get(&mut self, o: u64, n: usize) -> Option<&[u8]> {
        if self.start == u64::MAX || o < self.start || o + n as u64 > self.start + self.buf.len() as u64 {
            self.start = o;
            self.buf = self.src.read_upto(self.base + o, (1 << 20).max(n));
        }
        let s = (o - self.start) as usize;
        self.buf.get(s..s + n)
    }
    fn u8(&mut self, o: u64) -> Option<u8> {
        self.get(o, 1).map(|b| b[0])
    }
    fn be16(&mut self, o: u64) -> Option<u16> {
        self.get(o, 2).map(|b| be16(b, 0))
    }
    fn be32(&mut self, o: u64) -> Option<u32> {
        self.get(o, 4).map(|b| be32(b, 0))
    }
    fn le32(&mut self, o: u64) -> Option<u32> {
        self.get(o, 4).map(|b| le32(b, 0))
    }
    /// Find the next 0xFF at or after `o`, within `limit`.
    fn find_ff(&mut self, mut o: u64, limit: u64) -> Option<u64> {
        while o < limit {
            let n = ((limit - o).min(1 << 20)) as usize;
            let b = self.get(o, n.min(1 << 20)).map(|b| b.to_vec()).or_else(|| {
                // Near the end of the source: take what is available.
                let avail = self.src.len().saturating_sub(self.base + o) as usize;
                if avail == 0 {
                    None
                } else {
                    self.get(o, avail.min(n))
                        .map(|b| b.to_vec())
                }
            })?;
            if let Some(i) = memchr::memchr(0xFF, &b) {
                return Some(o + i as u64);
            }
            o += b.len() as u64;
        }
        None
    }
}

pub fn measure(src: &Src, at: u64, kind: Kind) -> Option<Carved> {
    match kind {
        Kind::Jpeg => jpeg(src, at),
        Kind::Png => png(src, at),
        Kind::Gif => gif(src, at),
        Kind::Bmp => bmp(src, at),
        Kind::Riff => riff(src, at),
        Kind::Tiff => tiff(src, at),
        Kind::Bmff => bmff(src, at),
    }
}

const MB: u64 = 1 << 20;

fn jpeg(src: &Src, at: u64) -> Option<Carved> {
    let mut w = Win::new(src, at);
    let cap = 80 * MB;
    let mut p = 2u64;
    let mut created = None;
    let mut saw_sos = false;
    loop {
        if p >= cap {
            return None;
        }
        if w.u8(p)? != 0xFF {
            return None;
        }
        while w.u8(p)? == 0xFF {
            p += 1;
        }
        let m = w.u8(p)?;
        p += 1;
        match m {
            0xD9 => {
                return if saw_sos { Some(Carved { len: p, ext: "jpg", prefix: "IMG", created, complete: true }) } else { None };
            }
            0xD8 | 0x00 => return None,
            0x01 | 0xD0..=0xD7 => continue,
            _ => {
                let len = w.be16(p)? as u64;
                if len < 2 {
                    return None;
                }
                if m == 0xE1 && created.is_none() {
                    if let Some(seg) = w.get(p + 2, (len as usize).saturating_sub(2)).map(|s| s.to_vec()) {
                        if seg.starts_with(b"Exif\0\0") {
                            created = exif_created(&seg[6..]);
                        }
                    }
                }
                p += len;
                if m == 0xDA {
                    saw_sos = true;
                    // Entropy-coded data: find the next real marker.
                    loop {
                        let ff = w.find_ff(p, cap)?;
                        let nx = w.u8(ff + 1)?;
                        if nx == 0x00 || (0xD0..=0xD7).contains(&nx) || nx == 0xFF {
                            p = ff + if nx == 0xFF { 1 } else { 2 };
                            continue;
                        }
                        p = ff;
                        break;
                    }
                }
            }
        }
    }
}

fn png(src: &Src, at: u64) -> Option<Carved> {
    let mut w = Win::new(src, at);
    let mut p = 8u64;
    for _ in 0..100_000 {
        let len = w.be32(p)? as u64;
        let t = w.get(p + 4, 4)?.to_vec();
        if len > 0x7FFF_FFFF || !t.iter().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        p += 12 + len;
        if &t == b"IEND" {
            return Some(Carved { len: p, ext: "png", prefix: "IMG", created: None, complete: true });
        }
        if p > 512 * MB {
            return None;
        }
    }
    None
}

fn gif(src: &Src, at: u64) -> Option<Carved> {
    let mut w = Win::new(src, at);
    let flags = w.u8(10)?;
    let mut p = 13u64;
    if flags & 0x80 != 0 {
        p += 3 * (1u64 << ((flags & 7) + 1));
    }
    let skip_sub = |w: &mut Win, mut p: u64| -> Option<u64> {
        loop {
            let n = w.u8(p)? as u64;
            p += 1;
            if n == 0 {
                return Some(p);
            }
            p += n;
            if p > 256 * MB {
                return None;
            }
        }
    };
    for _ in 0..1_000_000 {
        match w.u8(p)? {
            0x21 => p = skip_sub(&mut w, p + 2)?,
            0x2C => {
                let packed = w.u8(p + 9)?;
                p += 10;
                if packed & 0x80 != 0 {
                    p += 3 * (1u64 << ((packed & 7) + 1));
                }
                p = skip_sub(&mut w, p + 1)?;
            }
            0x3B => return Some(Carved { len: p + 1, ext: "gif", prefix: "IMG", created: None, complete: true }),
            _ => return None,
        }
    }
    None
}

fn bmp(src: &Src, at: u64) -> Option<Carved> {
    let mut w = Win::new(src, at);
    let size = w.le32(2)? as u64;
    let data = w.le32(10)? as u64;
    if size < 64 || size > 256 * MB || data >= size {
        return None;
    }
    Some(Carved { len: size, ext: "bmp", prefix: "IMG", created: None, complete: true })
}

fn riff(src: &Src, at: u64) -> Option<Carved> {
    let mut w = Win::new(src, at);
    let size = w.le32(4)? as u64 + 8;
    let t = w.get(8, 4)?.to_vec();
    if size < 64 || size > 4096 * MB {
        return None;
    }
    let (ext, prefix) = if &t == b"WEBP" { ("webp", "IMG") } else { ("avi", "VID") };
    let len = size + (size & 1);
    let complete = at + len <= src.len();
    Some(Carved { len: len.min(src.len() - at), ext, prefix, created: None, complete })
}

/// TIFF / camera RAW: the file length is the furthest byte referenced by any IFD.
fn tiff(src: &Src, at: u64) -> Option<Carved> {
    let head = src.read_upto(at, 16);
    let cr2 = head.get(8..10) == Some(&b"CR"[..]);
    let info = tiff_walk(&mut Win::new(src, at), 0, 512 * MB)?;
    if info.end < 2048 {
        return None;
    }
    let ext = if cr2 {
        "cr2"
    } else if info.dng {
        "dng"
    } else {
        let m = info.make.to_ascii_uppercase();
        if m.contains("NIKON") {
            "nef"
        } else if m.contains("SONY") {
            "arw"
        } else if m.contains("PENTAX") {
            "pef"
        } else if m.contains("CANON") {
            "cr2"
        } else {
            "tif"
        }
    };
    Some(Carved { len: info.end, ext, prefix: if ext == "tif" { "IMG" } else { "RAW" }, created: info.date, complete: true })
}

#[derive(Default)]
pub struct TiffInfo {
    pub end: u64,
    pub make: String,
    pub dng: bool,
    pub date: Option<i64>,
    /// Embedded JPEG previews (offset, length), relative to the TIFF header.
    pub jpegs: Vec<(u64, u64)>,
}

fn tiff_walk(w: &mut Win, base: u64, cap: u64) -> Option<TiffInfo> {
    let bo = w.get(base, 4)?.to_vec();
    let le = match &bo[..] {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let r16 = |w: &mut Win, o: u64| -> Option<u64> { w.get(base + o, 2).map(|b| if le { le16(b, 0) } else { be16(b, 0) } as u64) };
    let r32 = |w: &mut Win, o: u64| -> Option<u64> { w.get(base + o, 4).map(|b| if le { le32(b, 0) } else { be32(b, 0) } as u64) };
    let mut info = TiffInfo { end: 8, ..Default::default() };
    let first = r32(w, 4)?;
    let mut queue = vec![first];
    let mut seen = std::collections::HashSet::new();
    while let Some(ifd) = queue.pop() {
        if ifd < 8 || ifd > cap || !seen.insert(ifd) || seen.len() > 64 {
            continue;
        }
        let n = r16(w, ifd)?;
        if n == 0 || n > 1000 {
            continue;
        }
        info.end = info.end.max(ifd + 2 + n * 12 + 4);
        let (mut strips, mut counts, mut jpeg_off, mut jpeg_len) = (Vec::new(), Vec::new(), 0u64, 0u64);
        for i in 0..n {
            let e = ifd + 2 + i * 12;
            let tag = r16(w, e)?;
            let typ = r16(w, e + 2)?;
            let cnt = r32(w, e + 4)?;
            let tsz = match typ {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => continue,
            };
            let bytes = cnt.saturating_mul(tsz);
            let voff = if bytes > 4 { r32(w, e + 8)? } else { e + 8 };
            if bytes > 4 && voff + bytes <= cap {
                info.end = info.end.max(voff + bytes);
            }
            let read_vals = |w: &mut Win| -> Vec<u64> {
                (0..cnt.min(100_000))
                    .filter_map(|k| if tsz == 2 { r16(w, voff + k * 2) } else { r32(w, voff + k * 4) })
                    .collect()
            };
            match tag {
                0x0111 | 0x0144 => strips = read_vals(w),
                0x0117 | 0x0145 => counts = read_vals(w),
                0x0201 => jpeg_off = r32(w, voff).unwrap_or(0),
                0x0202 => jpeg_len = r32(w, voff).unwrap_or(0),
                0x014A | 0x8769 => queue.extend(read_vals(w)),
                0x010F => {
                    if let Some(s) = w.get(base + voff, cnt.min(64) as usize) {
                        info.make = String::from_utf8_lossy(s).trim_end_matches('\0').to_string();
                    }
                }
                0xC612 => info.dng = true,
                0x9003 | 0x0132 => {
                    if info.date.is_none() || tag == 0x9003 {
                        if let Some(s) = w.get(base + voff, 19) {
                            if let Some(d) = exif_date(&String::from_utf8_lossy(s)) {
                                info.date = Some(d);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for (s, c) in strips.iter().zip(counts.iter()) {
            if s + c <= cap {
                info.end = info.end.max(s + c);
            }
        }
        if jpeg_off > 0 && jpeg_len > 0 && jpeg_off + jpeg_len <= cap {
            info.end = info.end.max(jpeg_off + jpeg_len);
            info.jpegs.push((jpeg_off, jpeg_len));
        }
        // Single-strip JPEG-compressed previews are also common in RAW files.
        if strips.len() == 1 && counts.len() == 1 {
            if let Some(h) = w.get(base + strips[0], 2) {
                if h == [0xFF, 0xD8] {
                    info.jpegs.push((strips[0], counts[0]));
                }
            }
        }
        if let Some(next) = r32(w, ifd + 2 + n * 12) {
            if next != 0 {
                queue.push(next);
            }
        }
    }
    Some(info)
}

fn exif_created(tiff: &[u8]) -> Option<i64> {
    let src: Src = std::sync::Arc::new(crate::io::MemSrc(tiff.to_vec()));
    let mut w = Win::new(&src, 0);
    tiff_walk(&mut w, 0, tiff.len() as u64)?.date
}

/// Public helper used by previews: embedded JPEGs inside a TIFF/RAW file (absolute offsets).
pub fn tiff_previews(src: &Src, at: u64) -> Vec<(u64, u64)> {
    let mut w = Win::new(src, at);
    tiff_walk(&mut w, 0, 512 * MB).map(|i| i.jpegs.into_iter().map(|(o, l)| (at + o, l)).collect()).unwrap_or_default()
}

const TOP_BOXES: &[&[u8; 4]] = &[
    b"ftyp", b"moov", b"mdat", b"free", b"skip", b"wide", b"meta", b"uuid", b"pnot", b"moof", b"mfra", b"styp", b"sidx", b"pdin",
    b"prft", b"emsg", b"udta", b"junk", b"PICT", b"idat", b"jumb",
];

fn bmff(src: &Src, at: u64) -> Option<Carved> {
    let mut w = Win::new(src, at);
    let brand = w.get(8, 4)?.to_vec();
    let mut p = 0u64;
    let (mut moov, mut mdat, mut meta) = (false, false, false);
    let mut created = None;
    let cap = 8192 * MB;
    for _ in 0..100_000 {
        let Some(hdr) = w.get(p, 16).map(|b| b.to_vec()) else { break };
        let mut size = be32(&hdr, 0) as u64;
        let typ: [u8; 4] = hdr[4..8].try_into().unwrap();
        if !TOP_BOXES.contains(&&typ) {
            break;
        }
        if size == 1 {
            size = be64(&hdr, 8);
        } else if size == 0 {
            break;
        }
        if size < 8 || p + size > cap {
            break;
        }
        match &typ {
            b"moov" => {
                moov = true;
                // mvhd is normally the first child.
                if let Some(c) = w.get(p + 8, 64).map(|b| b.to_vec()) {
                    if &c[4..8] == b"mvhd" {
                        let t = if c[8] == 1 { be64(&c, 12) } else { be32(&c, 12) as u64 };
                        created = plausible(t as i64 - 2_082_844_800);
                    }
                }
            }
            b"mdat" => mdat = true,
            b"meta" => meta = true,
            _ => {}
        }
        p += size;
    }
    if p < 64 || !mdat || !(moov || meta) {
        // A file whose moov comes after a truncated mdat cannot be measured reliably.
        if mdat && p > 64 && !moov && !meta {
            let ext = brand_ext(&brand, false);
            return Some(Carved { len: p.min(src.len() - at), ext, prefix: "VID", created, complete: false });
        }
        return None;
    }
    let ext = brand_ext(&brand, meta && !moov);
    let prefix = if matches!(ext, "heic" | "avif") { "IMG" } else if ext == "cr3" { "RAW" } else { "VID" };
    Some(Carved { len: p, ext, prefix, created, complete: true })
}

fn brand_ext(b: &[u8], still: bool) -> &'static str {
    match b {
        b"qt  " => "mov",
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" if still => "heic",
        b"avif" | b"avis" => "avif",
        b"crx " => "cr3",
        b"M4V " | b"M4VH" | b"M4VP" => "m4v",
        b"3gp4" | b"3gp5" | b"3gp6" | b"3g2a" | b"3ge6" | b"3gs7" => "3gp",
        _ if still => "heic",
        _ => "mp4",
    }
}

/// Map a carved extension to the carving type group used in options.
pub fn ext_group(ext: &str) -> &'static str {
    match ext {
        "jpg" => "jpg",
        "png" => "png",
        "gif" => "gif",
        "bmp" => "bmp",
        "webp" => "webp",
        "avi" => "avi",
        "tif" => "tif",
        "cr2" | "nef" | "arw" | "dng" | "pef" | "cr3" => "raw",
        "heic" | "avif" => "heic",
        "mov" => "mov",
        "3gp" => "3gp",
        _ => "mp4",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn png_and_jpeg() {
        // Minimal PNG: signature + IHDR + IEND
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend([0, 0, 0, 13]);
        png.extend(b"IHDR");
        png.extend([0u8; 13 + 4]);
        png.extend([0, 0, 0, 0]);
        png.extend(b"IEND");
        png.extend([0u8; 4]);
        let plen = png.len() as u64;
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xDA, 0, 2, 1, 2, 3, 0xFF, 0x00, 4, 0xFF, 0xD9];
        let jlen = jpg.len() as u64;
        let mut disk = vec![0u8; 512];
        disk.extend(&png);
        disk.resize(1024, 0);
        disk.append(&mut jpg);
        disk.resize(4096, 0);
        let src: Src = Arc::new(crate::io::MemSrc(disk.clone()));
        assert_eq!(detect(&disk[512..]), Some(Kind::Png));
        assert_eq!(measure(&src, 512, Kind::Png).unwrap().len, plen);
        assert_eq!(detect(&disk[1024..]), Some(Kind::Jpeg));
        assert_eq!(measure(&src, 1024, Kind::Jpeg).unwrap().len, jlen);
    }
}
