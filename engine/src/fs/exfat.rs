//! exFAT: deleted directory entry sets (in-use bit cleared), allocation bitmap scoring.

use super::{Bitmap, FsKind, Volume};
use crate::io::{Cached, Src};
use crate::model::*;
use crate::util::*;
use std::collections::HashSet;

pub struct ExFat {
    src: Src,
    raw: Src,
    cluster: u64,
    heap_off: u64,
    clusters: u64,
    root: u32,
    fat_off: u64,
    bitmap: Option<Bitmap>,
}

fn ts(t: u32, ms10: u8, tz: u8) -> Option<i64> {
    if t == 0 {
        return None;
    }
    let y = 1980 + (t >> 25) as i64;
    let mo = ((t >> 21) & 0xF) as i64;
    let d = ((t >> 16) & 0x1F) as i64;
    let h = ((t >> 11) & 0x1F) as i64;
    let mi = ((t >> 5) & 0x3F) as i64;
    let s = ((t & 0x1F) * 2) as i64 + ms10 as i64 / 100;
    let mut u = civil_to_unix(y, mo, d, h, mi, s)?;
    if tz & 0x80 != 0 {
        let off = ((tz << 1) as i8 >> 1) as i64 * 15 * 60;
        u -= off;
    }
    plausible(u)
}

impl ExFat {
    pub fn open(raw: Src) -> anyhow::Result<Self> {
        let src = Cached::new(raw.clone());
        let b = src.read_vec(0, 512)?;
        let bps = 1u64 << b[108].min(12);
        let spc = 1u64 << b[109].min(25);
        let mut v = ExFat {
            src,
            raw,
            cluster: bps * spc,
            heap_off: le32(&b, 88) as u64 * bps,
            clusters: le32(&b, 92) as u64,
            root: le32(&b, 96),
            fat_off: le32(&b, 80) as u64 * bps,
            bitmap: None,
        };
        // Locate the allocation bitmap (entry type 0x81) in the root directory.
        let root = v.read_chain(v.root, u64::MAX);
        for e in root.chunks_exact(32) {
            if e[0] == 0x81 {
                let first = le32(e, 20);
                let len = le64(e, 24);
                let bits = v.src.read_upto(v.coff(first), len as usize);
                v.bitmap = Some(Bitmap { bits, count: v.clusters, msb_first: false });
                break;
            }
            if e[0] == 0 {
                break;
            }
        }
        Ok(v)
    }

    fn coff(&self, c: u32) -> u64 {
        self.heap_off + (c as u64).saturating_sub(2) * self.cluster
    }
    fn valid(&self, c: u32) -> bool {
        c >= 2 && (c as u64) < self.clusters + 2
    }
    fn fat(&self, c: u32) -> u32 {
        let b = self.src.read_upto(self.fat_off + c as u64 * 4, 4);
        le32(&b, 0)
    }
    fn chain(&self, start: u32, max: u64) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = start;
        let mut seen = HashSet::new();
        while self.valid(c) && seen.insert(c) && (out.len() as u64) < max.min(1 << 22) {
            out.push(c);
            c = self.fat(c);
        }
        out
    }
    fn read_chain(&self, start: u32, max_bytes: u64) -> Vec<u8> {
        let mut v = Vec::new();
        for c in self.chain(start, max_bytes.div_ceil(self.cluster)) {
            v.extend(self.src.read_upto(self.coff(c), self.cluster as usize));
        }
        v
    }
    fn read_contig(&self, start: u32, len: u64) -> Vec<u8> {
        self.src.read_upto(self.coff(start), len.min(64 << 20) as usize)
    }
    fn is_free(&self, c: u32) -> bool {
        self.bitmap.as_ref().map(|b| !b.used(c as u64 - 2)).unwrap_or(false)
    }
}

struct Set {
    deleted: bool,
    is_dir: bool,
    name: String,
    first: u32,
    len: u64,
    contiguous: bool,
    created: Option<i64>,
    modified: Option<i64>,
}

fn parse_sets(data: &[u8]) -> Vec<Set> {
    let mut out = Vec::new();
    let n = data.len() / 32;
    let mut i = 0;
    while i < n {
        let e = &data[i * 32..i * 32 + 32];
        let t = e[0];
        if t == 0 {
            break;
        }
        if t == 0x85 || t == 0x05 {
            let deleted = t == 0x05;
            let sec = e[1] as usize;
            if (2..=18).contains(&sec) && i + sec < n {
                let s = &data[(i + 1) * 32..(i + 2) * 32];
                let want_stream = if deleted { 0x40 } else { 0xC0 };
                let want_name = if deleted { 0x41 } else { 0xC1 };
                if s[0] == want_stream {
                    let nlen = s[3] as usize;
                    let mut units = Vec::new();
                    for k in 2..=sec {
                        let ne = &data[(i + k) * 32..(i + k + 1) * 32];
                        if ne[0] != want_name {
                            break;
                        }
                        for j in 0..15 {
                            units.push(le16(ne, 2 + j * 2));
                        }
                    }
                    units.truncate(nlen);
                    if units.len() == nlen && nlen > 0 {
                        out.push(Set {
                            deleted,
                            is_dir: le16(e, 4) & 0x10 != 0,
                            name: String::from_utf16_lossy(&units),
                            first: le32(s, 20),
                            len: le64(s, 24),
                            contiguous: s[1] & 0x02 != 0,
                            created: ts(le32(e, 8), e[20], e[22]),
                            modified: ts(le32(e, 12), e[21], e[23]),
                        });
                    }
                    i += 1 + sec;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

impl Volume for ExFat {
    fn kind(&self) -> FsKind {
        FsKind::ExFat
    }

    fn scan_deleted(&self, ctx: &ScanCtx, label: &str) -> anyhow::Result<()> {
        ctx.phase("exFAT: reading directories", 0);
        let mut stack: Vec<(Vec<u8>, String)> = vec![(self.read_chain(self.root, u64::MAX), String::new())];
        let mut visited = HashSet::new();
        visited.insert(self.root);
        let mut dirs = 0;
        while let Some((data, path)) = stack.pop() {
            if ctx.cancelled() {
                break;
            }
            dirs += 1;
            ctx.set_done(dirs);
            for s in parse_sets(&data) {
                let child = format!("{path}/{}", s.name);
                if s.is_dir {
                    if self.valid(s.first) && visited.insert(s.first) {
                        let d = if s.contiguous || s.deleted { self.read_contig(s.first, s.len.max(self.cluster)) } else { self.read_chain(s.first, s.len.max(self.cluster)) };
                        stack.push((d, child));
                    }
                    continue;
                }
                if !s.deleted || s.len == 0 || !self.valid(s.first) {
                    continue;
                }
                let n = s.len.div_ceil(self.cluster);
                let mut clusters: Vec<u32> = Vec::new();
                if !s.contiguous {
                    let ch = self.chain(s.first, n);
                    if ch.len() as u64 == n {
                        clusters = ch;
                    }
                }
                if clusters.is_empty() {
                    if s.first as u64 + n > self.clusters + 2 {
                        continue;
                    }
                    clusters = (0..n as u32).map(|k| s.first + k).collect();
                }
                let free = clusters.iter().filter(|&&c| self.is_free(c)).count() as u64;
                // Coalesce clusters into byte runs.
                let mut runs: Vec<Run> = Vec::new();
                let mut left = s.len;
                for c in clusters {
                    let take = self.cluster.min(left);
                    let off = self.coff(c);
                    match runs.last_mut() {
                        Some(r) if r.off.map(|o| o + r.len) == Some(off) => r.len += take,
                        _ => runs.push(Run { off: Some(off), len: take }),
                    }
                    left -= take;
                }
                let mut it = FileItem::new(s.name, if path.is_empty() { "/".into() } else { path.clone() }, s.len, Method::Metadata, Location::Extents { src: self.raw.clone(), runs });
                it.created = s.created;
                it.modified = s.modified;
                it.score = if self.bitmap.is_some() { score_from(free, n) } else { Score::Unknown };
                it.fs = "exFAT".into();
                it.source = label.into();
                it.note = "exFAT keeps no deletion date".into();
                ctx.add(it);
            }
        }
        Ok(())
    }

    fn free_ranges(&self) -> Option<Vec<(u64, u64)>> {
        self.bitmap.as_ref().map(|b| super::runs_to_bytes(b.free_runs(), self.cluster, self.heap_off))
    }
}
