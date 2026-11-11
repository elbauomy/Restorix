//! Tauri commands bridging the React UI and the recovery engine.

use restorix_engine::devices::{self, Device, Privileges};
use restorix_engine::model::{Method, Progress, ScanCtx};
use restorix_engine::preview::{self, Preview};
use restorix_engine::query::{self, Filter, FolderNode, Page, Sort, Stats};
use restorix_engine::restore::{self, RestoreProgress};
use restorix_engine::scan::{self, ScanRequest};
use restorix_engine::FileItem;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;

#[derive(Default)]
struct AppState {
    ctx: Mutex<Option<(ScanCtx, String)>>,
    restore: Arc<Mutex<RestoreProgress>>,
    restore_cancel: Arc<AtomicBool>,
}

impl AppState {
    fn ctx(&self) -> Option<ScanCtx> {
        self.ctx.lock().unwrap().as_ref().map(|(c, _)| c.clone())
    }
    fn items(&self) -> Vec<FileItem> {
        self.ctx().map(|c| c.items.read().unwrap().clone()).unwrap_or_default()
    }
}

type R<T> = Result<T, String>;

#[tauri::command]
async fn list_devices() -> Vec<Device> {
    tauri::async_runtime::spawn_blocking(devices::list).await.unwrap_or_default()
}

#[tauri::command]
fn privileges() -> Privileges {
    devices::privileges()
}

#[tauri::command]
fn start_scan(state: State<AppState>, req: ScanRequest) -> R<()> {
    let mut g = state.ctx.lock().unwrap();
    if let Some((c, _)) = g.as_ref() {
        if c.progress.lock().unwrap().running {
            return Err("A scan is already running.".into());
        }
    }
    let ctx = ScanCtx::new();
    ctx.progress.lock().unwrap().running = true;
    *g = Some((ctx.clone(), req.source.clone()));
    std::thread::spawn(move || scan::run(&req, &ctx));
    Ok(())
}

#[tauri::command]
fn scan_progress(state: State<AppState>) -> Option<Progress> {
    state.ctx().map(|c| c.progress.lock().unwrap().clone())
}

#[tauri::command]
fn cancel_scan(state: State<AppState>) {
    if let Some(c) = state.ctx() {
        c.cancel.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn query_items(state: State<AppState>, filter: Filter, sort: Sort, offset: usize, limit: usize) -> Page {
    let Some(c) = state.ctx() else {
        return query::page(&[], &filter, &sort, 0, 0);
    };
    let items = c.items.read().unwrap();
    query::page(&items, &filter, &sort, offset, limit.min(5000))
}

#[tauri::command]
fn folder_tree(state: State<AppState>, filter: Filter) -> Vec<FolderNode> {
    let Some(c) = state.ctx() else { return vec![] };
    let items = c.items.read().unwrap();
    query::tree(&items, &filter)
}

#[tauri::command]
fn scan_stats(state: State<AppState>) -> Stats {
    let Some(c) = state.ctx() else { return query::stats(&[]) };
    let items = c.items.read().unwrap();
    query::stats(&items)
}

#[tauri::command]
async fn preview_item(state: State<'_, AppState>, id: u64) -> R<Preview> {
    let it = state.items().into_iter().find(|i| i.id == id).ok_or("Item not found")?;
    tauri::async_runtime::spawn_blocking(move || preview::preview(&it)).await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DestCheck {
    same_disk: bool,
    exists: bool,
    message: Option<String>,
}

#[tauri::command]
fn check_destination(state: State<AppState>, dest: String) -> DestCheck {
    let source = state.ctx.lock().unwrap().as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
    let p = PathBuf::from(&dest);
    let same = !source.is_empty() && devices::same_disk(&source, &p);
    DestCheck {
        same_disk: same,
        exists: p.is_dir(),
        message: same.then(|| "This folder is on the disk being recovered. Writing there can overwrite the deleted files you are trying to save. Choose a folder on another drive (USB stick, external disk).".to_string()),
    }
}

#[tauri::command]
fn start_restore(state: State<AppState>, ids: Option<Vec<u64>>, filter: Option<Filter>, dest: String, keep_structure: bool) -> R<()> {
    if state.restore.lock().unwrap().running {
        return Err("A restore is already running.".into());
    }
    let all = state.items();
    let chosen: Vec<FileItem> = match (ids, filter) {
        (Some(ids), _) => {
            let set: std::collections::HashSet<u64> = ids.into_iter().collect();
            all.into_iter().filter(|i| set.contains(&i.id)).collect()
        }
        (None, Some(f)) => query::select(&all, &f).into_iter().cloned().collect(),
        _ => return Err("Nothing selected.".into()),
    };
    if chosen.is_empty() {
        return Err("Nothing selected.".into());
    }
    let source = state.ctx.lock().unwrap().as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
    let dest = PathBuf::from(dest);
    let raw = chosen.iter().any(|i| matches!(i.method, Method::Metadata | Method::Carved));
    if raw && devices::same_disk(&source, &dest) {
        return Err("The destination is on the disk being recovered. Choose a folder on a different drive so deleted data is not overwritten.".into());
    }
    state.restore_cancel.store(false, Ordering::Relaxed);
    {
        let mut p = state.restore.lock().unwrap();
        *p = RestoreProgress { running: true, total: chosen.len(), ..Default::default() };
    }
    let prog = state.restore.clone();
    let cancel = state.restore_cancel.clone();
    std::thread::spawn(move || restore::restore(chosen, &dest, keep_structure, prog, cancel));
    Ok(())
}

#[tauri::command]
fn restore_progress(state: State<AppState>) -> RestoreProgress {
    state.restore.lock().unwrap().clone()
}

#[tauri::command]
fn cancel_restore(state: State<AppState>) {
    state.restore_cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn export_csv(state: State<AppState>, filter: Filter, path: String) -> R<usize> {
    let all = state.items();
    let sel = query::select(&all, &filter);
    let d = |t: Option<i64>| t.map(restorix_engine::util::fmt_unix).unwrap_or_default();
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("name,folder,size,type,created,modified,deleted,deleted_after,method,recoverability,file_system,source,note\n");
    for i in &sel {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{:?},{:?},{},{},{}\n",
            q(&i.name),
            q(&i.dir),
            i.size,
            i.ext,
            d(i.created),
            d(i.modified),
            d(i.deleted),
            d(i.deleted_after),
            i.method,
            i.score,
            q(&i.fs),
            q(&i.source),
            q(&i.note)
        ));
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(sel.len())
}

/// Registers state and commands (shared by the app and the IPC integration test).
pub fn setup<R: tauri::Runtime>(b: tauri::Builder<R>) -> tauri::Builder<R> {
    b.manage(AppState::default()).invoke_handler(tauri::generate_handler![
            list_devices,
            privileges,
            start_scan,
            scan_progress,
            cancel_scan,
            query_items,
            folder_tree,
            scan_stats,
            preview_item,
            check_destination,
            start_restore,
            restore_progress,
            cancel_restore,
            export_csv
        ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    setup(tauri::Builder::default().plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_opener::init()))
        .run(tauri::generate_context!())
        .expect("error while running Restorix");
}
