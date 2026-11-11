//! Enumerate disks and volumes, report privileges, and check that a restore destination is on a
//! different physical disk than the one being recovered.

use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Default)]
pub struct Device {
    /// Path passed to `open_source`.
    pub path: String,
    pub name: String,
    /// "disk" | "partition" | "volume" | "container"
    pub kind: String,
    pub size: u64,
    pub fs: String,
    pub mounts: Vec<String>,
    pub internal: bool,
    pub removable: bool,
    pub ssd: Option<bool>,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Privileges {
    pub platform: String,
    pub elevated: bool,
    pub hint: String,
}

// ======================================================================= macOS
#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use plist::Value;
    use std::process::Command;

    fn diskutil(args: &[&str]) -> Option<Value> {
        let out = Command::new("/usr/sbin/diskutil").args(args).output().ok()?;
        Value::from_reader_xml(&out.stdout[..]).ok()
    }

    fn s(d: &plist::Dictionary, k: &str) -> String {
        d.get(k).and_then(|v| v.as_string()).unwrap_or("").to_string()
    }
    fn n(d: &plist::Dictionary, k: &str) -> u64 {
        d.get(k).and_then(|v| v.as_unsigned_integer()).unwrap_or(0)
    }
    fn b(d: &plist::Dictionary, k: &str) -> Option<bool> {
        d.get(k).and_then(|v| v.as_boolean())
    }

    fn info(id: &str) -> plist::Dictionary {
        diskutil(&["info", "-plist", id]).and_then(|v| v.into_dictionary()).unwrap_or_default()
    }

    pub fn list() -> Vec<Device> {
        let mut out = Vec::new();
        let Some(root) = diskutil(&["list", "-plist"]).and_then(|v| v.into_dictionary()) else { return out };
        let Some(all) = root.get("AllDisksAndPartitions").and_then(|v| v.as_array()) else { return out };
        for disk in all.iter().filter_map(|d| d.as_dictionary()) {
            let id = s(disk, "DeviceIdentifier");
            let di = info(&id);
            let internal = b(&di, "Internal").unwrap_or(false);
            let ssd = b(&di, "SolidState");
            let removable = b(&di, "RemovableMediaOrExternalDevice").or(b(&di, "Removable")).unwrap_or(!internal);
            let media = s(&di, "MediaName");
            let apfs_vols = disk.get("APFSVolumes").and_then(|v| v.as_array());
            let is_container = apfs_vols.is_some() && disk.get("APFSPhysicalStores").is_some();
            if is_container {
                let vols: Vec<&plist::Dictionary> = apfs_vols.unwrap().iter().filter_map(|v| v.as_dictionary()).collect();
                let names: Vec<String> = vols.iter().map(|v| s(v, "VolumeName")).filter(|n| !n.is_empty()).collect();
                let mounts: Vec<String> = vols.iter().map(|v| s(v, "MountPoint")).filter(|m| !m.is_empty()).collect();
                let phys = disk
                    .get("APFSPhysicalStores")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|d| d.as_dictionary())
                    .map(|d| s(d, "DeviceIdentifier"));
                out.push(Device {
                    path: format!("/dev/{id}"),
                    name: if names.is_empty() { format!("APFS container {id}") } else { names.join(", ") },
                    kind: "container".into(),
                    size: n(disk, "Size"),
                    fs: "APFS".into(),
                    mounts,
                    internal,
                    removable,
                    ssd,
                    parent: phys,
                });
                continue;
            }
            out.push(Device {
                path: format!("/dev/{id}"),
                name: if media.is_empty() { id.clone() } else { media },
                kind: "disk".into(),
                size: n(disk, "Size"),
                fs: s(disk, "Content"),
                mounts: vec![],
                internal,
                removable,
                ssd,
                parent: None,
            });
            for p in disk.get("Partitions").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|p| p.as_dictionary()) {
                let pid = s(p, "DeviceIdentifier");
                let content = s(p, "Content");
                if content == "EFI" || content == "Apple_Boot" {
                    continue;
                }
                let mp = s(p, "MountPoint");
                let vname = s(p, "VolumeName");
                out.push(Device {
                    path: format!("/dev/{pid}"),
                    name: if vname.is_empty() { format!("{pid} ({content})") } else { vname },
                    kind: "partition".into(),
                    size: n(p, "Size"),
                    fs: content,
                    mounts: if mp.is_empty() { vec![] } else { vec![mp] },
                    internal,
                    removable,
                    ssd,
                    parent: Some(format!("/dev/{id}")),
                });
            }
        }
        out
    }

    /// Physical whole-disk identifiers backing a disk identifier (follows APFS containers).
    pub fn physical(id: &str, depth: u32) -> Vec<String> {
        if depth > 4 {
            return vec![];
        }
        let d = info(id);
        if let Some(stores) = d.get("APFSPhysicalStores").and_then(|v| v.as_array()) {
            let mut v = Vec::new();
            for st in stores.iter().filter_map(|x| x.as_dictionary()) {
                let sid = s(st, "APFSPhysicalStore");
                v.extend(physical(&sid, depth + 1));
            }
            if !v.is_empty() {
                return v;
            }
        }
        let parent = s(&d, "ParentWholeDisk");
        if !parent.is_empty() && parent != id {
            return physical(&parent, depth + 1);
        }
        vec![if parent.is_empty() { id.to_string() } else { parent }]
    }

    pub fn dest_disks(dest: &Path) -> Vec<String> {
        use std::ffi::CString;
        let Ok(c) = CString::new(dest.to_string_lossy().as_bytes()) else { return vec![] };
        let mut st: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
            return vec![];
        }
        let from = unsafe { std::ffi::CStr::from_ptr(st.f_mntfromname.as_ptr()) }.to_string_lossy().to_string();
        match from.strip_prefix("/dev/") {
            Some(id) => physical(id, 0),
            None => vec![],
        }
    }

    pub fn source_disks(src: &str) -> Vec<String> {
        let id = src.trim_start_matches("/dev/").trim_start_matches('r');
        physical(id, 0)
    }

    pub fn privileges() -> Privileges {
        let root = unsafe { libc::geteuid() } == 0;
        Privileges {
            platform: "macos".into(),
            elevated: root,
            hint: if root {
                "Running as root.".into()
            } else {
                "Raw disk scans will ask for your administrator password. For the Trash and Time Machine, give Restorix Full Disk Access in System Settings › Privacy & Security.".into()
            },
        }
    }
}

// ======================================================================= Windows
#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405C;
    const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    fn open(path: &str, read: bool) -> Option<std::fs::File> {
        std::fs::OpenOptions::new()
            .access_mode(if read { 0x8000_0000 } else { 0 })
            .share_mode(0x1 | 0x2)
            .open(path)
            .ok()
    }

    fn ioctl(f: &std::fs::File, code: u32, input: &[u8], out_len: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; out_len];
        let mut ret = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                f.as_raw_handle() as _,
                code,
                input.as_ptr() as _,
                input.len() as u32,
                out.as_mut_ptr() as _,
                out_len as u32,
                &mut ret,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return None;
        }
        out.truncate(ret as usize);
        Some(out)
    }

    /// STORAGE_PROPERTY_QUERY { PropertyId, QueryType=0, AdditionalParameters[1] }
    fn query_prop(f: &std::fs::File, id: u32) -> Option<Vec<u8>> {
        let mut q = [0u8; 12];
        q[0..4].copy_from_slice(&id.to_le_bytes());
        ioctl(f, IOCTL_STORAGE_QUERY_PROPERTY, &q, 1024)
    }

    fn le32(b: &[u8], o: usize) -> u32 {
        crate::util::le32(b, o)
    }

    fn ssd_and_bus(f: &std::fs::File) -> (Option<bool>, Option<bool>, String) {
        let ssd = query_prop(f, 7).filter(|b| b.len() >= 9).map(|b| b[8] == 0);
        let (mut removable, mut model) = (None, String::new());
        if let Some(d) = query_prop(f, 0) {
            removable = Some(d.get(10).copied().unwrap_or(0) != 0 || le32(&d, 28) == 7);
            let po = le32(&d, 16) as usize;
            if po > 0 && po < d.len() {
                model = String::from_utf8_lossy(&d[po..]).split('\0').next().unwrap_or("").trim().to_string();
            }
        }
        (ssd, removable, model)
    }

    pub fn list() -> Vec<Device> {
        let mut out = Vec::new();
        for i in 0..32 {
            let path = format!(r"\\.\PhysicalDrive{i}");
            let Some(f) = open(&path, false) else { continue };
            let size = open(&path, true).and_then(|rf| ioctl(&rf, IOCTL_DISK_GET_LENGTH_INFO, &[], 8)).map(|b| crate::util::le64(&b, 0)).unwrap_or(0);
            let (ssd, removable, model) = ssd_and_bus(&f);
            out.push(Device {
                path,
                name: if model.is_empty() { format!("Physical drive {i}") } else { format!("{model} (drive {i})") },
                kind: "disk".into(),
                size,
                fs: String::new(),
                mounts: vec![],
                internal: !removable.unwrap_or(false),
                removable: removable.unwrap_or(false),
                ssd,
                parent: None,
            });
        }
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root = format!("{letter}:\\");
            let w = wide(&root);
            let dtype = unsafe { GetDriveTypeW(w.as_ptr()) };
            if !(dtype == 2 || dtype == 3) {
                continue; // removable or fixed only
            }
            let mut label = [0u16; 261];
            let mut fsname = [0u16; 261];
            let ok = unsafe {
                GetVolumeInformationW(w.as_ptr(), label.as_mut_ptr(), 261, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fsname.as_mut_ptr(), 261)
            };
            let label = String::from_utf16_lossy(&label).trim_end_matches('\0').to_string();
            let fs = if ok != 0 { String::from_utf16_lossy(&fsname).trim_end_matches('\0').to_string() } else { String::new() };
            let mut total: u64 = 0;
            unsafe { GetDiskFreeSpaceExW(w.as_ptr(), std::ptr::null_mut(), &mut total, std::ptr::null_mut()) };
            let vpath = format!(r"\\.\{letter}:");
            let (ssd, _, _) = open(&vpath, false).map(|f| ssd_and_bus(&f)).unwrap_or((None, None, String::new()));
            let parent = disk_numbers(&vpath).first().map(|n| format!(r"\\.\PhysicalDrive{n}"));
            out.push(Device {
                path: vpath,
                name: if label.is_empty() { format!("Local Disk ({letter}:)") } else { format!("{label} ({letter}:)") },
                kind: "volume".into(),
                size: total,
                fs,
                mounts: vec![root],
                internal: dtype == 3,
                removable: dtype == 2,
                ssd,
                parent,
            });
        }
        out
    }

    pub fn disk_numbers(volume_path: &str) -> Vec<u32> {
        let Some(f) = open(volume_path, false) else { return vec![] };
        let Some(b) = ioctl(&f, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], 1024) else { return vec![] };
        let n = le32(&b, 0) as usize;
        (0..n.min(32)).map(|i| le32(&b, 8 + i * 24)).collect()
    }

    pub fn dest_disks(dest: &Path) -> Vec<String> {
        let s = dest.to_string_lossy();
        let letter = s.chars().next().unwrap_or('C');
        disk_numbers(&format!(r"\\.\{letter}:")).into_iter().map(|n| n.to_string()).collect()
    }

    pub fn source_disks(src: &str) -> Vec<String> {
        if let Some(n) = src.to_ascii_lowercase().strip_prefix(r"\\.\physicaldrive") {
            return vec![n.to_string()];
        }
        disk_numbers(src).into_iter().map(|n| n.to_string()).collect()
    }

    pub fn privileges() -> Privileges {
        let elevated = open(r"\\.\PhysicalDrive0", true).is_some();
        Privileges {
            platform: "windows".into(),
            elevated,
            hint: if elevated { "Running as Administrator.".into() } else { "Restart Restorix with \"Run as administrator\" to scan disks.".into() },
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;
    pub fn list() -> Vec<Device> {
        vec![]
    }
    pub fn dest_disks(_: &Path) -> Vec<String> {
        vec![]
    }
    pub fn source_disks(_: &str) -> Vec<String> {
        vec![]
    }
    pub fn privileges() -> Privileges {
        Privileges { platform: "other".into(), elevated: unsafe { libc::geteuid() } == 0, hint: String::new() }
    }
}

pub fn list() -> Vec<Device> {
    imp::list()
}

pub fn privileges() -> Privileges {
    imp::privileges()
}

/// True when restoring into `dest` would write to the disk being recovered.
pub fn same_disk(source: &str, dest: &Path) -> bool {
    if !crate::io::is_device_path(source) {
        // An image file: only refuse writing into the image itself.
        return false;
    }
    let a = imp::source_disks(source);
    let b = imp::dest_disks(dest);
    a.iter().any(|x| b.contains(x))
}
