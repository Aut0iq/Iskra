//! zeron-pet-prepare-assets — split each generated four-cell strip in
//! assets/raw into 96x128 RGBA frames, looping GIFs, manifest.json and a
//! contact sheet. Thin CLI over `zeron_pet::prep`.
//!
//! Exit code: 0 only when ALL eight animations succeeded. On any rejection
//! the report is still written, but manifest.json is never overwritten
//! with a partial one — a previously valid manifest stays in place.

use serde_json::json;
use std::path::PathBuf;
use zeron_pet::assets::ANIMATIONS;
use zeron_pet::prep;

fn main() {
    let mut raw = PathBuf::from("assets/raw");
    let mut out = PathBuf::from("assets");
    let mut report_path = PathBuf::from("artifacts/prepare-report.json");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--raw" => {
                i += 1;
                raw = PathBuf::from(args.get(i).expect("--raw DIR"));
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(args.get(i).expect("--out DIR"));
            }
            "--report" => {
                i += 1;
                report_path = PathBuf::from(args.get(i).expect("--report PATH"));
            }
            other => {
                eprintln!("usage: zeron-pet-prepare-assets [--raw DIR] [--out DIR] [--report PATH] (got '{other}')");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let (reports, done) = prep::run_all(&raw, &out);
    let all_ok = done.len() == ANIMATIONS.len();

    // Manifest is written atomically and only on complete success; a
    // partial or failed run never clobbers a previously valid manifest.
    let mut manifest_path = None;
    if all_ok {
        let manifest = prep::manifest_json(&done, &reports);
        match prep::write_manifest_atomic(&out, &manifest) {
            Ok(p) => manifest_path = Some(p),
            Err(e) => eprintln!("manifest write failed: {e}"),
        }
    }

    let sheet = prep::contact_sheet(&out, &done).ok();

    let report = json!({
        "tool": "zeron-pet-prepare-assets",
        "recipe": "alpha<16 => transparent black; union bounds; shared baseline/center; nearest; 96x128; char<=80x112",
        "raw": raw,
        "out": out,
        "contact_sheet": sheet,
        "animations": reports.iter().map(|r| json!({"name": r.name, "status": r.status, "detail": r.detail})).collect::<Vec<_>>(),
        "ok": done,
        "all_ok": all_ok,
        "manifest_written": manifest_path.is_some(),
    });
    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap())
        .expect("write report");

    for r in &reports {
        println!("{:<14} {}", r.name, r.status);
    }
    if let Some(p) = &manifest_path {
        println!("manifest: {}", p.display());
    } else {
        eprintln!(
            "manifest: NOT written ({} of {} animations ok)",
            done.len(),
            ANIMATIONS.len()
        );
    }
    println!("report:   {}", report_path.display());
    std::process::exit(if all_ok && manifest_path.is_some() {
        0
    } else {
        1
    });
}
