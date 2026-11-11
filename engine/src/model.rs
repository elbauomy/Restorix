//! Core data model shared by all scanners.

use crate::io::Src;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Trash,
    Snapshot,
    Metadata,
    Carved,
}

/// How likely the file content is still intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Score {
    Good,
    Partial,
    Overwritten,
    Unknown,
}

/// A contiguous piece of file content on the source. `off == None` means a sparse hole (zeros).
#[derive(Clone, Debug)]
pub struct Run {
    pub off: Option<u64>,
    pub len: u64,
}

#[derive(Clone)]
pub enum Location {
    /// A regular file reachable through the OS (Recycle Bin, Trash, mounted snapshot).
    HostFile(PathBuf),
    /// Raw extents on a device or image.
    Extents { src: Src, runs: Vec<Run> },
    /// Small file whose data lives inside the metadata record (NTFS resident data).
    Resident(Arc<Vec<u8>>),
}

#[derive(Clone, Serialize)]
pub struct FileItem {
    pub id: u64,
    pub name: String,
    /// Original folder, '/'-separated, always starting with '/'.
    pub dir: String,
    pub ext: String,
    pub category: Category,
    pub size: u64,
    pub created: Option<i64>,
    pub modified: Option<i64>,
    pub deleted: Option<i64>,
    /// When the exact deletion time is unknown but a lower bound is (snapshots).
    pub deleted_after: Option<i64>,
    pub method: Method,
    pub fs: String,
    pub source: String,
    pub score: Score,
    pub note: String,
    #[serde(skip)]
    pub loc: Location,
}

impl FileItem {
    pub fn new(name: String, dir: String, size: u64, method: Method, loc: Location) -> Self {
        // "._name" files are macOS AppleDouble sidecars (resource forks), not real content.
        let ext = if name.starts_with("._") { String::new() } else { crate::util::ext_of(&name) };
        let category = crate::util::category_of(&ext);
        FileItem {
            id: 0,
            name,
            dir,
            ext,
            category,
            size,
            created: None,
            modified: None,
            deleted: None,
            deleted_after: None,
            method,
            fs: String::new(),
            source: String::new(),
            score: Score::Unknown,
            note: String::new(),
            loc,
        }
    }

    /// Absolute offset of the first data byte on the raw source (used to de-duplicate carving).
    pub fn first_offset(&self) -> Option<u64> {
        match &self.loc {
            Location::Extents { runs, .. } => runs.iter().find_map(|r| r.off),
            _ => None,
        }
    }

    pub fn full_path(&self) -> String {
        if self.dir.ends_with('/') {
            format!("{}{}", self.dir, self.name)
        } else {
            format!("{}/{}", self.dir, self.name)
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub running: bool,
    pub phase: String,
    pub done: u64,
    pub total: u64,
    pub found: u64,
    pub messages: Vec<String>,
    pub error: Option<String>,
    pub started: i64,
    pub finished: Option<i64>,
}

/// Shared state of one scan. Cheap to clone.
#[derive(Clone)]
pub struct ScanCtx {
    pub items: Arc<RwLock<Vec<FileItem>>>,
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    next_id: Arc<AtomicU64>,
}

impl Default for ScanCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanCtx {
    pub fn new() -> Self {
        ScanCtx {
            items: Arc::new(RwLock::new(Vec::new())),
            progress: Arc::new(Mutex::new(Progress::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn add(&self, mut item: FileItem) {
        item.id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut v = self.items.write().unwrap();
        v.push(item);
        self.progress.lock().unwrap().found = v.len() as u64;
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn phase(&self, phase: impl Into<String>, total: u64) {
        let mut p = self.progress.lock().unwrap();
        p.phase = phase.into();
        p.done = 0;
        p.total = total;
    }

    pub fn set_done(&self, done: u64) {
        self.progress.lock().unwrap().done = done;
    }

    pub fn warn(&self, msg: impl Into<String>) {
        let m = msg.into();
        let mut p = self.progress.lock().unwrap();
        if !p.messages.contains(&m) {
            p.messages.push(m);
        }
    }
}

/// Helper: compute a score from (bytes still free, total bytes).
pub fn score_from(free_units: u64, total_units: u64) -> Score {
    if total_units == 0 {
        Score::Unknown
    } else if free_units == total_units {
        Score::Good
    } else if free_units == 0 {
        Score::Overwritten
    } else {
        Score::Partial
    }
}
