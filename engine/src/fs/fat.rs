//! FAT12 / FAT16 / FAT32: deleted directory entries (0xE5), long-file-name reconstruction and
//! contiguous-cluster recovery (FAT chains are zeroed on delete).

use super::{FsKind, Volume};
use crate::io::{Cached, Src};
use crate::model::*;
use crate::util::*;
use std::collections::HashSet;

pub struct Fat {
    src: Src,
    raw: Src,
    bits: u8,
    cluster: u64,
    data_off: u64,
    clusters: u64, // number of data clusters
    root_off: u64,
    root_len: u64,
    root_cluster: u32,
    fat: Vec<u8>,
}

enum DirLoc {
    Fixed(u64, u64),
    Chain(u32),
    Single(u32),
}

impl Fat {
    pub fn open(raw: Src) -> anyhow::Result<Self> {
        let src = Cached::new(raw.clone());
        let b = src.read_vec(0, 512)?;
        let bps = le16(&b, 11) as u64;
        let spc = b[13] as u64;
        let reserved = le16(&b, 14) as u64;
        let nfats = b[16] as u64;
        let root_entries = le16(&b, 17) as u64;
        let total = if le16(&b, 19) != 0 { le16(&b, 19) as u64 } else { le32(&b, 32) as u64 };
        let fatsz = if le16(&b, 22) != 0 { le16(&b, 22) as u64 } else { le32(&b, 36) as u64 };
        if bps == 0 || spc == 0 || fatsz == 0 {
            anyhow::bail!("FAT: invalid boot sector");
        }
        let root_secs = (root_entries * 32).div_ceil(bps);
        let data_sec = reserved + nfats * fatsz + root_secs;
        let clusters = total.saturating_sub(data_sec) / spc;
        let bits = if clusters < 4085 { 12 } else if clusters < 65525 { 16 } else { 32 };
        let fat_bytes = fatsz * bps;
        if fat_bytes > 512 << 20 {
            anyhow::bail!("FAT: allocation table too large");
        }
        let fat = src.read_vec(reserved * bps, fat_bytes as usize)?;
        Ok(Fat {
            src,
            raw,
            bits,
            cluster: bps * spc,
            data_off: data_sec * bps,
            clusters,
            root_off: (reserved + nfats * fatsz) * bps,
            root_len: root_secs * bps,
            root_cluster: if bits == 32 { le32(&b, 44) } else { 0 },
            fat,
        })
    }

    pub fn entry(&self, n: u32) -> u32 {
        let n = n as usize;
        match self.bits {
            12 => {
                let v = le16(&self.fat, n + n / 2) as u32;
                if n & 1 == 1 {
                    v >> 4
                } else {
                    v & 0xFFF
                }
            }
            16 => le16(&self.fat, n * 2) as u32,
            _ => le32(&self.fat, n * 4) & 0x0FFF_FFFF,
        }
    }

    fn valid(&self, c: u32) -> bool {
        c >= 2 && (c as u64) < self.clusters + 2
    }

    fn is_eoc(&self, v: u32) -> bool {
        match self.bits {
            12 => v >= 0xFF8,
            16 => v >= 0xFFF8,
            _ => v >= 0x0FFF_FFF8,
        }
    }

    fn chain(&self, start: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = start;
        let mut seen = HashSet::new();
        while self.valid(c) && seen.insert(c) && out.len() < 1 << 22 {
            out.push(c);
            let n = self.entry(c);
            if self.is_eoc(n) || n == 0 {
                break;
            }
            c = n;
        }
        out
    }

    fn coff(&self, c: u32) -> u64 {
        self.data_off + (c as u64 - 2) * self.cluster
    }

    fn read_dir(&self, loc: &DirLoc) -> Vec<u8> {
        match *loc {
            DirLoc::Fixed(o, l) => self.src.read_upto(o, l as usize),
            DirLoc::Chain(c) => {
                let mut v = Vec::new();
                for c in self.chain(c).into_iter().take(65536) {
                    v.extend(self.src.read_upto(self.coff(c), self.cluster as usize));
                }
                v
            }
            DirLoc::Single(c) => {
                // Chain is gone: take following free clusters while they still look like directory entries.
                let mut v = self.src.read_upto(self.coff(c), self.cluster as usize);
                let mut n = c + 1;
                while self.valid(n) && self.entry(n) == 0 && v.len() < (1 << 20) && !v.chunks_exact(32).any(|e| e[0] == 0) {
                    let next = self.src.read_upto(self.coff(n), self.cluster as usize);
                    if !looks_like_dir(&next) {
                        break;
                    }
                    v.extend(next);
                    n += 1;
                }
                v
            }
        }
    }

    fn free_clusters_in(&self, start: u32, n: u64) -> u64 {
        (0..n).filter(|&i| self.valid(start + i as u32) && self.entry(start + i as u32) == 0).count() as u64
    }
}

fn looks_like_dir(b: &[u8]) -> bool {
    let mut good = 0;
    for e in b.chunks_exact(32).take(16) {
        if e[0] == 0 {
            break;
        }
        let attr = e[11];
        let ok = if attr == 0x0F {
            e[12] == 0 && le16(e, 26) == 0
        } else {
            attr & 0xC0 == 0 && e[1..11].iter().all(|&c| c >= 0x20 && c != 0x7F)
        };
        if !ok {
            return false;
        }
        good += 1;
    }
    good > 0
}

fn sfn_checksum(n: &[u8]) -> u8 {
    let mut s: u8 = 0;
    for &c in &n[..11] {
        s = (s >> 1 | (s & 1) << 7).wrapping_add(c);
    }
    s
}

fn sfn_name(e: &[u8], first: u8) -> String {
    let mut base: Vec<u8> = e[0..8].to_vec();
    base[0] = first;
    let mut base = String::from_utf8_lossy(&base).trim_end().to_string();
    let mut ext = String::from_utf8_lossy(&e[8..11]).trim_end().to_string();
    // Windows NT / macOS store all-lowercase 8.3 names with case flags instead of an LFN.
    if e[12] & 0x08 != 0 {
        base = base.to_lowercase();
    }
    if e[12] & 0x10 != 0 {
        ext = ext.to_lowercase();
    }
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

fn lfn_chars(e: &[u8]) -> Vec<u16> {
    let mut v = Vec::with_capacity(13);
    for r in [(1usize, 11usize), (14, 26), (28, 32)] {
        for i in (r.0..r.1).step_by(2) {
            v.push(le16(e, i));
        }
    }
    v
}

fn magic_ok(ext: &str, head: &[u8]) -> Option<bool> {
    let m: &[u8] = match ext {
        "jpg" | "jpeg" => &[0xFF, 0xD8, 0xFF],
        "png" => b"\x89PNG",
        "gif" => b"GIF8",
        "pdf" => b"%PDF",
        "zip" | "docx" | "xlsx" | "pptx" => b"PK\x03\x04",
        "mp4" | "mov" | "m4v" | "heic" => return Some(head.get(4..8) == Some(b"ftyp")),
        _ => return None,
    };
    Some(head.starts_with(m))
}

impl Volume for Fat {
    fn kind(&self) -> FsKind {
        FsKind::Fat
    }

    fn scan_deleted(&self, ctx: &ScanCtx, label: &str) -> anyhow::Result<()> {
        let fsname = format!("FAT{}", self.bits);
        ctx.phase(format!("{fsname}: reading directories"), 0);
        let root = if self.bits == 32 { DirLoc::Chain(self.root_cluster) } else { DirLoc::Fixed(self.root_off, self.root_len) };
        let mut stack: Vec<(DirLoc, String)> = vec![(root, String::new())];
        let mut visited: HashSet<u32> = HashSet::new();
        let mut dir_paths: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        let mut dirs_done = 0u64;
        let mut orphan_pass = false;
        let mut seen_files: HashSet<(u64, u64)> = HashSet::new();
        loop {
            let Some((loc, path)) = stack.pop() else {
                if orphan_pass || ctx.cancelled() {
                    break;
                }
                // Second pass: free clusters that still hold directory fragments whose FAT chain was
                // erased (common when a deleted folder spanned several clusters).
                orphan_pass = true;
                let total = self.clusters.min(8 << 20) as u32;
                ctx.phase(format!("{fsname}: looking for orphaned folder fragments"), total as u64);
                let mut used: HashSet<u32> = visited.clone();
                for c in 2..total + 2 {
                    if c % 4096 == 0 {
                        ctx.set_done(c as u64);
                        if ctx.cancelled() {
                            break;
                        }
                    }
                    if self.entry(c) != 0 || used.contains(&c) {
                        continue;
                    }
                    let head = self.raw.read_upto(self.coff(c), 512);
                    if head.first().copied() != Some(0xE5) || !looks_like_dir(&head) {
                        continue;
                    }
                    used.insert(c);
                    let p = dir_paths.get(&(c - 1)).cloned().unwrap_or_else(|| format!("/$OrphanFolder{c}"));
                    stack.push((DirLoc::Single(c), p));
                }
                continue;
            };
            if let DirLoc::Single(c) | DirLoc::Chain(c) = loc {
                dir_paths.insert(c, path.clone());
                visited.insert(c);
            }
            if ctx.cancelled() {
                break;
            }
            dirs_done += 1;
            ctx.set_done(dirs_done);
            let data = self.read_dir(&loc);
            let mut lfn: Vec<Vec<u8>> = Vec::new();
            for e in data.chunks_exact(32) {
                let b0 = e[0];
                if b0 == 0 {
                    break;
                }
                let attr = e[11];
                if attr == 0x0F {
                    lfn.push(e.to_vec());
                    continue;
                }
                let deleted = b0 == 0xE5;
                let pending = std::mem::take(&mut lfn);
                if attr & 0x08 != 0 || &e[0..2] == b". " || &e[0..3] == b"..\x20" || e[0..2] == [b'.', b'.'] {
                    continue;
                }
                // Resolve the name: LFN parts are stored in reverse order before the 8.3 entry.
                let mut name = None;
                let mut first = if b0 == 0x05 { 0xE5 } else { b0 };
                if !pending.is_empty() {
                    let ck = pending[0][13];
                    if pending.iter().all(|p| p[13] == ck) {
                        let ok = if deleted {
                            // The first byte was overwritten with 0xE5: brute-force it from the checksum.
                            let mut sfn = e[0..11].to_vec();
                            (0u8..=255).find(|&c| {
                                sfn[0] = c;
                                sfn_checksum(&sfn) == ck
                            })
                            .map(|c| first = c)
                            .is_some()
                        } else {
                            sfn_checksum(e) == ck
                        };
                        if ok {
                            let mut units = Vec::new();
                            for p in pending.iter().rev() {
                                units.extend(lfn_chars(p));
                            }
                            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
                            name = Some(String::from_utf16_lossy(&units[..end]));
                        }
                    }
                }
                let name = name.unwrap_or_else(|| sfn_name(e, if deleted && first == 0xE5 { b'_' } else { first }));
                let hi = if self.bits == 32 { le16(e, 20) as u32 } else { 0 };
                let start = hi << 16 | le16(e, 26) as u32;
                let size = le32(e, 28) as u64;
                let child_path = format!("{path}/{name}");

                if attr & 0x10 != 0 {
                    if self.valid(start) && visited.insert(start) {
                        let loc = if deleted { DirLoc::Single(start) } else { DirLoc::Chain(start) };
                        if deleted {
                            // Only follow a deleted directory if its first cluster still looks like one.
                            let head = self.src.read_upto(self.coff(start), 32);
                            if head.get(0..2) != Some(b". ") {
                                continue;
                            }
                        }
                        stack.push((loc, child_path));
                    }
                    continue;
                }
                if !deleted || size == 0 {
                    continue;
                }
                let n = size.div_ceil(self.cluster);
                // FAT32 deletion often clears the high word of the start cluster: try candidates.
                let ext = ext_of(&name);
                let mut cands: Vec<u32> = vec![start];
                if self.bits == 32 && hi == 0 {
                    let maxhi = ((self.clusters + 2) >> 16) as u32;
                    cands.extend((1..=maxhi).map(|h| h << 16 | start));
                }
                cands.retain(|&c| self.valid(c) && (c as u64 + n) <= self.clusters + 2);
                if cands.is_empty() {
                    continue;
                }
                let pick = cands
                    .iter()
                    .copied()
                    .find(|&c| magic_ok(&ext, &self.src.read_upto(self.coff(c), 16)) == Some(true))
                    .or_else(|| cands.iter().copied().find(|&c| self.free_clusters_in(c, n) == n))
                    .unwrap_or(cands[0]);
                if !seen_files.insert((self.coff(pick), size)) {
                    continue;
                }
                let free = self.free_clusters_in(pick, n);
                let mut it = FileItem::new(
                    name,
                    if path.is_empty() { "/".into() } else { path.clone() },
                    size,
                    Method::Metadata,
                    Location::Extents { src: self.raw.clone(), runs: vec![Run { off: Some(self.coff(pick)), len: size }] },
                );
                it.created = dos_to_unix(le16(e, 16), le16(e, 14));
                it.modified = dos_to_unix(le16(e, 24), le16(e, 22));
                it.score = score_from(free, n);
                if magic_ok(&ext, &self.src.read_upto(self.coff(pick), 16)) == Some(false) {
                    it.score = Score::Overwritten;
                }
                it.fs = fsname.clone();
                it.source = label.into();
                it.note = "FAT keeps no deletion date; data assumed contiguous".into();
                ctx.add(it);
            }
        }
        Ok(())
    }

    fn free_ranges(&self) -> Option<Vec<(u64, u64)>> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut c = 2u32;
        let end = (self.clusters + 2) as u32;
        while c < end {
            if self.entry(c) == 0 {
                let s = c;
                while c < end && self.entry(c) == 0 {
                    c += 1;
                }
                out.push((self.coff(s), (c - s) as u64 * self.cluster));
            } else {
                c += 1;
            }
        }
        Some(out)
    }
}
