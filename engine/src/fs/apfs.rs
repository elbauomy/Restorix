//! APFS (best effort).
//!
//! APFS is copy-on-write: when a file is deleted the B-tree leaf that described it is rewritten
//! elsewhere and the old node is released, but its bytes usually stay on disk until reused.
//! We (1) walk each volume's *current* file-system tree to learn which inodes are alive and which
//! blocks live files occupy, then (2) scan the whole container for stale, checksum-valid FS-tree
//! leaf nodes and collect inode / directory-entry / file-extent records of inodes that no longer
//! exist. Software-encrypted volumes cannot be parsed this way.

use super::{FsKind, Volume};
use crate::io::{Cached, Src};
use crate::model::*;
use crate::util::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const OBJ_BTREE: u32 = 2;
const OBJ_BTREE_NODE: u32 = 3;
const OBJ_NXSB: u32 = 1;
const OBJ_FS: u32 = 0x0D;
const SUB_FSTREE: u32 = 0x0E;
const SUB_OMAP: u32 = 0x0B;

const T_INODE: u64 = 3;
const T_EXTENT: u64 = 8;
const T_DREC: u64 = 9;

pub fn fletcher_ok(b: &[u8]) -> bool {
    if b.len() < 16 {
        return false;
    }
    let (mut s1, mut s2) = (0u64, 0u64);
    for c in b[8..].chunks_exact(4) {
        s1 = (s1 + u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as u64) % 0xFFFF_FFFF;
        s2 = (s2 + s1) % 0xFFFF_FFFF;
    }
    let c1 = 0xFFFF_FFFF - ((s1 + s2) % 0xFFFF_FFFF);
    let c2 = 0xFFFF_FFFF - ((s1 + c1) % 0xFFFF_FFFF);
    (c2 << 32 | c1) == le64(b, 0)
}

struct Node<'a> {
    b: &'a [u8],
    leaf: bool,
    fixed: bool,
    n: usize,
    toc: usize,
    keys: usize,
    vend: usize,
}

impl<'a> Node<'a> {
    fn parse(b: &'a [u8]) -> Option<Node<'a>> {
        let flags = le16(b, 32);
        let n = le32(b, 36) as usize;
        let toff = le16(b, 40) as usize;
        let tlen = le16(b, 42) as usize;
        let root = flags & 1 != 0;
        let toc = 56 + toff;
        let keys = toc + tlen;
        let vend = if root { b.len().checked_sub(40)? } else { b.len() };
        if keys > b.len() || n > b.len() / 4 {
            return None;
        }
        Some(Node { b, leaf: flags & 2 != 0, fixed: flags & 4 != 0, n, toc, keys, vend })
    }

    /// (key, value) for entry i. For fixed-size nodes, key/value sizes are passed in.
    fn kv(&self, i: usize, fk: usize, fv: usize) -> Option<(&'a [u8], &'a [u8])> {
        let (ko, kl, vo, vl) = if self.fixed {
            let e = self.toc + i * 4;
            (le16(self.b, e) as usize, fk, le16(self.b, e + 2) as usize, if self.leaf { fv } else { 8 })
        } else {
            let e = self.toc + i * 8;
            (le16(self.b, e) as usize, le16(self.b, e + 2) as usize, le16(self.b, e + 4) as usize, le16(self.b, e + 6) as usize)
        };
        let k = self.b.get(self.keys + ko..self.keys + ko + kl)?;
        if vo == 0xFFFF {
            return Some((k, &[]));
        }
        let vs = self.vend.checked_sub(vo)?;
        let v = self.b.get(vs..vs + vl)?;
        Some((k, v))
    }
}

#[derive(Clone, Default)]
struct Inode {
    xid: u64,
    parent: u64,
    private_id: u64,
    created: Option<i64>,
    modified: Option<i64>,
    mode: u16,
    size: u64,
    name: Option<String>,
}

#[derive(Default)]
struct Records {
    inodes: HashMap<u64, Inode>,
    drecs: HashMap<u64, (u64, u64, String, u16)>, // file_id -> (xid, parent, name, dtype)
    extents: HashMap<u64, BTreeMap<u64, (u64, u64, u64)>>, // obj -> logical -> (xid, len, pblock)
}

fn parse_inode(v: &[u8], xid: u64) -> Inode {
    let mut ino = Inode {
        xid,
        parent: le64(v, 0),
        private_id: le64(v, 8),
        created: ns_to_unix(le64(v, 16)),
        modified: ns_to_unix(le64(v, 24)),
        mode: le16(v, 80),
        ..Default::default()
    };
    // Extended fields (xf_blob) start at 92.
    if v.len() > 96 {
        let n = le16(v, 92) as usize;
        let mut data = 96 + n * 4;
        for i in 0..n.min(32) {
            let h = 96 + i * 4;
            let t = byte(v, h);
            let sz = le16(v, h + 2) as usize;
            if let Some(d) = v.get(data..data + sz) {
                match t {
                    4 => ino.name = Some(String::from_utf8_lossy(d).trim_end_matches('\0').to_string()),
                    8 => ino.size = le64(d, 0),
                    _ => {}
                }
            }
            data += sz.div_ceil(8) * 8;
        }
    }
    ino
}

fn parse_drec_key(k: &[u8]) -> Option<String> {
    // Hashed key: hdr(8) name_len_and_hash(4) name ; unhashed: hdr(8) name_len(2) name
    let hl = (le32(k, 8) & 0x3FF) as usize;
    if 12 + hl == k.len() && hl > 0 {
        return Some(String::from_utf8_lossy(&k[12..12 + hl]).trim_end_matches('\0').to_string());
    }
    let ul = le16(k, 8) as usize;
    if 10 + ul == k.len() && ul > 0 {
        return Some(String::from_utf8_lossy(&k[10..10 + ul]).trim_end_matches('\0').to_string());
    }
    None
}

/// Collect FS-tree leaf records. `filter` decides whether an object id is worth keeping.
fn collect_leaf(b: &[u8], xid: u64, r: &mut Records, keep: &dyn Fn(u64, u64) -> bool) {
    let Some(node) = Node::parse(b) else { return };
    if !node.leaf || node.fixed {
        return;
    }
    for i in 0..node.n {
        let Some((k, v)) = node.kv(i, 0, 0) else { continue };
        if k.len() < 8 {
            continue;
        }
        let hdr = le64(k, 0);
        let oid = hdr & 0x0FFF_FFFF_FFFF_FFFF;
        let typ = hdr >> 60;
        match typ {
            T_INODE if v.len() >= 92 => {
                if !keep(T_INODE, oid) {
                    continue;
                }
                let ino = parse_inode(v, xid);
                match r.inodes.get(&oid) {
                    Some(old) if old.xid >= xid => {}
                    _ => {
                        r.inodes.insert(oid, ino);
                    }
                }
            }
            T_DREC if v.len() >= 18 => {
                let file_id = le64(v, 0);
                if !keep(T_DREC, file_id) {
                    continue;
                }
                if let Some(name) = parse_drec_key(k) {
                    let dt = le16(v, 16) & 0xF;
                    match r.drecs.get(&file_id) {
                        Some(old) if old.0 >= xid => {}
                        _ => {
                            r.drecs.insert(file_id, (xid, oid, name, dt));
                        }
                    }
                }
            }
            T_EXTENT if v.len() >= 24 && k.len() >= 16 => {
                if !keep(T_EXTENT, oid) {
                    continue;
                }
                let logical = le64(k, 8);
                let len = le64(v, 0) & 0x00FF_FFFF_FFFF_FFFF;
                let pb = le64(v, 8);
                let m = r.extents.entry(oid).or_default();
                match m.get(&logical) {
                    Some(old) if old.0 >= xid => {}
                    _ => {
                        m.insert(logical, (xid, len, pb));
                    }
                }
            }
            _ => {}
        }
    }
}

struct VolInfo {
    name: String,
    encrypted: bool,
    live_inodes: HashSet<u64>,
    live_private: HashSet<u64>,
    dirs: HashMap<u64, (u64, String)>, // dir inode -> (parent, name)
}

pub struct Apfs {
    src: Src,
    raw: Src,
    bs: u64,
    blocks: u64,
    xid: u64,
    omap_root: u64,
    fs_oids: Vec<u64>,
    used: std::sync::Mutex<Option<Vec<(u64, u64)>>>, // merged block ranges used by live files
}

impl Apfs {
    pub fn open(raw: Src) -> anyhow::Result<Self> {
        let src = Cached::new(raw.clone());
        let b0 = src.read_vec(0, 4096)?;
        let bs = le32(&b0, 36) as u64;
        if !(4096..=65536).contains(&bs) {
            anyhow::bail!("APFS: bad block size");
        }
        let mut best = src.read_vec(0, bs as usize)?;
        // Newest valid superblock in the checkpoint descriptor area.
        let desc_base = le64(&best, 112);
        let desc_blocks = (le32(&best, 104) & 0x7FFF_FFFF) as u64;
        if le32(&best, 104) & 0x8000_0000 == 0 {
            for i in 0..desc_blocks.min(4096) {
                if let Ok(b) = src.read_vec((desc_base + i) * bs, bs as usize) {
                    if le32(&b, 24) & 0xFFFF == OBJ_NXSB && &b[32..36] == b"NXSB" && fletcher_ok(&b) && le64(&b, 16) > le64(&best, 16) {
                        best = b;
                    }
                }
            }
        }
        let xid = le64(&best, 16);
        let blocks = le64(&best, 40);
        let omap = src.read_vec(le64(&best, 160) * bs, bs as usize)?;
        let omap_root = le64(&omap, 48);
        let maxfs = (le32(&best, 180) as usize).min(100);
        let fs_oids = (0..maxfs).map(|i| le64(&best, 184 + i * 8)).filter(|&o| o != 0).collect();
        Ok(Apfs { src, raw, bs, blocks, xid, omap_root, fs_oids, used: std::sync::Mutex::new(None) })
    }

    fn block(&self, paddr: u64) -> Option<Vec<u8>> {
        if paddr == 0 || paddr >= self.blocks {
            return None;
        }
        self.src.read_vec(paddr * self.bs, self.bs as usize).ok()
    }

    /// Load an object map (oid -> paddr at the newest xid <= self.xid).
    fn load_omap(&self, root: u64) -> HashMap<u64, u64> {
        let mut out: HashMap<u64, (u64, u64)> = HashMap::new();
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        while let Some(p) = stack.pop() {
            if !seen.insert(p) || seen.len() > 1_000_000 {
                continue;
            }
            let Some(b) = self.block(p) else { continue };
            if !fletcher_ok(&b) {
                continue;
            }
            let Some(n) = Node::parse(&b) else { continue };
            for i in 0..n.n {
                let Some((k, v)) = n.kv(i, 16, 16) else { continue };
                if n.leaf {
                    let (oid, x) = (le64(k, 0), le64(k, 8));
                    if x > self.xid || le32(v, 0) & 1 != 0 {
                        continue; // newer than checkpoint, or OMAP_VAL_DELETED
                    }
                    let pa = le64(v, 8);
                    match out.get(&oid) {
                        Some(&(ox, _)) if ox >= x => {}
                        _ => {
                            out.insert(oid, (x, pa));
                        }
                    }
                } else {
                    stack.push(le64(v, 0));
                }
            }
        }
        out.into_iter().map(|(k, (_, p))| (k, p)).collect()
    }

    fn walk_volume(&self, sb_oid: u64, cmap: &HashMap<u64, u64>, used: &mut Vec<(u64, u64)>, ctx: &ScanCtx) -> Option<VolInfo> {
        let sb = self.block(*cmap.get(&sb_oid)?)?;
        if &sb[32..36] != b"APSB" {
            return None;
        }
        let name = String::from_utf8_lossy(&sb[704..960]).trim_end_matches('\0').to_string();
        let encrypted = le64(&sb, 264) & 1 == 0;
        let vomap_blk = self.block(le64(&sb, 128))?;
        let vomap = self.load_omap(le64(&vomap_blk, 48));
        let root = *vomap.get(&le64(&sb, 136))?;
        let mut recs = Records::default();
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut visited = 0u64;
        while let Some(p) = stack.pop() {
            if !seen.insert(p) || ctx.cancelled() {
                continue;
            }
            let Some(b) = self.block(p) else { continue };
            if !fletcher_ok(&b) || le32(&b, 28) != SUB_FSTREE {
                if visited == 0 && encrypted {
                    return Some(VolInfo { name, encrypted, live_inodes: HashSet::new(), live_private: HashSet::new(), dirs: HashMap::new() });
                }
                continue;
            }
            visited += 1;
            let Some(n) = Node::parse(&b) else { continue };
            if n.leaf {
                collect_leaf(&b, le64(&b, 16), &mut recs, &|_, _| true);
                // Keep only extent block ranges for "used" set; drop the rest to save memory.
                for (_, m) in recs.extents.drain() {
                    for (_, (_, len, pb)) in m {
                        if pb != 0 {
                            used.push((pb, pb + len.div_ceil(self.bs)));
                        }
                    }
                }
            } else {
                for i in 0..n.n {
                    if let Some((_, v)) = n.kv(i, 0, 0) {
                        if let Some(&pa) = vomap.get(&le64(v, 0)) {
                            stack.push(pa);
                        }
                    }
                }
            }
        }
        let live_private = recs.inodes.values().map(|i| i.private_id).collect();
        let live_inodes: HashSet<u64> = recs.inodes.keys().copied().collect();
        let mut dirs = HashMap::new();
        for (fid, (_, parent, nm, dt)) in recs.drecs {
            if dt == 4 {
                dirs.insert(fid, (parent, nm));
            }
        }
        Some(VolInfo { name, encrypted, live_inodes, live_private, dirs })
    }

    fn used_frac(used: &[(u64, u64)], start: u64, n: u64) -> u64 {
        // Number of blocks in [start, start+n) that are used by live files.
        let end = start + n;
        let i = used.partition_point(|&(_, e)| e <= start);
        let mut count = 0;
        for &(s, e) in &used[i..] {
            if s >= end {
                break;
            }
            count += e.min(end) - s.max(start);
        }
        count
    }
}

fn merge_ranges(mut v: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    v.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(v.len());
    for (s, e) in v {
        match out.last_mut() {
            Some(l) if s <= l.1 => l.1 = l.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

impl Volume for Apfs {
    fn kind(&self) -> FsKind {
        FsKind::Apfs
    }

    fn scan_deleted(&self, ctx: &ScanCtx, label: &str) -> anyhow::Result<()> {
        ctx.phase("APFS: reading volume trees", self.fs_oids.len() as u64);
        let cmap = self.load_omap(self.omap_root);
        let mut used = Vec::new();
        let mut vols = Vec::new();
        for (i, &oid) in self.fs_oids.iter().enumerate() {
            if let Some(v) = self.walk_volume(oid, &cmap, &mut used, ctx) {
                if v.encrypted && v.live_inodes.is_empty() {
                    ctx.warn(format!("{label}: APFS volume \"{}\" is encrypted; deep metadata recovery is not possible. Use Trash and Snapshots.", v.name));
                }
                vols.push(v);
            }
            ctx.set_done(i as u64 + 1);
        }
        let used = merge_ranges(used);
        *self.used.lock().unwrap() = Some(used.clone());
        if vols.is_empty() {
            anyhow::bail!("APFS: no readable volumes");
        }

        let live_inodes: HashSet<u64> = vols.iter().flat_map(|v| v.live_inodes.iter().copied()).collect();
        let live_private: HashSet<u64> = vols.iter().flat_map(|v| v.live_private.iter().copied()).collect();
        let mut dirs: HashMap<u64, (u64, String)> = HashMap::new();
        for v in &vols {
            for (k, d) in &v.dirs {
                dirs.entry(*k).or_insert_with(|| d.clone());
            }
        }

        // Deep scan for stale FS-tree leaves.
        ctx.phase("APFS: deep scan for deleted records", self.blocks);
        let keep = |t: u64, id: u64| match t {
            T_INODE => !live_inodes.contains(&id),
            T_DREC => !live_inodes.contains(&id),
            T_EXTENT => !live_private.contains(&id),
            _ => false,
        };
        let mut recs = Records::default();
        let per = (4u64 << 20) / self.bs;
        let mut blk = 0u64;
        while blk < self.blocks {
            if ctx.cancelled() {
                return Ok(());
            }
            let n = per.min(self.blocks - blk);
            let buf = self.raw.read_upto(blk * self.bs, (n * self.bs) as usize);
            for b in buf.chunks_exact(self.bs as usize) {
                let t = le32(b, 24) & 0xFFFF;
                if (t == OBJ_BTREE || t == OBJ_BTREE_NODE) && le32(b, 28) == SUB_FSTREE && le16(b, 32) & 2 != 0 && fletcher_ok(b) {
                    collect_leaf(b, le64(b, 16), &mut recs, &keep);
                }
            }
            blk += n;
            ctx.set_done(blk);
        }

        // Deleted directories found in stale records extend the path map.
        for (fid, (_, parent, nm, dt)) in &recs.drecs {
            if *dt == 4 {
                dirs.entry(*fid).or_insert((*parent, nm.clone()));
            }
        }
        let vol_name = vols.first().map(|v| v.name.clone()).unwrap_or_default();
        for (id, ino) in &recs.inodes {
            if ino.mode & 0o170000 != 0o100000 {
                continue; // regular files only
            }
            let Some(ext) = recs.extents.get(&ino.private_id) else { continue };
            let size = if ino.size > 0 { ino.size } else { ext.values().map(|e| e.1).sum() };
            if size == 0 {
                continue;
            }
            let name = recs.drecs.get(id).map(|d| d.2.clone()).or_else(|| ino.name.clone()).unwrap_or_else(|| format!("inode_{id}"));
            let mut parts = Vec::new();
            let mut p = ino.parent;
            for _ in 0..64 {
                if p <= 2 {
                    break;
                }
                match dirs.get(&p) {
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
            let dir = if vols.len() > 1 { format!("/{}", parts.join("/")) } else { format!("/{}", parts.join("/")) };

            let mut runs = Vec::new();
            let mut pos = 0u64;
            let (mut usedb, mut total) = (0u64, 0u64);
            for (&logical, &(_, len, pb)) in ext {
                if logical > pos {
                    runs.push(Run { off: None, len: logical - pos });
                    pos = logical;
                }
                if logical < pos {
                    continue;
                }
                runs.push(Run { off: if pb == 0 { None } else { Some(pb * self.bs) }, len });
                if pb != 0 {
                    let nb = len.div_ceil(self.bs);
                    usedb += Apfs::used_frac(&used, pb, nb);
                    total += nb;
                }
                pos += len;
            }
            if pos < size {
                runs.push(Run { off: None, len: size - pos });
            }
            let mut it = FileItem::new(name, dir, size, Method::Metadata, Location::Extents { src: self.raw.clone(), runs });
            it.created = ino.created;
            it.modified = ino.modified;
            it.score = if pos < size { Score::Partial } else { score_from(total - usedb, total) };
            it.fs = "APFS".into();
            it.source = if vol_name.is_empty() { label.into() } else { format!("{label} ({vol_name})") };
            it.note = "APFS keeps no deletion date; recovered from a stale metadata block".into();
            ctx.add(it);
        }
        let _ = SUB_OMAP;
        let _ = OBJ_FS;
        Ok(())
    }

    fn free_ranges(&self) -> Option<Vec<(u64, u64)>> {
        // Complement of blocks used by live files (metadata blocks are included, which is harmless).
        let used = self.used.lock().unwrap().clone()?;
        let mut out = Vec::new();
        let mut cur = 0u64;
        for (s, e) in used {
            if s > cur {
                out.push((cur * self.bs, (s - cur) * self.bs));
            }
            cur = cur.max(e);
        }
        if cur < self.blocks {
            out.push((cur * self.bs, (self.blocks - cur) * self.bs));
        }
        Some(out)
    }
}
