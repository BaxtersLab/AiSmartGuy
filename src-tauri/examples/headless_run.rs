//! A real AiSmartGuy run without the window: the operator-proof harness.
//!
//! It does what `cmd_begin_run` does and nothing else: the manifest comes from
//! the GUI's own builder (`ui::lanes::build_run_manifest`), the run from the
//! GUI's own `ui::commands::start_run`, with the cancel flag cleared first and
//! the orchestrator's progress events and model_loader's log lines printed
//! where the GUI would show them. Only the window is missing.
//!
//!   cargo run --release -p aismartguy-app --example headless_run -- \
//!       --pdf BOOK.pdf --lane MODEL_DIR [--fusion MODEL_DIR] [--mode 1|2] \
//!       [--ctx N] [--throttle N] --out DIR
//!
//! Structured findings (Phase 2) are the default exactly as in the app,
//! wherever the llama build supports grammars; `ASG_STRUCTURED_FINDINGS=0`
//! opts out.

use std::path::PathBuf;
use std::time::Instant;

const RAG_DEFAULTS: &[(&str, &str)] = &[
    ("001_fallacies.json",           include_str!("../../assets/rag_defaults/001_fallacies.json")),
    ("002_weaponized_language.json", include_str!("../../assets/rag_defaults/002_weaponized_language.json")),
    ("006_ambiguous_framing.json",   include_str!("../../assets/rag_defaults/006_ambiguous_framing.json")),
    ("007_racism_intolerance.json",  include_str!("../../assets/rag_defaults/007_racism_intolerance.json")),
    ("009_nlp_techniques.json",      include_str!("../../assets/rag_defaults/009_nlp_techniques.json")),
];

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let need = |n: &str| arg(&args, n).unwrap_or_else(|| panic!("missing {n} (see the file header)"));
    let pdf = PathBuf::from(need("--pdf"));
    let lane = need("--lane");
    let fusion = arg(&args, "--fusion").unwrap_or_default();
    let mode = arg(&args, "--mode").unwrap_or_else(|| if fusion.is_empty() { "1".into() } else { "2".into() });
    let ctx = arg(&args, "--ctx").map(|v| v.parse().expect("--ctx N"));
    let throttle = arg(&args, "--throttle").map(|v| v.parse().expect("--throttle N"));
    let out = PathBuf::from(need("--out"));

    // The GUI seeds these at start-up (seed_rag_defaults). Same bytes.
    let home = std::env::var("HOME").expect("HOME");
    let rag = PathBuf::from(&home).join(".aismartguy").join("rag_defaults");
    if !rag.is_dir() {
        std::fs::create_dir_all(&rag).expect("create rag_defaults");
        for (name, body) in RAG_DEFAULTS {
            std::fs::write(rag.join(name), body).expect("seed rag default");
        }
        eprintln!("[harness] seeded {} RAG default packets into {}", RAG_DEFAULTS.len(), rag.display());
    }

    eprintln!("[harness] llama: {:?}", model_loader::detect_llama());
    eprintln!("[harness] structured findings (ASG_STRUCTURED_FINDINGS): {:?}",
              std::env::var("ASG_STRUCTURED_FINDINGS").ok());

    let run_id = ui::lanes::new_run_id();
    let manifest = ui::lanes::build_run_manifest(
        &mode, [&lane, "", "", &fusion], ctx, throttle, &pdf,
        run_id.clone(), ui::lanes::run_timestamp(), env!("CARGO_PKG_VERSION"),
    ).unwrap_or_else(|e| panic!("manifest: {e}"));
    let run_dir = out.join(&run_id);
    std::fs::create_dir_all(&run_dir).expect("run dir");
    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap()).expect("manifest");
    eprintln!("[harness] mode {mode}; model1 = {}; fusion = {:?}",
              manifest.models.model1.as_ref().unwrap().path,
              manifest.models.fusion.as_ref().map(|m| m.path.clone()));

    let t0 = Instant::now();
    orchestrator::set_progress_callback(move |ev| {
        eprintln!("[{:>7.1}s] {:>5.1}% {} — {}", t0.elapsed().as_secs_f64(), ev.percent * 100.0, ev.stage, ev.message);
    });
    model_loader::set_log_callback(move |line| eprintln!("[{:>7.1}s]   {line}", t0.elapsed().as_secs_f64()));

    let state = ui::state::new_shared_state();
    {
        let mut s = state.lock().unwrap();
        s.pdf_path = Some(pdf.to_string_lossy().into_owned());
        s.pdf_loaded = true;
        s.manifest = Some(manifest);
        s.config_detected = true;
    }
    model_loader::cancel::clear();
    let result = ui::commands::start_run(state, manifest_path, run_dir.clone());
    let secs = t0.elapsed().as_secs_f64();
    match result {
        Ok(()) => {
            println!("RESULT ok after {secs:.1}s; run dir {}", run_dir.display());
        }
        Err(e) => {
            println!("RESULT failed after {secs:.1}s: {e:?}");
            std::process::exit(1);
        }
    }
}
