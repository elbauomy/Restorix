//! Byte readers, timestamp conversions and file-type helpers.

use crate::model::Category;

#[inline]
fn take<const N: usize>(b: &[u8], o: usize) -> [u8; N] {
    let mut a = [0u8; N];
    if let Some(s) = b.get(o..o.saturating_add(N)) {
        a.copy_from_slice(s);
    }
    a
}

#[inline]
pub fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(take::<2>(b, o))
}
#[inline]
pub fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(take::<4>(b, o))
}
#[inline]
pub fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(take::<8>(b, o))
}
#[inline]
pub fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes(take::<2>(b, o))
}
#[inline]
pub fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(take::<4>(b, o))
}
#[inline]
pub fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(take::<8>(b, o))
}
#[inline]
pub fn byte(b: &[u8], o: usize) -> u8 {
    b.get(o).copied().unwrap_or(0)
}

/// Decode UTF-16LE (stops at the first NUL).
pub fn utf16le(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

pub fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Days since 1970-01-01 for a proleptic Gregorian civil date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_to_unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 61 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s)
}

/// Sanity window for timestamps: 1980..2100.
pub fn plausible(t: i64) -> Option<i64> {
    if (315_532_800..4_102_444_800).contains(&t) {
        Some(t)
    } else {
        None
    }
}

/// Windows FILETIME (100ns since 1601) -> unix seconds.
pub fn filetime_to_unix(ft: u64) -> Option<i64> {
    if ft == 0 {
        return None;
    }
    plausible((ft / 10_000_000) as i64 - 11_644_473_600)
}

/// HFS+ seconds since 1904 -> unix seconds.
pub fn hfs_to_unix(t: u32) -> Option<i64> {
    if t == 0 {
        return None;
    }
    plausible(t as i64 - 2_082_844_800)
}

/// APFS nanoseconds since 1970 -> unix seconds.
pub fn ns_to_unix(ns: u64) -> Option<i64> {
    if ns == 0 {
        return None;
    }
    plausible((ns / 1_000_000_000) as i64)
}

/// MS-DOS packed date + time -> unix seconds (local time, treated as UTC).
pub fn dos_to_unix(date: u16, time: u16) -> Option<i64> {
    if date == 0 {
        return None;
    }
    let y = 1980 + (date >> 9) as i64;
    let mo = ((date >> 5) & 0xF) as i64;
    let d = (date & 0x1F) as i64;
    let h = (time >> 11) as i64;
    let mi = ((time >> 5) & 0x3F) as i64;
    let s = ((time & 0x1F) * 2) as i64;
    civil_to_unix(y, mo, d, h, mi, s).and_then(plausible)
}

/// "YYYY:MM:DD HH:MM:SS" (EXIF) -> unix seconds.
pub fn exif_date(s: &str) -> Option<i64> {
    let s = s.trim_matches(char::from(0)).trim();
    if s.len() < 19 {
        return None;
    }
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    civil_to_unix(n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?).and_then(plausible)
}

pub fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && e.len() <= 10 && !e.contains(' ') => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

pub fn category_of(ext: &str) -> Category {
    match ext {
        "jpg" | "jpeg" | "jpe" | "png" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "webp" | "avif" | "cr2"
        | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" | "pef" | "srw" | "psd" | "svg" | "ico" => {
            Category::Image
        }
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "wmv" | "3gp" | "3g2" | "mts" | "m2ts" | "webm" | "flv" | "mpg"
        | "mpeg" | "vob" => Category::Video,
        "mp3" | "m4a" | "wav" | "aac" | "flac" | "ogg" | "wma" | "aiff" | "aif" | "opus" => Category::Audio,
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pdf" | "txt" | "rtf" | "odt" | "ods" | "odp" | "csv"
        | "md" | "pages" | "numbers" | "key" | "html" | "htm" | "json" | "xml" | "eml" | "msg" | "pst" => {
            Category::Document
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "dmg" | "iso" => Category::Archive,
        _ => Category::Other,
    }
}

pub fn is_text_ext(ext: &str) -> bool {
    matches!(
        ext,
        "txt" | "csv" | "md" | "json" | "xml" | "html" | "htm" | "log" | "ini" | "cfg" | "conf" | "yaml" | "yml"
            | "js" | "ts" | "py" | "rs" | "c" | "h" | "cpp" | "java" | "sh" | "bat" | "ps1" | "css" | "sql" | "eml"
            | "rtf" | "srt"
    )
}

pub fn mime_of(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Glob-style match supporting `*` and `?`, case-insensitive (inputs must be lowercase).
pub fn glob_match(pat: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while t < text.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = p;
            mark = t;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

/// Make a single path component safe on Windows and macOS.
pub fn sanitize_component(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        out.push('_');
    }
    let upper = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    if reserved.contains(&upper.as_str()) {
        out.insert(0, '_');
    }
    if out.len() > 200 {
        let mut cut = 200;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}

pub fn fmt_unix(t: i64) -> String {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil_from_days
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m, d, secs / 3600, (secs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(civil_to_unix(2024, 1, 1, 0, 0, 0), Some(1704067200));
        assert_eq!(fmt_unix(1704067200), "2024-01-01 00:00");
        assert_eq!(exif_date("2023:06:15 10:20:30"), civil_to_unix(2023, 6, 15, 10, 20, 30));
        assert_eq!(filetime_to_unix(133_485_408_000_000_000), Some(1_704_067_200));
    }
    #[test]
    fn glob() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert!(glob_match(&c("*.jpg"), &c("img_1.jpg")));
        assert!(glob_match(&c("img_?.jpg"), &c("img_1.jpg")));
        assert!(!glob_match(&c("*.png"), &c("img_1.jpg")));
    }
}
