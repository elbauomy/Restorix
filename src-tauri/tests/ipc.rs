//! Drives the real Tauri commands through the IPC layer (JSON in / JSON out), the same way the UI
//! does. Set RESTORIX_TEST_IMAGE to a disk image containing deleted files to run the scan part.

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
use tauri::webview::InvokeRequest;

fn call(w: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, body: Value) -> Value {
    let req = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
        body: InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    get_ipc_response(w, req).map(|r| r.deserialize::<Value>().unwrap()).unwrap_or_else(|e| panic!("{cmd} failed: {e:?}"))
}

#[test]
fn ipc_end_to_end() {
    let app = restorix_lib::setup(mock_builder()).build(tauri::generate_context!()).unwrap();
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    let priv_ = call(&w, "privileges", json!({}));
    assert!(priv_["platform"].is_string());
    let devs = call(&w, "list_devices", json!({}));
    assert!(devs.is_array());
    println!("devices: {}", devs.as_array().unwrap().len());

    let Ok(img) = std::env::var("RESTORIX_TEST_IMAGE") else { return };
    call(&w, "start_scan", json!({ "req": { "source": img, "label": "test", "mounts": [], "trash": false, "snapshots": false, "metadata": true, "carving": true, "carveFreeOnly": true, "carveTypes": [] } }));
    loop {
        let p = call(&w, "scan_progress", json!({}));
        if !p["running"].as_bool().unwrap() {
            assert!(p["error"].is_null(), "scan error: {}", p["error"]);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let filter = json!({ "text": "", "categories": ["image"], "exts": [], "methods": [], "scores": [], "deletedFrom": null, "deletedTo": null,
        "createdFrom": null, "createdTo": null, "includeUnknownDates": true, "sizeMin": null, "sizeMax": null, "folder": null });
    let page = call(&w, "query_items", json!({ "filter": filter, "sort": { "field": "name", "desc": false }, "offset": 0, "limit": 100 }));
    let items = page["items"].as_array().unwrap();
    println!("images: {} totalSize={}", page["total"], page["totalSize"]);
    assert!(!items.is_empty());
    let jpg = items.iter().find(|i| i["name"] == "beach photo.jpg").expect("beach photo.jpg");

    let tree = call(&w, "folder_tree", json!({ "filter": filter }));
    assert!(tree.as_array().unwrap().iter().any(|n| n["name"] == "Photos"));
    let stats = call(&w, "scan_stats", json!({}));
    assert!(stats["byCategory"]["image"].as_u64().unwrap() > 0);

    let pv = call(&w, "preview_item", json!({ "id": jpg["id"] }));
    assert_eq!(pv["kind"], "image");
    assert!(pv["dataUrl"].as_str().unwrap().starts_with("data:image/jpeg;base64,/9j/"));

    let dest = std::env::temp_dir().join(format!("restorix-ipc-test-{}", std::process::id()));
    let chk = call(&w, "check_destination", json!({ "dest": dest.display().to_string() }));
    assert_eq!(chk["sameDisk"], false);
    call(&w, "start_restore", json!({ "ids": null, "filter": filter, "dest": dest.display().to_string(), "keepStructure": true }));
    loop {
        let p = call(&w, "restore_progress", json!({}));
        if !p["running"].as_bool().unwrap() {
            println!("restored ok={} failed={} report={}", p["ok"], p["failed"], p["report"]);
            assert_eq!(p["failed"], 0);
            assert_eq!(p["ok"].as_u64().unwrap() as usize, page["total"].as_u64().unwrap() as usize);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let n = call(&w, "export_csv", json!({ "filter": filter, "path": dest.join("list.csv").display().to_string() }));
    assert_eq!(n, page["total"]);
    let _ = std::fs::remove_dir_all(dest);
}
