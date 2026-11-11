//! Reading recovered content and writing it to a destination folder.

use crate::model::*;
use crate::util::*;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Stream an item's content to `out`, at most `max` bytes.
pub fn copy_item(it: &FileItem, out: &mut dyn Write, max: u64) -> io::Result<u64> {
    let limit = it.size.min(max);
    let mut written = 0u64;
    match &it.loc {
        Location::HostFile(p) => {
            let mut f = File::open(p)?;
            let mut buf = vec![0u8; 1 << 20];
            while written < limit {
                let n = f.read(&mut buf[..((limit - written).min(1 << 20)) as usize])?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
                written += n as u64;
            }
        }
        Location::Resident(d) => {
            let n = (d.len() as u64).min(limit) as usize;
            out.write_all(&d[..n])?;
            written = n as u64;
        }
        Location::Extents { src, runs } => {
            let zeros = vec![0u8; 1 << 20];
            for r in runs {
                let mut left = r.len.min(limit - written);
                let mut pos = 0u64;
                while left > 0 {
                    let n = left.min(1 << 20) as usize;
                    match r.off {
                        Some(o) => {
                            let b = src.read_upto(o + pos, n);
                            if b.is_empty() {
                                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "data lies beyond the end of the source"));
                            }
                            out.write_all(&b)?;
                            pos += b.len() as u64;
                            left -= b.len() as u64;
                            written += b.len() as u64;
                            continue;
                        }
                        None => out.write_all(&zeros[..n])?,
                    }
                    pos += n as u64;
                    left -= n as u64;
                    written += n as u64;
                }
                if written >= limit {
                    break;
                }
            }
        }
    }
    Ok(written)
}

pub fn read_head(it: &FileItem, max: u64) -> io::Result<Vec<u8>> {
    let mut v = Vec::new();
    copy_item(it, &mut v, max)?;
    Ok(v)
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreProgress {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
    pub ok: usize,
    pub failed: usize,
    pub errors: Vec<String>,
    pub dest: String,
    pub report: Option<String>,
}

fn unique(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    for i in 1..100_000 {
        let c = p.with_file_name(format!("{stem} ({i}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    p
}

pub fn target_path(dest: &Path, it: &FileItem, keep_structure: bool) -> PathBuf {
    let mut p = dest.to_path_buf();
    if keep_structure {
        p.push(sanitize_component(&it.source));
        for c in it.dir.split('/').filter(|c| !c.is_empty()) {
            p.push(sanitize_component(c));
        }
    }
    p.push(sanitize_component(&it.name));
    p
}

fn csv(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn restore(items: Vec<FileItem>, dest: &Path, keep_structure: bool, prog: Arc<Mutex<RestoreProgress>>, cancel: Arc<AtomicBool>) {
    {
        let mut p = prog.lock().unwrap();
        *p = RestoreProgress { running: true, total: items.len(), dest: dest.display().to_string(), ..Default::default() };
    }
    let mut report = String::from("status,restored_to,original_folder,name,size,method,score,created,modified,deleted,source,error\n");
    for it in &items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let target = unique(target_path(dest, it, keep_structure));
        let res = (|| -> io::Result<u64> {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut f = io::BufWriter::with_capacity(1 << 20, File::create(&target)?);
            let n = copy_item(it, &mut f, u64::MAX)?;
            f.flush()?;
            drop(f);
            if let Some(m) = it.modified {
                let _ = filetime::set_file_mtime(&target, filetime::FileTime::from_unix_time(m, 0));
            }
            Ok(n)
        })();
        let d = |t: Option<i64>| t.map(fmt_unix).unwrap_or_default();
        let mut p = prog.lock().unwrap();
        p.done += 1;
        let (status, err) = match &res {
            Ok(n) => {
                p.ok += 1;
                p.bytes += n;
                ("restored", String::new())
            }
            Err(e) => {
                p.failed += 1;
                let m = format!("{}: {e}", it.full_path());
                if p.errors.len() < 200 {
                    p.errors.push(m.clone());
                }
                ("failed", e.to_string())
            }
        };
        report.push_str(&format!(
            "{status},{},{},{},{},{:?},{:?},{},{},{},{},{}\n",
            csv(&target.display().to_string()),
            csv(&it.dir),
            csv(&it.name),
            it.size,
            it.method,
            it.score,
            d(it.created),
            d(it.modified),
            d(it.deleted),
            csv(&it.source),
            csv(&err)
        ));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let rp = dest.join(format!("restorix-report-{now}.csv"));
    let ok = fs::create_dir_all(dest).and_then(|_| fs::write(&rp, report)).is_ok();
    let mut p = prog.lock().unwrap();
    p.running = false;
    if ok {
        p.report = Some(rp.display().to_string());
    }
}
