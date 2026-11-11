//! Signature-based carving for photos and videos.
//!
//! Every 512-byte boundary of the scanned ranges is checked for a known header; when one matches,
//! a format-aware parser walks the file structure to find its true length (JPEG markers, PNG chunks,
//! ISO-BMFF boxes, TIFF IFDs, RIFF sizes, GIF blocks). Carving assumes the file is stored
//! contiguously, which holds for most photos and many videos.

pub mod formats;

use crate::io::Src;
use crate::model::*;
use formats::{detect, ext_group, measure, Carved};
use std::collections::HashSet;

pub const ALL_TYPES: &[&str] = &["jpg", "png", "gif", "bmp", "webp", "tif", "raw", "heic", "mp4", "mov", "avi", "3gp"];

pub struct CarveOpts {
    pub types: HashSet<String>,
    pub min_size: u64,
}

impl Default for CarveOpts {
    fn default() -> Self {
        CarveOpts { types: ALL_TYPES.iter().map(|s| s.to_string()).collect(), min_size: 1024 }
    }
}

/// Carve `ranges` (byte ranges relative to `src`). `skip` holds start offsets already recovered by
/// metadata scanners so the same file is not listed twice.
pub fn carve(src: &Src, ranges: &[(u64, u64)], opts: &CarveOpts, skip: &HashSet<u64>, label: &str, fs: &str, ctx: &ScanCtx) {
    let total: u64 = ranges.iter().map(|r| r.1).sum();
    ctx.phase(format!("Carving photos & videos on {label}"), total);
    let chunk: u64 = 8 << 20;
    let mut done_before = 0u64;
    let mut count = 0u64;
    for &(rs, rl) in ranges {
        let re = rs + rl;
        let mut pos = rs.div_ceil(512) * 512;
        while pos < re {
            if ctx.cancelled() {
                return;
            }
            let n = chunk.min(re - pos);
            let buf = src.read_upto(pos, n as usize + 64);
            if buf.is_empty() {
                break;
            }
            let mut i = 0usize;
            let mut next = pos + n;
            while (i as u64) < n && i + 32 <= buf.len() {
                let at = pos + i as u64;
                if let Some(kind) = detect(&buf[i..]) {
                    if !skip.contains(&at) {
                        if let Some(c) = measure(src, at, kind) {
                            if c.len >= opts.min_size && opts.types.contains(ext_group(c.ext)) {
                                count += 1;
                                let len = c.len;
                                add(ctx, src, at, c, count, label, fs);
                                // Continue after the carved file (skips embedded thumbnails, etc.).
                                let aligned = (at + len).div_ceil(512) * 512;
                                if aligned >= pos + n {
                                    next = aligned;
                                    break;
                                }
                                i = (aligned - pos) as usize;
                                continue;
                            }
                        }
                    }
                }
                i += 512;
            }
            done_before += next.min(re) - pos;
            ctx.set_done(done_before.min(total));
            pos = next;
        }
    }
}

fn add(ctx: &ScanCtx, src: &Src, at: u64, c: Carved, n: u64, label: &str, fs: &str) {
    let name = format!("{}_{:06}_{:X}.{}", c.prefix, n, at, c.ext);
    let mut it = FileItem::new(name, String::new(), c.len, Method::Carved, Location::Extents { src: src.clone(), runs: vec![Run { off: Some(at), len: c.len }] });
    let cat = match it.category {
        Category::Video => "Videos",
        Category::Image => "Photos",
        _ => "Other",
    };
    it.dir = format!("/Carved/{cat}/{}", c.ext.to_uppercase());
    it.created = c.created;
    it.score = if c.complete { Score::Good } else { Score::Partial };
    it.fs = fs.into();
    it.source = label.into();
    it.note = if c.complete {
        "Found by signature; original name and folder are lost".into()
    } else {
        "Found by signature; end of file not found (may be fragmented)".into()
    };
    ctx.add(it);
}
