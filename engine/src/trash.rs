//! Windows Recycle Bin ($I/$R pairs) and macOS Trash folders.

use crate::model::*;
use crate::util::*;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn meta_times(m: &fs::Metadata) -> (Option<i64>, Option<i64>) {
    let t = |r: std::io::Result<std::time::SystemTime>| {
        r.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
    };
    (t(m.created()), t(m.modified()))
}

#[cfg(unix)]
fn ctime(m: &fs::Metadata) -> Option<i64> {
    use std::os::unix::fs::MetadataExt;
    Some(m.ctime())
}
#[cfg(not(unix))]
fn ctime(_: &fs::Metadata) -> Option<i64> {
    None
}

fn host_item(path: &Path, name: String, dir: String, method: Method) -> Option<FileItem> {
    let m = fs::symlink_metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let mut it = FileItem::new(name, dir, m.len(), method, Location::HostFile(path.to_path_buf()));
    let (c, md) = meta_times(&m);
    it.created = c;
    it.modified = md;
    it.score = Score::Good;
    Some(it)
}

/// Split a Windows path like `C:\Users\me\file.txt` into ("/C:/Users/me", "file.txt").
fn split_win(p: &str) -> (String, String) {
    let p = p.replace('\\', "/");
    match p.rsplit_once('/') {
        Some((d, n)) => (format!("/{}", d.trim_start_matches('/')), n.to_string()),
        None => ("/".into(), p),
    }
}

/// Parse a `$I` metadata file. Returns (original path, deletion time).
pub fn parse_dollar_i(b: &[u8]) -> Option<(String, Option<i64>)> {
    if b.len() < 24 {
        return None;
    }
    let ver = le64(b, 0);
    let del = filetime_to_unix(le64(b, 16));
    let path = match ver {
        1 => utf16le(b.get(24..24 + 520)?),
        2 => {
            let n = le32(b, 24) as usize;
            utf16le(b.get(28..28 + n * 2)?)
        }
        _ => return None,
    };
    Some((path, del))
}

pub fn scan_windows_recycle(mount: &str, ctx: &ScanCtx, label: &str) {
    let root = Path::new(mount).join("$Recycle.Bin");
    let Ok(sids) = fs::read_dir(&root) else {
        return;
    };
    for sid in sids.flatten() {
        let Ok(files) = fs::read_dir(sid.path()) else {
            ctx.warn(format!("Recycle Bin folder {} is not readable (run as Administrator).", sid.path().display()));
            continue;
        };
        for f in files.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            let Some(suffix) = fname.strip_prefix("$I") else { continue };
            let Ok(b) = fs::read(f.path()) else { continue };
            let Some((orig, del)) = parse_dollar_i(&b) else { continue };
            let rpath = sid.path().join(format!("$R{suffix}"));
            let (dir, name) = split_win(&orig);
            if rpath.is_dir() {
                for e in WalkDir::new(&rpath).into_iter().flatten().filter(|e| e.file_type().is_file()) {
                    let rel = e.path().strip_prefix(&rpath).unwrap_or(e.path());
                    let rel_dir = rel.parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                    let d = if rel_dir.is_empty() { format!("{dir}/{name}") } else { format!("{dir}/{name}/{rel_dir}") };
                    if let Some(mut it) = host_item(e.path(), e.file_name().to_string_lossy().to_string(), d, Method::Trash) {
                        it.deleted = del;
                        it.source = label.into();
                        it.fs = "Recycle Bin".into();
                        it.note = "In the Recycle Bin (deleted folder)".into();
                        ctx.add(it);
                    }
                }
            } else if let Some(mut it) = host_item(&rpath, name, dir, Method::Trash) {
                it.deleted = del;
                it.source = label.into();
                it.fs = "Recycle Bin".into();
                it.note = "In the Recycle Bin".into();
                ctx.add(it);
            }
        }
    }
}

/// macOS: ~/.Trash for the boot volume, `<mount>/.Trashes/<uid>` for other volumes.
pub fn scan_mac_trash(mount: &str, ctx: &ScanCtx, label: &str) {
    let mut roots: Vec<PathBuf> = Vec::new();
    if mount == "/" || mount == "/System/Volumes/Data" {
        if let Ok(home) = std::env::var("HOME") {
            roots.push(PathBuf::from(home).join(".Trash"));
        }
        if let Ok(users) = fs::read_dir("/Users") {
            for u in users.flatten() {
                let t = u.path().join(".Trash");
                if !roots.contains(&t) {
                    roots.push(t);
                }
            }
        }
    } else if let Ok(rd) = fs::read_dir(Path::new(mount).join(".Trashes")) {
        roots.extend(rd.flatten().map(|e| e.path()));
    }
    for root in roots {
        let Ok(top) = fs::read_dir(&root) else {
            if root.exists() {
                ctx.warn(format!("{} is not readable. Give Restorix Full Disk Access in System Settings › Privacy & Security.", root.display()));
            }
            continue;
        };
        for entry in top.flatten() {
            let top_name = entry.file_name().to_string_lossy().to_string();
            if top_name == ".DS_Store" {
                continue;
            }
            // Moving to the Trash updates the item's status-change time: our best deletion estimate.
            let del = fs::symlink_metadata(entry.path()).ok().and_then(|m| ctime(&m));
            for e in WalkDir::new(entry.path()).into_iter().flatten().filter(|e| e.file_type().is_file()) {
                let rel_dir = e
                    .path()
                    .parent()
                    .and_then(|p| p.strip_prefix(&root).ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let dir = if rel_dir.is_empty() { "/Trash".to_string() } else { format!("/Trash/{rel_dir}") };
                let n = e.file_name().to_string_lossy().to_string();
                if n == ".DS_Store" {
                    continue;
                }
                if let Some(mut it) = host_item(e.path(), n, dir, Method::Trash) {
                    it.deleted = del;
                    it.source = label.into();
                    it.fs = "Trash".into();
                    it.note = "In the Trash (deletion time = time it was moved to the Trash)".into();
                    ctx.add(it);
                }
            }
        }
    }
}

pub fn scan(mount: &str, ctx: &ScanCtx, label: &str) {
    ctx.phase(format!("Checking Trash / Recycle Bin on {mount}"), 0);
    if cfg!(windows) {
        scan_windows_recycle(mount, ctx, label);
    } else {
        scan_mac_trash(mount, ctx, label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dollar_i_v2() {
        let path = "C:\\Users\\me\\a.txt";
        let mut b = Vec::new();
        b.extend(2u64.to_le_bytes());
        b.extend(123u64.to_le_bytes());
        b.extend(133_485_408_000_000_000u64.to_le_bytes());
        let u: Vec<u16> = path.encode_utf16().chain([0]).collect();
        b.extend((u.len() as u32).to_le_bytes());
        for c in u {
            b.extend(c.to_le_bytes());
        }
        let (p, d) = parse_dollar_i(&b).unwrap();
        assert_eq!(p, path);
        assert_eq!(d, Some(1_704_067_200));
        assert_eq!(split_win(&p), ("/C:/Users/me".to_string(), "a.txt".to_string()));
    }
}
