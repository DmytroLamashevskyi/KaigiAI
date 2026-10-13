//! KaigiAI benchmark CLI (docs/PROJECT.md §10.19).
//!
//!   node scripts/fleurs-fetch.mjs                      # once: FLEURS sample
//!   cargo run --release --example bench -- [options]
//!
//! Options:
//!   --label NAME          report name (default "run")
//!   --langs ru,en,ja      source languages (default: all in the manifest)
//!   --targets ru,en       translation targets (default: all in the manifest)
//!   --limit N             utterances per source language (default 20)
//!   --mode seq|par|both   how target languages are translated (default both)
//!   --settings FILE.json  settings blob instead of the app's database
//!   --db PATH             app database (default %APPDATA%\ai.kaigi.desktop\kaigi.db)
//!   --set key=value       override one setting (repeatable), e.g.
//!                         --set localLlmPath=C:\models\qwen2.5-7b.gguf
//!   --data DIR            data dir (default ../bench/data)
//!
//! Writes bench/results/<unix-time>-<label>.md and .json and prints the table.

use std::path::PathBuf;

use app_lib::bench::{self, BenchOpts, TranslateMode};
use app_lib::sidecar::Sidecars;

fn main() {
    if let Err(e) = tauri::async_runtime::block_on(run()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    // Repo root (the crate lives in src-tauri/).
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut label = "run".to_string();
    let mut sources = None;
    let mut targets = None;
    let mut limit = 20;
    let mut mode = TranslateMode::Both;
    let mut settings_file: Option<PathBuf> = None;
    let mut db_path = bench::default_db_path();
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut data_dir = root.join("bench").join("data");

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or(format!("{a} needs a value"));
        let list = |s: String| s.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>();
        match a.as_str() {
            "--label" => label = val()?,
            "--langs" => sources = Some(list(val()?)),
            "--targets" => targets = Some(list(val()?)),
            "--limit" => limit = val()?.parse().map_err(|_| "--limit: not a number")?,
            "--mode" => {
                mode = match val()?.as_str() {
                    "seq" => TranslateMode::Sequential,
                    "par" => TranslateMode::Parallel,
                    "both" => TranslateMode::Both,
                    m => return Err(format!("--mode: unknown '{m}' (seq|par|both)")),
                }
            }
            "--settings" => settings_file = Some(val()?.into()),
            "--db" => db_path = Some(val()?.into()),
            "--set" => {
                let kv = val()?;
                let (k, v) = kv.split_once('=').ok_or("--set expects key=value")?;
                overrides.push((k.to_string(), v.to_string()));
            }
            "--data" => data_dir = val()?.into(),
            "-h" | "--help" => {
                println!("see the header of src-tauri/examples/bench.rs for options");
                return Ok(());
            }
            other => return Err(format!("unknown option {other}")),
        }
    }

    let mut settings = match settings_file {
        Some(f) => {
            let raw = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", f.display()))?
        }
        None => {
            let db = db_path.ok_or("no app database path; pass --db or --settings")?;
            bench::load_app_settings(&db).await?
        }
    };
    if let Some(o) = settings.as_object_mut() {
        for (k, v) in overrides {
            // Numbers / booleans stay typed (e.g. nGpuLayers=99).
            let value = serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v));
            o.insert(k, value);
        }
    }

    let sidecars = Sidecars::default();
    let result = async {
        let (stt, translator) = bench::build_providers(&mut settings, &sidecars)?;
        let opts = BenchOpts {
            data_dir,
            sources,
            targets,
            limit,
            mode,
            settings: settings.clone(),
            label: label.clone(),
        };
        bench::run(&opts, stt.as_ref(), translator.as_ref()).await
    }
    .await;
    sidecars.shutdown();
    let report = result?;

    let out = root.join("bench").join("results");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = out.join(format!("{stamp}-{label}"));
    std::fs::write(base.with_extension("md"), &report.markdown).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&report.items).map_err(|e| e.to_string())?;
    std::fs::write(base.with_extension("json"), json).map_err(|e| e.to_string())?;
    println!("{}", report.markdown);
    eprintln!("saved {}.md / .json", base.display());
    Ok(())
}
