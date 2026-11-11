//! HFS+ / HFSX: deleted catalog records recovered from unused B-tree nodes and from old node
//! copies left in the journal. Allocation-file scoring.

use super::{Bitmap, FsKind, Volume};
use crate::io::{Cached, Src};
use crate::model::*;
use crate::util::*;
use std::collections::{HashMap, HashSet};

pub struct Hfs {
    src: Src,
    raw: Src,
    bs: u64,
    total_blocks: u64,
    catalog: Vec<(u64, u64)>, // (start block, block count)
    alloc: Option<Bitmap>,
    journal: Option<(u64, u64)>,
}

#[derive(Clone)]
struct FileRec {
    id: u32,
    parent: u32,
    name: String,
    created: Option<i64>,
    modified: Option<i64>,
    size: u64,
    extents: Vec<(u64, u64)>,
}

#[derive(Default)]
struct Found {
    files: Vec<FileRec>,
    folders: HashMap<u32, (u32, String)>,
}

fn fork(b: &[u8], o: usize) -> (u64, Vec<(u64, u64)>) {
    let size = be64(b, o);
    let ext = (0..8)
        .map(|i| (be32(b, o + 16 + i * 8) as u64, be32(b, o + 20 + i * 8) as u64))
        .filter(|&(_, c)| c > 0)
        .collect();
    (size, ext)
}

/// Parse a catalog leaf node; tolerant of garbage (returns what it can).
fn parse_leaf(node: &[u8], out: &mut Found) -> bool {
    let ns = node.len();
    if ns < 512 || node[8] != 0xFF || node[9] != 1 {
        return false;
    }
    let n = be16(node, 10) as usize;
    if n == 0 || n > ns / 8 {
        return false;
    }
    if be16(node, ns - 2) != 14 {
        return false;
    }
    let mut prev = 0usize;
    for i in 0..n {
        let off = be16(node, ns - 2 * (i + 1)) as usize;
        if off < 14 || off <= prev && i > 0 || off >= ns - 2 * (n + 1) {
            return i > 0;
        }
        prev = off;
        let klen = be16(node, off) as usize;
        if klen < 6 || off + 2 + klen > ns {
            continue;
        }
        let parent = be32(node, off + 2);
        let nlen = be16(node, off + 6) as usize;
        if 8 + nlen * 2 > klen + 2 {
            continue;
        }
        let name = utf16be(&node[off + 8..off + 8 + nlen * 2]);
        let mut d = off + 2 + klen;
        if d % 2 == 1 {
            d += 1;
        }
        let rec = match node.get(d..) {
            Some(r) => r,
            None => continue,
        };
        match be16(rec, 0) {
            1 if rec.len() >= 88 => {
                out.folders.insert(be32(rec, 8), (parent, name));
            }
            2 if rec.len() >= 248 => {
                let (size, extents) = fork(rec, 88);
                out.files.push(FileRec {
                    id: be32(rec, 8),
                    parent,
                    name,
                    created: hfs_to_unix(be32(rec, 12)),
                    modified: hfs_to_unix(be32(rec, 16)),
                    size,
                    extents,
                });
            }
            _ => {}
        }
    }
    true
}

/// Scan raw bytes (catalog node slack space, freed nodes, journal) for catalog records at any
/// even offset. Deleting a record compacts the node, but stale copies stay in the free area.
fn scan_raw(buf: &[u8], out: &mut Found) {
    let mut o = 0usize;
    while o + 16 < buf.len() {
        let klen = be16(buf, o) as usize;
        if (8..=516).contains(&klen) {
            let nlen = be16(buf, o + 6) as usize;
            if nlen > 0 && 6 + nlen * 2 == klen && o + 2 + klen + 248 <= buf.len() {
                let parent = be32(buf, o + 2);
                let units: Vec<u16> = (0..nlen).map(|i| be16(buf, o + 8 + i * 2)).collect();
                let d = o + 2 + klen;
                let rec = &buf[d..];
                let rt = be16(rec, 0);
                if parent >= 1 && !units.iter().any(|&u| u == 0 || u == 0xFFFF) {
                    let name = String::from_utf16_lossy(&units);
                    if rt == 1 && be32(rec, 8) >= 16 && hfs_to_unix(be32(rec, 12)).is_some() {
                        out.folders.entry(be32(rec, 8)).or_insert((parent, name));
                        o = d + 88;
                        continue;
                    }
                    if rt == 2 && be32(rec, 8) >= 16 && hfs_to_unix(be32(rec, 12)).is_some() {
                        let (size, extents) = fork(rec, 88);
                        out.files.push(FileRec {
                            id: be32(rec, 8),
                            parent,
                            name,
                            created: hfs_to_unix(be32(rec, 12)),
                            modified: hfs_to_unix(be32(rec, 16)),
                            size,
                            extents,
                        });
                        o = d + 248;
                        continue;
                    }
                }
            }
        }
        o += 2;
    }
}

impl Hfs {
    pub fn open(raw: Src) -> anyhow::Result<Self> {
        let src = Cached::new(raw.clone());
        let vh = src.read_vec(1024, 512)?;
        let bs = be32(&vh, 40) as u64;
        if bs < 512 || !bs.is_power_of_two() {
            anyhow::bail!("HFS+: bad block size");
        }
        let total_blocks = be32(&vh, 44) as u64;
        let (_, catalog) = fork(&vh, 272);
        let (asize, aext) = fork(&vh, 112);
        let mut bits = Vec::new();
        for (s, c) in &aext {
            bits.extend(src.read_upto(s * bs, (c * bs) as usize));
        }
        bits.truncate(asize as usize);
        let alloc = if bits.is_empty() { None } else { Some(Bitmap { bits, count: total_blocks, msb_first: true }) };
        let attrs = be32(&vh, 4);
        let mut journal = None;
        if attrs & 0x2000 != 0 {
            let jib = src.read_upto(be32(&vh, 12) as u64 * bs, 512);
            let (off, size) = (be64(&jib, 36), be64(&jib, 44));
            if size > 0 && off + size <= raw.len() {
                journal = Some((off, size));
            }
        }
        Ok(Hfs { src, raw, bs, total_blocks, catalog, alloc, journal })
    }

    fn cat_offset(&self, fork_off: u64) -> Option<u64> {
        let mut acc = 0u64;
        for &(s, c) in &self.catalog {
            let len = c * self.bs;
            if fork_off < acc + len {
                return Some(s * self.bs + fork_off - acc);
            }
            acc += len;
        }
        None
    }

    fn node(&self, i: u64, nsize: u64) -> Option<Vec<u8>> {
        let o = self.cat_offset(i * nsize)?;
        self.src.read_vec(o, nsize as usize).ok()
    }
}

impl Volume for Hfs {
    fn kind(&self) -> FsKind {
        FsKind::HfsPlus
    }

    fn scan_deleted(&self, ctx: &ScanCtx, label: &str) -> anyhow::Result<()> {
        let hdr = self.node(0, 512).ok_or_else(|| anyhow::anyhow!("HFS+: catalog unreadable"))?;
        let nsize = be16(&hdr, 32) as u64;
        let first_leaf = be32(&hdr, 24) as u64;
        let total_nodes = be32(&hdr, 36) as u64;
        if !(512..=32768).contains(&nsize) {
            anyhow::bail!("HFS+: bad node size");
        }

        // 1. Live leaf chain.
        ctx.phase("HFS+: reading catalog", total_nodes);
        let mut live = Found::default();
        let mut live_nodes = HashSet::new();
        let mut cur = first_leaf;
        while cur != 0 && live_nodes.insert(cur) && !ctx.cancelled() {
            let Some(n) = self.node(cur, nsize) else { break };
            parse_leaf(&n, &mut live);
            cur = be32(&n, 0) as u64;
        }
        let live_ids: HashSet<u32> = live.files.iter().map(|f| f.id).collect();

        // 2. Unused catalog nodes that still contain leaf records.
        let mut stale = Found::default();
        for i in 1..total_nodes {
            if ctx.cancelled() {
                return Ok(());
            }
            if let Some(n) = self.node(i, nsize) {
                if n[8] == 0xFF || n.iter().skip(14).any(|&b| b != 0) {
                    scan_raw(&n, &mut stale);
                }
            }
            if i % 256 == 0 {
                ctx.set_done(i);
            }
        }

        // 3. Old node copies in the journal.
        if let Some((joff, jsize)) = self.journal {
            ctx.phase("HFS+: reading journal", jsize);
            let mut p = 0u64;
            let chunk = 8u64 << 20;
            while p < jsize && !ctx.cancelled() {
                let buf = self.raw.read_upto(joff + p, (chunk + 1024) as usize);
                scan_raw(&buf, &mut stale);
                p += chunk;
                ctx.set_done(p.min(jsize));
            }
        }

        // 4. Deleted = file records whose CNID is no longer in the live catalog.
        let mut folders = live.folders;
        for (k, v) in stale.folders {
            folders.entry(k).or_insert(v);
        }
        let mut best: HashMap<u32, FileRec> = HashMap::new();
        for f in stale.files {
            if live_ids.contains(&f.id) || f.size == 0 || f.extents.is_empty() {
                continue;
            }
            let keep = match best.get(&f.id) {
                Some(b) => f.modified > b.modified,
                None => true,
            };
            if keep {
                best.insert(f.id, f);
            }
        }
        ctx.phase("HFS+: building results", best.len() as u64);
        for f in best.into_values() {
            let mut parts = Vec::new();
            let mut p = f.parent;
            for _ in 0..64 {
                if p <= 2 {
                    break;
                }
                match folders.get(&p) {
                    Some((pp, n)) => {
                        parts.push(n.clone());
                        p = *pp;
                    }
                    None => {
                        parts.push(format!("$Folder{p}"));
                        break;
                    }
                }
            }
            parts.reverse();
            let dir = format!("/{}", parts.join("/"));
            let mut runs = Vec::new();
            let (mut free, mut total) = (0u64, 0u64);
            let mut left = f.size;
            let mut bad = false;
            for &(s, c) in &f.extents {
                if left == 0 {
                    break;
                }
                if s + c > self.total_blocks {
                    bad = true;
                    break;
                }
                let n = (c * self.bs).min(left);
                runs.push(Run { off: Some(s * self.bs), len: n });
                let blocks = n.div_ceil(self.bs);
                if let Some(a) = &self.alloc {
                    free += a.free_in(s, blocks);
                }
                total += blocks;
                left -= n;
            }
            if bad {
                continue;
            }
            let mut it = FileItem::new(f.name, dir, f.size, Method::Metadata, Location::Extents { src: self.raw.clone(), runs });
            it.created = f.created;
            it.modified = f.modified;
            it.score = if left > 0 {
                Score::Partial
            } else if self.alloc.is_some() {
                score_from(free, total)
            } else {
                Score::Unknown
            };
            it.fs = "HFS+".into();
            it.source = label.into();
            it.note = "HFS+ keeps no deletion date".into();
            ctx.add(it);
        }
        Ok(())
    }

    fn free_ranges(&self) -> Option<Vec<(u64, u64)>> {
        self.alloc.as_ref().map(|b| super::runs_to_bytes(b.free_runs(), self.bs, 0))
    }
}
