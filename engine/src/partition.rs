//! MBR / GPT partition discovery and file-system sniffing.

use crate::fs::{sniff, FsKind};
use crate::io::Src;
use crate::util::*;

#[derive(Clone, Debug)]
pub struct Part {
    pub index: usize,
    pub start: u64,
    pub len: u64,
    pub fs: Option<FsKind>,
}

/// Find volumes on a source. If the source itself holds a file system (a volume device or a
/// volume image) a single entry at offset 0 is returned.
pub fn volumes(src: &Src) -> Vec<Part> {
    if let Some(fs) = sniff(src, 0) {
        return vec![Part { index: 0, start: 0, len: src.len(), fs: Some(fs) }];
    }
    let mut parts = gpt(src).unwrap_or_default();
    if parts.is_empty() {
        parts = mbr(src);
    }
    for p in parts.iter_mut() {
        p.fs = sniff(&slice(src, p.start, p.len), 0);
    }
    parts
}

pub fn slice(src: &Src, start: u64, len: u64) -> Src {
    if start == 0 && len == src.len() {
        return src.clone();
    }
    std::sync::Arc::new(crate::io::Slice { inner: src.clone(), start, size: len.min(src.len().saturating_sub(start)) })
}

fn gpt(src: &Src) -> Option<Vec<Part>> {
    for ss in [512u64, 4096] {
        let h = src.read_upto(ss, 512);
        if h.get(0..8) != Some(b"EFI PART") {
            continue;
        }
        let entries_lba = le64(&h, 72);
        let count = le32(&h, 80).min(1024) as u64;
        let esize = le32(&h, 84) as u64;
        if esize < 128 || esize > 4096 {
            return None;
        }
        let table = src.read_upto(entries_lba * ss, (count * esize) as usize);
        let mut parts = Vec::new();
        for i in 0..count as usize {
            let e = match table.get(i * esize as usize..(i + 1) * esize as usize) {
                Some(e) => e,
                None => break,
            };
            if e[0..16].iter().all(|&b| b == 0) {
                continue;
            }
            let first = le64(e, 32);
            let last = le64(e, 40);
            if last < first {
                continue;
            }
            parts.push(Part { index: parts.len() + 1, start: first * ss, len: (last - first + 1) * ss, fs: None });
        }
        return Some(parts);
    }
    None
}

fn mbr(src: &Src) -> Vec<Part> {
    let s = src.read_upto(0, 512);
    let mut parts = Vec::new();
    if s.len() < 512 || s[510] != 0x55 || s[511] != 0xAA {
        return parts;
    }
    for i in 0..4 {
        let e = &s[446 + i * 16..446 + (i + 1) * 16];
        let ptype = e[4];
        let lba = le32(e, 8) as u64;
        let cnt = le32(e, 12) as u64;
        if ptype == 0 || cnt == 0 {
            continue;
        }
        if matches!(ptype, 0x05 | 0x0F | 0x85) {
            // Extended partition: follow the EBR chain.
            let ext_base = lba;
            let mut next = lba;
            for _ in 0..128 {
                let ebr = src.read_upto(next * 512, 512);
                if ebr.len() < 512 || ebr[510] != 0x55 || ebr[511] != 0xAA {
                    break;
                }
                let l0 = le32(&ebr, 446 + 8) as u64;
                let c0 = le32(&ebr, 446 + 12) as u64;
                if c0 > 0 && ebr[446 + 4] != 0 {
                    parts.push(Part { index: parts.len() + 1, start: (next + l0) * 512, len: c0 * 512, fs: None });
                }
                let l1 = le32(&ebr, 462 + 8) as u64;
                if l1 == 0 {
                    break;
                }
                next = ext_base + l1;
            }
        } else if ptype != 0xEE {
            parts.push(Part { index: parts.len() + 1, start: lba * 512, len: cnt * 512, fs: None });
        }
    }
    parts
}
