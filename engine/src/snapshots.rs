//! Point-in-time copies: Windows Volume Shadow Copies, macOS APFS local snapshots and Time Machine
//! backups. A file that exists in a snapshot but no longer exists on the live volume is reported as
//! deleted (some time after the snapshot was taken). These copies survive SSD TRIM.

use crate::model::*;
use crate::util::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

pub struct Snapshot {
    pub label: String,
    pub time: Option<i64>,
    /// Folder inside the snapshot that corresponds to `live`.
    pub root: PathBuf,
    pub live: PathBuf,
}

const NOISE: &[&str] = &["/Library/Caches/", "/AppData/Local/Temp/", "/.Trash/", "/$Recycle.Bin/", "/.Spotlight-V100/", "/.fseventsd/", "/Library/Logs/", "/.cache/"];

fn is_noise(rel: &str) -> bool {
    let r = format!("/{}/", rel.replace('\\', "/"));
    NOISE.iter().any(|n| r.contains(n))
}

/// Compare a snapshot tree with the live tree; report files that disappeared.
pub fn diff(snaps: &[Snapshot], ctx: &ScanCtx, label: &str, dir_prefix: &str) {
    // Newest snapshot first so the most recent copy of a file wins.
    let mut order: Vec<&Snapshot> = snaps.iter().collect();
    order.sort_by_key(|s| std::cmp::Reverse(s.time));
    let mut seen: HashMap<String, ()> = HashMap::new();
    for s in order {
        ctx.phase(format!("Comparing snapshot {} with the disk", s.label), 0);
        let mut n = 0u64;
        for e in WalkDir::new(&s.root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            if ctx.cancelled() {
                return;
            }
            if !e.file_type().is_file() {
                continue;
            }
            n += 1;
            if n % 2000 == 0 {
                ctx.set_done(n);
            }
            let Ok(rel) = e.path().strip_prefix(&s.root) else { continue };
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if is_noise(&rel_s) || seen.contains_key(&rel_s) {
                continue;
            }
            if fs::symlink_metadata(s.live.join(rel)).is_ok() {
                continue;
            }
            seen.insert(rel_s.clone(), ());
            let Ok(m) = e.metadata() else { continue };
            let (dir, name) = match rel_s.rsplit_once('/') {
                Some((d, n)) => (format!("{dir_prefix}/{d}"), n.to_string()),
                None => (if dir_prefix.is_empty() { "/".into() } else { dir_prefix.to_string() }, rel_s.clone()),
            };
            let mut it = FileItem::new(name, dir, m.len(), Method::Snapshot, Location::HostFile(e.path().to_path_buf()));
            let ts = |r: std::io::Result<std::time::SystemTime>| r.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
            it.created = ts(m.created());
            it.modified = ts(m.modified());
            it.deleted_after = s.time;
            it.score = Score::Good;
            it.fs = "Snapshot".into();
            it.source = label.into();
            it.note = format!("Copy found in {}; missing on the disk now", s.label);
            ctx.add(it);
        }
    }
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
pub fn find(mount: &str, ctx: &ScanCtx) -> Vec<Snapshot> {
    let mut out = Vec::new();
    let data = if mount == "/" { "/System/Volumes/Data" } else { mount };
    let live_users = if mount == "/" || mount == "/System/Volumes/Data" { Some(PathBuf::from("/Users")) } else { None };

    // 1. APFS local snapshots.
    let names: Vec<String> = Command::new("/usr/bin/tmutil")
        .args(["listlocalsnapshots", data])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| l.starts_with("com.apple")).collect())
        .unwrap_or_default();
    if !names.is_empty() {
        let base = std::env::temp_dir().join("restorix-snapshots");
        let _ = fs::create_dir_all(&base);
        let mut script = String::new();
        let mut planned = Vec::new();
        for n in &names {
            let mp = base.join(n);
            let _ = fs::create_dir_all(&mp);
            if fs::read_dir(&mp).map(|mut d| d.next().is_some()).unwrap_or(false) {
                planned.push((n.clone(), mp));
                continue; // already mounted earlier
            }
            script.push_str(&format!("/sbin/mount_apfs -o ro,nobrowse -s '{}' '{}' '{}' ; ", n, data, mp.display()));
            planned.push((n.clone(), mp));
        }
        if !script.is_empty() {
            let ok = if unsafe { libc::geteuid() } == 0 {
                Command::new("/bin/sh").arg("-c").arg(&script).status().map(|s| s.success()).unwrap_or(false)
            } else {
                let osa = format!("do shell script \"{}\" with administrator privileges with prompt \"Restorix needs your password to open APFS snapshots (read-only).\"", script.replace('\\', "\\\\").replace('"', "\\\""));
                Command::new("/usr/bin/osascript").arg("-e").arg(osa).status().map(|s| s.success()).unwrap_or(false)
            };
            if !ok {
                ctx.warn("Could not mount APFS snapshots (password not given or not permitted).");
            }
        }
        for (n, mp) in planned {
            let time = snap_time(&n);
            let (root, live) = match &live_users {
                Some(lu) => (mp.join("Users"), lu.clone()),
                None => (mp.clone(), PathBuf::from(mount)),
            };
            if root.exists() {
                out.push(Snapshot { label: format!("local snapshot {}", n.trim_start_matches("com.apple.TimeMachine.").trim_end_matches(".local")), time, root, live });
            }
        }
    }

    // 2. Time Machine backups (needs Full Disk Access).
    if let Some(lu) = &live_users {
        let backups: Vec<String> = Command::new("/usr/bin/tmutil")
            .arg("listbackups")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| l.starts_with('/')).collect())
            .unwrap_or_default();
        for b in backups.iter().rev().take(5) {
            let bp = Path::new(b);
            let Ok(rd) = fs::read_dir(bp) else { continue };
            for vol in rd.flatten() {
                let users = vol.path().join("Users");
                if users.is_dir() {
                    let name = bp.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    out.push(Snapshot { label: format!("Time Machine backup {}", name.trim_end_matches(".backup")), time: snap_time(&name), root: users, live: lu.clone() });
                }
            }
        }
    }
    out
}

/// "...2026-11-10-120000..." -> unix seconds
pub fn snap_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(16) {
        let w = &s[i..i + 17];
        let d = |a: usize, c: usize| w.get(a..c)?.parse::<i64>().ok();
        if w.as_bytes()[4] == b'-' && w.as_bytes()[7] == b'-' && w.as_bytes()[10] == b'-' {
            if let (Some(y), Some(mo), Some(da), Some(h), Some(mi), Some(se)) = (d(0, 4), d(5, 7), d(8, 10), d(11, 13), d(13, 15), d(15, 17)) {
                return civil_to_unix(y, mo, da, h, mi, se);
            }
        }
    }
    None
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
pub fn find(mount: &str, ctx: &ScanCtx) -> Vec<Snapshot> {
    use std::os::windows::process::CommandExt;
    let letter = mount.trim_end_matches('\\').trim_end_matches(':').to_ascii_uppercase();
    let ps = r#"$v=@{}; Get-CimInstance Win32_Volume | % { $v[$_.DeviceID]=$_.DriveLetter };
Get-CimInstance Win32_ShadowCopy | % { [pscustomobject]@{ dev=$_.DeviceObject; drive=$v[$_.VolumeName]; t=([DateTimeOffset]$_.InstallDate).ToUnixTimeSeconds() } } | ConvertTo-Json -Compress"#;
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
    let Ok(out) = out else {
        ctx.warn("Could not query Volume Shadow Copies.");
        return vec![];
    };
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap_or(serde_json::Value::Null);
    let arr = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![v],
        _ => vec![],
    };
    let mut res = Vec::new();
    for s in arr {
        let drive = s["drive"].as_str().unwrap_or("").trim_end_matches(':').to_ascii_uppercase();
        if drive != letter {
            continue;
        }
        let dev = s["dev"].as_str().unwrap_or("").to_string();
        let t = s["t"].as_i64();
        let base = PathBuf::from(format!("{dev}\\"));
        let live = PathBuf::from(format!("{letter}:\\"));
        let (root, live) = if base.join("Users").is_dir() { (base.join("Users"), live.join("Users")) } else { (base, live) };
        res.push(Snapshot { label: format!("shadow copy from {}", t.map(fmt_unix).unwrap_or_default()), time: t, root, live });
    }
    if res.is_empty() {
        ctx.warn(format!("No Volume Shadow Copies found for {letter}: (System Protection may be off)."));
    }
    res
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn find(_mount: &str, _ctx: &ScanCtx) -> Vec<Snapshot> {
    vec![]
}

pub fn scan(mount: &str, ctx: &ScanCtx, label: &str) {
    ctx.phase(format!("Looking for snapshots of {mount}"), 0);
    let snaps = find(mount, ctx);
    if snaps.is_empty() {
        return;
    }
    let prefix = if cfg!(windows) {
        let l = mount.trim_end_matches('\\');
        let users = snaps.iter().any(|s| s.live.ends_with("Users"));
        if users { format!("/{l}/Users") } else { format!("/{l}") }
    } else if snaps.iter().any(|s| s.live == Path::new("/Users")) {
        "/Users".to_string()
    } else {
        mount.trim_end_matches('/').to_string()
    };
    diff(&snaps, ctx, label, &prefix);
}

#[cfg(test)]
mod tests {
    #[test]
    fn time() {
        assert_eq!(super::snap_time("com.apple.TimeMachine.2024-01-01-000000.local"), Some(1_704_067_200));
    }
}
