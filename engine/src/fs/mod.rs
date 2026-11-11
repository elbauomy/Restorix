//! File-system parsers. Each one finds deleted entries from on-disk metadata and can report the
//! currently free space (used for scoring and for carving only unallocated areas).

pub mod apfs;
pub mod exfat;
pub mod fat;
pub mod hfsplus;
pub mod ntfs;

use crate::io::Src;
use crate::model::ScanCtx;
use crate::util::*;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum FsKind {
    Ntfs,
    Fat,
    ExFat,
    HfsPlus,
    Apfs,
}

impl FsKind {
    pub fn label(&self) -> &'static str {
        match self {
            FsKind::Ntfs => "NTFS",
            FsKind::Fat => "FAT",
            FsKind::ExFat => "exFAT",
            FsKind::HfsPlus => "HFS+",
            FsKind::Apfs => "APFS",
        }
    }
}

pub fn sniff(src: &Src, off: u64) -> Option<FsKind> {
    let b = src.read_upto(off, 4096);
    if b.len() < 1536 {
        return None;
    }
    if &b[3..11] == b"NTFS    " {
        return Some(FsKind::Ntfs);
    }
    if &b[3..11] == b"EXFAT   " {
        return Some(FsKind::ExFat);
    }
    if &b[32..36] == b"NXSB" {
        return Some(FsKind::Apfs);
    }
    if &b[1024..1026] == b"H+" || &b[1024..1026] == b"HX" {
        return Some(FsKind::HfsPlus);
    }
    if b[510] == 0x55 && b[511] == 0xAA {
        let bps = le16(&b, 11);
        let spc = b[13];
        let nfats = b[16];
        let fat16 = &b[54..57] == b"FAT";
        let fat32 = &b[82..87] == b"FAT32";
        if matches!(bps, 512 | 1024 | 2048 | 4096) && spc.is_power_of_two() && (1..=2).contains(&nfats) && (fat16 || fat32 || le16(&b, 14) > 0) {
            if fat16 || fat32 {
                return Some(FsKind::Fat);
            }
        }
    }
    None
}

pub trait Volume: Send + Sync {
    fn kind(&self) -> FsKind;
    /// Find deleted files and add them to `ctx`. `label` is the source description shown to the user.
    fn scan_deleted(&self, ctx: &ScanCtx, label: &str) -> anyhow::Result<()>;
    /// Currently unallocated byte ranges (relative to the volume), if known.
    fn free_ranges(&self) -> Option<Vec<(u64, u64)>>;
}

pub fn open_volume(kind: FsKind, src: Src) -> anyhow::Result<Box<dyn Volume>> {
    Ok(match kind {
        FsKind::Ntfs => Box::new(ntfs::Ntfs::open(src)?),
        FsKind::Fat => Box::new(fat::Fat::open(src)?),
        FsKind::ExFat => Box::new(exfat::ExFat::open(src)?),
        FsKind::HfsPlus => Box::new(hfsplus::Hfs::open(src)?),
        FsKind::Apfs => Box::new(apfs::Apfs::open(src)?),
    })
}

/// Allocation bitmap helper. `msb_first` selects HFS+ bit order; everything else is LSB-first.
pub struct Bitmap {
    pub bits: Vec<u8>,
    pub count: u64,
    pub msb_first: bool,
}

impl Bitmap {
    #[inline]
    pub fn used(&self, i: u64) -> bool {
        if i >= self.count {
            return true;
        }
        let b = self.bits.get((i / 8) as usize).copied().unwrap_or(0xFF);
        let bit = if self.msb_first { 0x80 >> (i % 8) } else { 1 << (i % 8) };
        b & bit != 0
    }

    /// Count free units in [start, start+n).
    pub fn free_in(&self, start: u64, n: u64) -> u64 {
        (start..start.saturating_add(n)).filter(|&i| !self.used(i)).count() as u64
    }

    /// Free runs as (first_unit, unit_count).
    pub fn free_runs(&self) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut i = 0u64;
        while i < self.count {
            // Skip fully-used bytes quickly.
            if i % 8 == 0 {
                let byte = self.bits.get((i / 8) as usize).copied().unwrap_or(0xFF);
                if byte == 0xFF {
                    i += 8;
                    continue;
                }
            }
            if self.used(i) {
                i += 1;
                continue;
            }
            let s = i;
            while i < self.count && !self.used(i) {
                if i % 8 == 0 && self.bits.get((i / 8) as usize) == Some(&0) && i + 8 <= self.count {
                    i += 8;
                } else {
                    i += 1;
                }
            }
            out.push((s, i - s));
        }
        out
    }
}

/// Convert unit runs to byte ranges.
pub fn runs_to_bytes(runs: Vec<(u64, u64)>, unit: u64, base: u64) -> Vec<(u64, u64)> {
    runs.into_iter().map(|(s, n)| (base + s * unit, n * unit)).collect()
}
