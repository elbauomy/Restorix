//! Previews: images inline, RAW via embedded JPEG, HEIC via macOS `sips`, video via a temp file,
//! text, and a hex dump fallback (useful to spot overwritten / zeroed data).

use crate::model::*;
use crate::restore::{copy_item, read_head};
use crate::util::*;
use base64::Engine;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// "image" | "video" | "text" | "hex" | "none"
    pub kind: String,
    pub data_url: Option<String>,
    pub path: Option<String>,
    pub text: Option<String>,
    pub note: Option<String>,
    /// Fraction of the first 64 KiB that is zero bytes (a strong hint the data was wiped/trimmed).
    pub zero_ratio: f32,
}

fn data_url(mime: &str, b: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(b))
}

fn hex(b: &[u8]) -> String {
    let mut s = String::new();
    for (i, row) in b.chunks(16).take(64).enumerate() {
        s.push_str(&format!("{:08x}  ", i * 16));
        for (j, x) in row.iter().enumerate() {
            s.push_str(&format!("{x:02x}{}", if j == 7 { "  " } else { " " }));
        }
        s.push_str(&" ".repeat((16 - row.len()) * 3));
        s.push_str(" |");
        s.extend(row.iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '.' }));
        s.push_str("|\n");
    }
    s
}

pub fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join("restorix-preview");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn preview(it: &FileItem) -> Preview {
    let head = match read_head(it, 64 * 1024) {
        Ok(h) => h,
        Err(e) => return Preview { kind: "none".into(), note: Some(format!("Cannot read data: {e}")), ..Default::default() },
    };
    let zero_ratio = if head.is_empty() { 1.0 } else { head.iter().filter(|&&b| b == 0).count() as f32 / head.len() as f32 };
    let mut p = Preview { zero_ratio, ..Default::default() };
    if zero_ratio > 0.98 && it.size > 0 {
        p.note = Some("The start of this file is all zeros. It was probably wiped (SSD TRIM) or overwritten.".into());
    }
    let ext = it.ext.as_str();
    match ext {
        "jpg" | "jpeg" | "jpe" | "png" | "gif" | "bmp" | "webp" | "avif" if it.size <= 40 << 20 => {
            if let Ok(b) = read_head(it, 40 << 20) {
                p.kind = "image".into();
                p.data_url = Some(data_url(mime_of(ext), &b));
                return p;
            }
        }
        "cr2" | "nef" | "arw" | "dng" | "pef" | "tif" | "tiff" => {
            if let Location::Extents { src, runs } = &it.loc {
                if let (Some(off), 1) = (runs.first().and_then(|r| r.off), runs.len()) {
                    let mut prevs = crate::carve::formats::tiff_previews(src, off);
                    prevs.sort_by_key(|p| std::cmp::Reverse(p.1));
                    if let Some(&(o, l)) = prevs.first() {
                        if l < 40 << 20 {
                            let b = src.read_upto(o, l as usize);
                            p.kind = "image".into();
                            p.data_url = Some(data_url("image/jpeg", &b));
                            p.note = Some("Showing the preview embedded in the RAW file".into());
                            return p;
                        }
                    }
                }
            }
            if let Some(url) = convert_with_sips(it) {
                p.kind = "image".into();
                p.data_url = Some(url);
                return p;
            }
        }
        "heic" | "heif" => {
            if let Some(url) = convert_with_sips(it) {
                p.kind = "image".into();
                p.data_url = Some(url);
                return p;
            }
            p.note = Some("HEIC previews need macOS. Restore the file to open it.".into());
        }
        "mp4" | "mov" | "m4v" | "webm" => {
            if it.size <= 1 << 30 {
                let path = temp_dir().join(format!("{}.{}", it.id, ext));
                if path.exists() || std::fs::File::create(&path).and_then(|mut f| copy_item(it, &mut f, u64::MAX)).is_ok() {
                    p.kind = "video".into();
                    p.path = Some(path.display().to_string());
                    return p;
                }
            }
        }
        _ => {}
    }
    if is_text_ext(ext) || (!head.is_empty() && head.iter().take(4096).all(|&c| c == b'\n' || c == b'\r' || c == b'\t' || (0x20..0x7f).contains(&c) || c >= 0x80)) {
        p.kind = "text".into();
        p.text = Some(String::from_utf8_lossy(&head).to_string());
        return p;
    }
    p.kind = "hex".into();
    p.text = Some(hex(&head[..head.len().min(1024)]));
    p
}

#[cfg(target_os = "macos")]
fn convert_with_sips(it: &FileItem) -> Option<String> {
    if it.size > 200 << 20 {
        return None;
    }
    let dir = temp_dir();
    let inp = dir.join(format!("{}.{}", it.id, it.ext));
    let out = dir.join(format!("{}.preview.jpg", it.id));
    if !out.exists() {
        let mut f = std::fs::File::create(&inp).ok()?;
        copy_item(it, &mut f, u64::MAX).ok()?;
        let ok = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "jpeg", "-Z", "1600"])
            .arg(&inp)
            .arg("--out")
            .arg(&out)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&inp);
        if !ok {
            return None;
        }
    }
    std::fs::read(&out).ok().map(|b| data_url("image/jpeg", &b))
}

#[cfg(not(target_os = "macos"))]
fn convert_with_sips(_: &FileItem) -> Option<String> {
    None
}
