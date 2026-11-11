//! Scan orchestration: Trash -> Snapshots -> file-system metadata -> carving.

use crate::carve::{self, CarveOpts};
use crate::fs::{open_volume, FsKind};
use crate::io::open_source;
use crate::model::*;
use crate::partition;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanRequest {
    /// Device path or image file path.
    pub source: String,
    /// Display name.
    pub label: String,
    /// Mount points of the source, for Trash and Snapshot scanning.
    pub mounts: Vec<String>,
    pub trash: bool,
    pub snapshots: bool,
    pub metadata: bool,
    pub carving: bool,
    /// Carve only unallocated space (faster, avoids listing existing files).
    pub carve_free_only: bool,
    /// Carving type groups (see carve::ALL_TYPES). Empty = all.
    pub carve_types: Vec<String>,
}

impl Default for ScanRequest {
    fn default() -> Self {
        ScanRequest {
            source: String::new(),
            label: String::new(),
            mounts: vec![],
            trash: true,
            snapshots: true,
            metadata: true,
            carving: true,
            carve_free_only: true,
            carve_types: vec![],
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Run a full scan synchronously (call from a background thread). Progress and results go to `ctx`.
pub fn run(req: &ScanRequest, ctx: &ScanCtx) {
    {
        let mut p = ctx.progress.lock().unwrap();
        *p = Progress { running: true, started: now(), ..Default::default() };
    }
    if let Err(e) = run_inner(req, ctx) {
        ctx.progress.lock().unwrap().error = Some(format!("{e:#}"));
    }
    let mut p = ctx.progress.lock().unwrap();
    p.running = false;
    p.finished = Some(now());
    p.phase = if ctx.cancelled() { "Cancelled".into() } else { "Finished".into() };
}

fn run_inner(req: &ScanRequest, ctx: &ScanCtx) -> anyhow::Result<()> {
    let label = if req.label.is_empty() { req.source.clone() } else { req.label.clone() };

    if req.trash {
        for m in &req.mounts {
            crate::trash::scan(m, ctx, &label);
        }
    }
    if req.snapshots && !ctx.cancelled() {
        for m in &req.mounts {
            crate::snapshots::scan(m, ctx, &label);
        }
    }
    if !(req.metadata || req.carving) || ctx.cancelled() {
        return Ok(());
    }

    ctx.phase(format!("Opening {label}"), 0);
    let src = open_source(&req.source)?;
    let vols = partition::volumes(&src);
    let mut skip: HashSet<u64> = HashSet::new();
    let mut carve_ranges: Vec<(String, String, crate::io::Src, Vec<(u64, u64)>)> = Vec::new();

    if vols.is_empty() {
        ctx.warn(format!("{label}: no partitions or known file system found; carving the whole device."));
        carve_ranges.push((label.clone(), "Unknown".into(), src.clone(), vec![(0, src.len())]));
    }

    for p in &vols {
        if ctx.cancelled() {
            break;
        }
        let vsrc = partition::slice(&src, p.start, p.len);
        let vlabel = if vols.len() > 1 { format!("{label} · part {}", p.index) } else { label.clone() };
        let Some(kind) = p.fs else {
            if req.carving {
                carve_ranges.push((vlabel, "Unknown".into(), vsrc.clone(), vec![(0, vsrc.len())]));
            }
            continue;
        };
        let fsname = kind.label().to_string();
        let vol = match open_volume(kind, vsrc.clone()) {
            Ok(v) => Some(v),
            Err(e) => {
                ctx.warn(format!("{vlabel}: cannot read {fsname} metadata ({e}); falling back to carving."));
                None
            }
        };
        if let Some(v) = &vol {
            // APFS computes its free map during the metadata pass, so run it when carving free space too.
            let need_meta = req.metadata || (req.carving && req.carve_free_only && kind == FsKind::Apfs);
            if need_meta {
                let before = ctx.items.read().unwrap().len();
                if let Err(e) = v.scan_deleted(ctx, &vlabel) {
                    ctx.warn(format!("{vlabel}: {e}"));
                }
                let items = ctx.items.read().unwrap();
                for it in &items[before..] {
                    if let Some(o) = it.first_offset() {
                        skip.insert(o);
                    }
                }
                drop(items);
                if !req.metadata {
                    // Metadata was only needed for the free map: drop the results.
                    ctx.items.write().unwrap().truncate(before);
                }
            }
        }
        if req.carving {
            let ranges = if req.carve_free_only { vol.as_ref().and_then(|v| v.free_ranges()) } else { None };
            let ranges = ranges.unwrap_or_else(|| vec![(0, vsrc.len())]);
            carve_ranges.push((vlabel, fsname, vsrc, ranges));
        }
    }

    if req.carving {
        let mut opts = CarveOpts::default();
        if !req.carve_types.is_empty() {
            opts.types = req.carve_types.iter().cloned().collect();
        }
        for (vlabel, fsname, vsrc, ranges) in carve_ranges {
            if ctx.cancelled() {
                break;
            }
            carve::carve(&vsrc, &ranges, &opts, &skip, &vlabel, &fsname, ctx);
        }
    }
    Ok(())
}
