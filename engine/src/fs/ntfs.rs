//! NTFS: deleted MFT records, $UsnJrnl:$J deletion timestamps, $Bitmap-based scoring.

use super::{Bitmap, FsKind, Volume};
use crate::io::{Cached, Src};
use crate::model::*;
use crate::util::*;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
struct NRun {
    lcn: Option<u64>,
    clusters: u64,
}

#[derive(Default, Clone)]
struct DataAttr {
    resident: Option<Vec<u8>>,
    runs: Vec<(u64, Vec<NRun>)>, // (start_vcn, runs) fragments
    size: u64,
}

impl DataAttr {
    fn sorted_runs(&self) -> Vec<NRun> {
        let mut f = self.runs.clone();
        f.sort_by_key(|(v, _)| *v);
        f.into_iter().flat_map(|(_, r)| r).collect()
    }
}

#[derive(Default)]
struct Rec {
    in_use: bool,
    is_dir: bool,
    seq: u16,
    base: u64,
    name: Option<(u64, u16, u8, String)>, // parent rec, parent seq, namespace, name
    si: [Option<i64>; 2],                 // created, modified
    fnt: [Option<i64>; 2],
    data: Option<DataAttr>,
    named: Vec<(String, DataAttr)>,
}

/// Compact entry kept for every record (for path reconstruction).
#[derive(Clone)]
struct Entry {
    name: String,
    parent: u64,
    parent_seq: u16,
    seq: u16,
    in_use: bool,
}

pub struct Ntfs {
    src: Src,
    raw: Src,
    cluster: u64,
    rec_size: u64,
    mft_runs: Vec<NRun>,
    total_clusters: u64,
    bitmap: Option<Bitmap>,
}

fn decode_runs(d: &[u8]) -> Vec<NRun> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut lcn: i64 = 0;
    while i < d.len() {
        let h = d[i];
        if h == 0 {
            break;
        }
        let ls = (h & 0xF) as usize;
        let os = (h >> 4) as usize;
        i += 1;
        if ls == 0 || ls > 8 || os > 8 || i + ls + os > d.len() {
            break;
        }
        let mut len: u64 = 0;
        for k in 0..ls {
            len |= (d[i + k] as u64) << (8 * k);
        }
        i += ls;
        if os == 0 {
            out.push(NRun { lcn: None, clusters: len });
        } else {
            let mut delta: i64 = 0;
            for k in 0..os {
                delta |= (d[i + k] as i64) << (8 * k);
            }
            let shift = 64 - 8 * os as u32;
            delta = (delta << shift) >> shift; // sign-extend
            lcn += delta;
            if lcn < 0 {
                break;
            }
            out.push(NRun { lcn: Some(lcn as u64), clusters: len });
        }
        i += os;
    }
    out
}

fn fixup(buf: &mut [u8]) -> bool {
    let uo = le16(buf, 4) as usize;
    let uc = le16(buf, 6) as usize;
    if uc == 0 || uo + uc * 2 > buf.len() {
        return false;
    }
    let usn = [buf[uo], buf[uo + 1]];
    for i in 1..uc {
        let pos = i * 512 - 2;
        if pos + 2 > buf.len() {
            break;
        }
        if buf[pos..pos + 2] != usn {
            return false;
        }
        buf[pos] = buf[uo + 2 * i];
        buf[pos + 1] = buf[uo + 2 * i + 1];
    }
    true
}

fn parse_record(buf: &mut [u8]) -> Option<Rec> {
    if &buf[0..4] != b"FILE" || !fixup(buf) {
        return None;
    }
    let flags = le16(buf, 0x16);
    let mut r = Rec { in_use: flags & 1 != 0, is_dir: flags & 2 != 0, seq: le16(buf, 0x10), base: le64(buf, 0x20) & 0xFFFF_FFFF_FFFF, ..Default::default() };
    let mut off = le16(buf, 0x14) as usize;
    let used = (le32(buf, 0x18) as usize).min(buf.len());
    while off + 16 <= used {
        let atype = le32(buf, off);
        let alen = le32(buf, off + 4) as usize;
        if atype == 0xFFFF_FFFF || alen < 16 || off + alen > used {
            break;
        }
        let a = &buf[off..off + alen];
        let nonres = a[8] != 0;
        let nlen = a[9] as usize;
        let noff = le16(a, 10) as usize;
        let aname = if nlen > 0 { utf16le(a.get(noff..noff + nlen * 2).unwrap_or(&[])) } else { String::new() };
        let value = if !nonres {
            let vl = le32(a, 0x10) as usize;
            let vo = le16(a, 0x14) as usize;
            a.get(vo..vo + vl)
        } else {
            None
        };
        match atype {
            0x10 => {
                if let Some(v) = value {
                    r.si = [filetime_to_unix(le64(v, 0)), filetime_to_unix(le64(v, 8))];
                }
            }
            0x30 => {
                if let Some(v) = value {
                    let ln = byte(v, 0x40) as usize;
                    let ns = byte(v, 0x41);
                    let name = utf16le(v.get(0x42..0x42 + ln * 2).unwrap_or(&[]));
                    let pref = le64(v, 0);
                    // Prefer Win32 / POSIX names over 8.3 DOS names.
                    let better = match &r.name {
                        None => true,
                        Some((_, _, old_ns, _)) => *old_ns == 2 && ns != 2,
                    };
                    if better && !name.is_empty() {
                        r.name = Some((pref & 0xFFFF_FFFF_FFFF, (pref >> 48) as u16, ns, name));
                        r.fnt = [filetime_to_unix(le64(v, 8)), filetime_to_unix(le64(v, 16))];
                    }
                }
            }
            0x80 => {
                let mut da = DataAttr::default();
                if let Some(v) = value {
                    da.size = v.len() as u64;
                    da.resident = Some(v.to_vec());
                } else if nonres {
                    let start_vcn = le64(a, 0x10);
                    let ro = le16(a, 0x20) as usize;
                    let runs = decode_runs(a.get(ro..).unwrap_or(&[]));
                    da.size = if start_vcn == 0 { le64(a, 0x30) } else { 0 };
                    da.runs.push((start_vcn, runs));
                }
                if aname.is_empty() {
                    match &mut r.data {
                        Some(d) => merge(d, da),
                        None => r.data = Some(da),
                    }
                } else if let Some((_, d)) = r.named.iter_mut().find(|(n, _)| *n == aname) {
                    merge(d, da);
                } else {
                    r.named.push((aname, da));
                }
            }
            _ => {}
        }
        off += alen;
    }
    Some(r)
}

fn merge(d: &mut DataAttr, n: DataAttr) {
    if n.resident.is_some() {
        d.resident = n.resident;
    }
    if n.size > 0 {
        d.size = n.size;
    }
    d.runs.extend(n.runs);
}

impl Ntfs {
    pub fn open(raw: Src) -> anyhow::Result<Self> {
        let src = Cached::new(raw.clone());
        let b = src.read_vec(0, 512)?;
        let bps = le16(&b, 0x0B) as u64;
        let spc_raw = b[0x0D];
        let spc = if spc_raw > 0x80 { 1u64 << (256 - spc_raw as u32) } else { spc_raw as u64 };
        let cluster = bps * spc;
        if cluster == 0 || !cluster.is_power_of_two() {
            anyhow::bail!("NTFS: bad cluster size");
        }
        let total_clusters = le64(&b, 0x28) * bps / cluster;
        let mft_lcn = le64(&b, 0x30);
        let cpr = b[0x40] as i8;
        let rec_size = if cpr < 0 { 1u64 << (-cpr as u32) } else { cpr as u64 * cluster };
        if !(256..=65536).contains(&rec_size) {
            anyhow::bail!("NTFS: bad MFT record size");
        }
        let mut r0 = src.read_vec(mft_lcn * cluster, rec_size as usize)?;
        let rec0 = parse_record(&mut r0).ok_or_else(|| anyhow::anyhow!("NTFS: $MFT record unreadable"))?;
        let mft_runs = rec0.data.map(|d| d.sorted_runs()).unwrap_or_default();
        if mft_runs.is_empty() {
            anyhow::bail!("NTFS: $MFT has no data runs");
        }
        let mut v = Ntfs { src, raw, cluster, rec_size, mft_runs, total_clusters, bitmap: None };
        // $Bitmap = record 6
        if let Some(mut rec6) = v.read_record(6) {
            if let Some(d) = parse_record(&mut rec6).and_then(|r| r.data) {
                let bits = v.read_data(&d, (total_clusters / 8 + 1) as usize);
                v.bitmap = Some(Bitmap { bits, count: total_clusters, msb_first: false });
            }
        }
        Ok(v)
    }

    fn read_record(&self, idx: u64) -> Option<Vec<u8>> {
        let byte = idx * self.rec_size;
        let mut vcn_off = 0u64;
        for r in &self.mft_runs {
            let len = r.clusters * self.cluster;
            if byte < vcn_off + len {
                let lcn = r.lcn?;
                return self.src.read_vec(lcn * self.cluster + (byte - vcn_off), self.rec_size as usize).ok();
            }
            vcn_off += len;
        }
        None
    }

    fn read_data(&self, d: &DataAttr, max: usize) -> Vec<u8> {
        if let Some(r) = &d.resident {
            return r.clone();
        }
        let mut out = Vec::new();
        let want = (d.size as usize).min(max);
        for r in d.sorted_runs() {
            if out.len() >= want {
                break;
            }
            let n = ((r.clusters * self.cluster) as usize).min(want - out.len());
            match r.lcn {
                Some(l) => out.extend(self.raw.read_upto(l * self.cluster, n)),
                None => out.resize(out.len() + n, 0),
            }
        }
        out.truncate(want);
        out
    }

    fn byte_runs(&self, runs: &[NRun], size: u64) -> Vec<Run> {
        let mut out = Vec::new();
        let mut left = size;
        for r in runs {
            if left == 0 {
                break;
            }
            let n = (r.clusters * self.cluster).min(left);
            out.push(Run { off: r.lcn.map(|l| l * self.cluster), len: n });
            left -= n;
        }
        out
    }

    fn score(&self, runs: &[NRun], size: u64) -> Score {
        let Some(bm) = &self.bitmap else { return Score::Unknown };
        let need = size.div_ceil(self.cluster);
        let (mut free, mut total, mut seen) = (0u64, 0u64, 0u64);
        for r in runs {
            if seen >= need {
                break;
            }
            let n = r.clusters.min(need - seen);
            if let Some(l) = r.lcn {
                if l + n > self.total_clusters {
                    return Score::Overwritten;
                }
                free += bm.free_in(l, n);
                total += n;
            }
            seen += n;
        }
        if seen < need {
            return if free == 0 { Score::Overwritten } else { Score::Partial };
        }
        score_from(free, total)
    }

    /// Map MFT record number -> (seq, deletion time) from $UsnJrnl:$J.
    fn usn_deletions(&self, entries: &[Option<Entry>], records: &HashMap<u64, Rec>, ctx: &ScanCtx) -> HashMap<u64, (u16, i64)> {
        let mut out: HashMap<u64, (u16, i64)> = HashMap::new();
        // Find "$UsnJrnl" whose parent is $Extend (record 11).
        let idx = entries.iter().position(|e| matches!(e, Some(e) if e.name == "$UsnJrnl" && e.parent == 11 && e.in_use));
        let Some(idx) = idx else { return out };
        let j = match records.get(&(idx as u64)) {
            Some(r) => r.named.iter().find(|(n, _)| n == "$J").map(|(_, d)| d.clone()),
            None => self.read_record(idx as u64).and_then(|mut b| parse_record(&mut b)).and_then(|r| r.named.into_iter().find(|(n, _)| n == "$J").map(|(_, d)| d)),
        };
        let Some(j) = j else { return out };
        ctx.phase("NTFS: reading change journal", j.size);
        let mut vpos = 0u64;
        for r in j.sorted_runs() {
            let len = r.clusters * self.cluster;
            if let Some(lcn) = r.lcn {
                let mut p = 0u64;
                while p < len && !ctx.cancelled() {
                    let n = (len - p).min(8 << 20);
                    let buf = self.raw.read_upto(lcn * self.cluster + p, n as usize);
                    parse_usn(&buf, &mut out);
                    p += n;
                    ctx.set_done(vpos + p);
                }
            }
            vpos += len;
        }
        out
    }
}

fn parse_usn(buf: &[u8], out: &mut HashMap<u64, (u16, i64)>) {
    let mut p = 0usize;
    while p + 64 <= buf.len() {
        let len = le32(buf, p) as usize;
        let major = le16(buf, p + 4);
        if len < 60 || len > 4096 || len % 8 != 0 || p + len > buf.len() || !(2..=3).contains(&major) {
            p += 8;
            continue;
        }
        let r = &buf[p..p + len];
        let (fref, ts, reason) = if major == 2 {
            (le64(r, 8), le64(r, 32), le32(r, 40))
        } else {
            (le64(r, 8), le64(r, 48), le32(r, 56))
        };
        if reason & 0x200 != 0 {
            if let Some(t) = filetime_to_unix(ts) {
                let rec = fref & 0xFFFF_FFFF_FFFF;
                let seq = (fref >> 48) as u16;
                let e = out.entry(rec).or_insert((seq, t));
                if t >= e.1 {
                    *e = (seq, t);
                }
            }
        }
        p += len;
    }
}

impl Volume for Ntfs {
    fn kind(&self) -> FsKind {
        FsKind::Ntfs
    }

    fn scan_deleted(&self, ctx: &ScanCtx, label: &str) -> anyhow::Result<()> {
        let total_recs: u64 = self.mft_runs.iter().map(|r| r.clusters * self.cluster).sum::<u64>() / self.rec_size;
        ctx.phase("NTFS: reading Master File Table", total_recs);
        let mut entries: Vec<Option<Entry>> = vec![None; total_recs as usize];
        let mut deleted: HashMap<u64, Rec> = HashMap::new();
        let mut idx = 0u64;
        let chunk_recs = ((1 << 20) / self.rec_size).max(1);
        for r in &self.mft_runs {
            let recs_in_run = r.clusters * self.cluster / self.rec_size;
            let Some(lcn) = r.lcn else {
                idx += recs_in_run;
                continue;
            };
            let mut k = 0;
            while k < recs_in_run {
                if ctx.cancelled() {
                    return Ok(());
                }
                let n = chunk_recs.min(recs_in_run - k);
                let mut buf = self.raw.read_upto(lcn * self.cluster + k * self.rec_size, (n * self.rec_size) as usize);
                for (i, rb) in buf.chunks_exact_mut(self.rec_size as usize).enumerate() {
                    let rec_no = idx + k + i as u64;
                    if let Some(rec) = parse_record(rb) {
                        if rec.base != 0 {
                            continue;
                        }
                        if let Some((p, ps, _, name)) = &rec.name {
                            entries[rec_no as usize] = Some(Entry { name: name.clone(), parent: *p, parent_seq: *ps, seq: rec.seq, in_use: rec.in_use });
                        }
                        let keep = (!rec.in_use && !rec.is_dir && rec.data.is_some() && rec.name.is_some() && rec_no >= 24)
                            || (rec.in_use && rec.name.as_ref().is_some_and(|n| n.3 == "$UsnJrnl"));
                        if keep {
                            deleted.insert(rec_no, rec);
                        }
                    }
                }
                k += n;
                ctx.set_done(idx + k);
            }
            idx += recs_in_run;
        }

        let usn = self.usn_deletions(&entries, &deleted, ctx);
        ctx.phase("NTFS: rebuilding folders", deleted.len() as u64);
        let mut dir_cache: HashMap<u64, String> = HashMap::new();
        let mut done = 0;
        for (rec_no, rec) in deleted.iter() {
            done += 1;
            if rec.in_use {
                continue;
            }
            let (parent, pseq, _, name) = rec.name.clone().unwrap();
            let data = rec.data.as_ref().unwrap();
            let size = data.size;
            if size == 0 {
                continue;
            }
            let dir = path_of(parent, pseq, &entries, &mut dir_cache);
            let (loc, score) = if let Some(res) = &data.resident {
                (Location::Resident(Arc::new(res.clone())), Score::Good)
            } else {
                let runs = data.sorted_runs();
                (Location::Extents { src: self.raw.clone(), runs: self.byte_runs(&runs, size) }, self.score(&runs, size))
            };
            let mut it = FileItem::new(name, dir, size, Method::Metadata, loc);
            it.created = rec.si[0].or(rec.fnt[0]);
            it.modified = rec.si[1].or(rec.fnt[1]);
            if let Some((seq, t)) = usn.get(rec_no) {
                if *seq == rec.seq || seq.wrapping_add(1) == rec.seq {
                    it.deleted = Some(*t);
                }
            }
            if it.deleted.is_none() {
                it.note = "Deletion time not in the NTFS change journal".into();
            }
            it.score = score;
            it.fs = "NTFS".into();
            it.source = label.to_string();
            ctx.add(it);
            if done % 1000 == 0 {
                ctx.set_done(done);
            }
        }
        if usn.is_empty() {
            ctx.warn(format!("{label}: NTFS change journal not found or empty; deletion dates unavailable."));
        }
        Ok(())
    }

    fn free_ranges(&self) -> Option<Vec<(u64, u64)>> {
        self.bitmap.as_ref().map(|b| super::runs_to_bytes(b.free_runs(), self.cluster, 0))
    }
}

fn path_of(mut parent: u64, mut pseq: u16, entries: &[Option<Entry>], cache: &mut HashMap<u64, String>) -> String {
    let start = parent;
    if let Some(p) = cache.get(&start) {
        return p.clone();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut prefix = String::new();
    for _ in 0..64 {
        if parent == 5 {
            break;
        }
        match entries.get(parent as usize).and_then(|e| e.as_ref()) {
            Some(e) if e.seq == pseq || (!e.in_use && e.seq == pseq.wrapping_add(1)) => {
                parts.push(e.name.clone());
                pparent(&mut parent, &mut pseq, e);
            }
            _ => {
                prefix = "/$Orphan".into();
                break;
            }
        }
    }
    parts.reverse();
    let p = format!("{}/{}", prefix, parts.join("/"));
    let p = if p.len() > 1 && p.ends_with('/') { p[..p.len() - 1].to_string() } else { p };
    cache.insert(start, p.clone());
    p
}

fn pparent(parent: &mut u64, pseq: &mut u16, e: &Entry) {
    *parent = e.parent;
    *pseq = e.parent_seq;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runs() {
        // 0x21: 1-byte length, 2-byte offset. len=0x18, lcn=0x5634; then 0x11 len 4, delta -2 ; sparse 0x01 len 8
        let r = decode_runs(&[0x21, 0x18, 0x34, 0x56, 0x11, 0x04, 0xFE, 0x01, 0x08, 0x00]);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].lcn, Some(0x5634));
        assert_eq!(r[1].lcn, Some(0x5632));
        assert_eq!(r[2].lcn, None);
    }
}
