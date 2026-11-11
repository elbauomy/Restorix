//! Command-line front end for the engine (also used for testing).
//!
//!   restorix-cli devices
//!   restorix-cli scan <device-or-image> [--no-meta] [--no-carve] [--all-space] [--json]
//!   restorix-cli restore <device-or-image> <dest-folder> [--no-carve] [--flat]

use restorix_engine::{devices, query, restore, scan, util::fmt_unix, ScanCtx};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.get(1).map(|s| s.as_str()) {
        Some("devices") => {
            println!("{}", serde_json::to_string_pretty(&devices::list()).unwrap());
            println!("{}", serde_json::to_string_pretty(&devices::privileges()).unwrap());
        }
        Some("scan") | Some("restore") if args.len() >= 3 => {
            let req = scan::ScanRequest {
                source: args[2].clone(),
                label: std::path::Path::new(&args[2]).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                trash: false,
                snapshots: false,
                metadata: !flag("--no-meta"),
                carving: !flag("--no-carve"),
                carve_free_only: !flag("--all-space"),
                ..Default::default()
            };
            let ctx = ScanCtx::new();
            let c2 = ctx.clone();
            let h = std::thread::spawn(move || scan::run(&req, &c2));
            let mut last = String::new();
            while !h.is_finished() {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let p = ctx.progress.lock().unwrap().clone();
                let line = format!("{} {}/{} found={}", p.phase, p.done, p.total, p.found);
                if line != last {
                    eprintln!("{line}");
                    last = line;
                }
            }
            h.join().unwrap();
            let p = ctx.progress.lock().unwrap().clone();
            for m in &p.messages {
                eprintln!("note: {m}");
            }
            if let Some(e) = &p.error {
                eprintln!("error: {e}");
            }
            let items = ctx.items.read().unwrap().clone();
            if args[1] == "scan" {
                if flag("--json") {
                    println!("{}", serde_json::to_string_pretty(&items).unwrap());
                } else {
                    for it in &items {
                        let d = |t: Option<i64>| t.map(fmt_unix).unwrap_or_else(|| "-".into());
                        println!(
                            "{:<8} {:<11} {:>10}  c={:<16} d={:<16} {}",
                            format!("{:?}", it.method),
                            format!("{:?}", it.score),
                            it.size,
                            d(it.created),
                            d(it.deleted),
                            it.full_path()
                        );
                    }
                    let st = query::stats(&items);
                    eprintln!("total: {} {:?} {:?}", st.total, st.by_method, st.by_score);
                }
            } else {
                let dest = std::path::PathBuf::from(args.get(3).expect("destination folder"));
                let prog = Arc::new(Mutex::new(restore::RestoreProgress::default()));
                restore::restore(items, &dest, !flag("--flat"), prog.clone(), Arc::new(AtomicBool::new(false)));
                let p = prog.lock().unwrap();
                eprintln!("restored {} files ({} bytes), {} failed. Report: {:?}", p.ok, p.bytes, p.failed, p.report);
                for e in &p.errors {
                    eprintln!("  {e}");
                }
            }
        }
        _ => {
            eprintln!("usage:\n  restorix-cli devices\n  restorix-cli scan <device|image> [--no-meta] [--no-carve] [--all-space] [--json]\n  restorix-cli restore <device|image> <dest> [--no-carve] [--flat]");
            std::process::exit(2);
        }
    }
}
