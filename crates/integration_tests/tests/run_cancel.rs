//! Terminate Run, through the GUI's own `ui::commands::start_run`. llama.cpp
//! is a TEST DOUBLE that answers at once or blocks until killed; the cancel
//! is requested from another thread, as the Terminate button does.
//!
//! Found in the VM harness (2026-09-27): terminating a run killed llama, but
//! the orchestrator took it for a failure -- it "retried in 3s", and the
//! window then said "Something went wrong ... All models failed ... check
//! that your models fit in available VRAM". A run cancelled during the fold
//! was worse: it came back as a finished report with no synthesis.
//!
//! One test in its own binary: it points HOME and PATH at a temp directory.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// FAKE_LLAMA_BLOCK=model1|fold: that stage's passes block until killed;
/// every other pass answers at once.
const FAKE_LLAMA: &str = r#"#!/bin/sh
here=$(dirname "$0"); prompt=; prev=
for a in "$@"; do [ "$prev" = -f ] && prompt=$a; prev=$a; done
which="$(basename "$(dirname "$prompt")")/$(basename "$prompt")"
echo "$which" >> "$here/calls.log"
case "$FAKE_LLAMA_BLOCK:$which" in
  model1:model1/*|fold:fusion/pass*) exec sleep 60 ;;
esac
echo "Answer for $which"
"#;

fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn calls(llama_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(llama_dir.join("calls.log")).unwrap_or_default()
        .lines().map(String::from).collect()
}

/// Start a mode-2 run; request a cancel once a llama call matching `when`
/// has started. Returns the outcome, the run dir and how long it took.
fn run_and_cancel(root: &Path, llama_dir: &Path, tag: &str, when: &'static str)
    -> (Result<(), String>, PathBuf, Duration)
{
    let pdf = root.join("book.pdf");
    let models = root.join("models").display().to_string();
    let run_id = ui::lanes::new_run_id();
    let manifest = ui::lanes::build_run_manifest(
        "2", [&models, "", "", &models], None, None, &pdf,
        run_id.clone(), ui::lanes::run_timestamp(), "test",
    ).unwrap();
    let run_dir = root.join(tag).join(&run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    let state = ui::state::new_shared_state();
    {
        let mut s = state.lock().unwrap();
        s.pdf_path = Some(pdf.to_string_lossy().into_owned());
        s.pdf_loaded = true;
        s.manifest = Some(manifest);
        s.config_detected = true;
    }
    let _ = std::fs::remove_file(llama_dir.join("calls.log"));
    model_loader::cancel::clear();

    let watch = llama_dir.to_path_buf();
    let canceller = std::thread::spawn(move || {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(30) {
            if calls(&watch).iter().any(|c| c.starts_with(when)) {
                std::thread::sleep(Duration::from_millis(300));   // let it block
                model_loader::cancel::request();
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    });
    let t0 = Instant::now();
    let r = ui::commands::start_run(state, manifest_path, run_dir.clone()).map_err(|e| format!("{e:?}"));
    let took = t0.elapsed();
    assert!(canceller.join().unwrap(), "the blocking pass ({when}) never started: {:?}", calls(llama_dir));
    (r, run_dir, took)
}

fn has_report(run_dir: &Path) -> bool {
    std::fs::read_dir(run_dir).unwrap().filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().ends_with("_AiSmartGuy_Report.pdf"))
}

#[test]
fn a_terminated_run_says_so_and_stops() {
    let root = std::env::temp_dir().join(format!("asg_run_cancel_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let llama_dir = home.join(".aismartguy/llama-cpp");
    let rag = home.join(".aismartguy/rag_defaults");
    for d in [&llama_dir, &rag, &root.join("models")] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_exe(&llama_dir.join("llama-completion"), FAKE_LLAMA);
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/rag_defaults");
    for e in std::fs::read_dir(&assets).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), rag.join(e.file_name())).unwrap();
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("PATH", "/usr/bin:/bin");
    // Prose mode, as this test was written for: structured findings are now
    // the default wherever llama supports grammars (manager ruling F1).
    std::env::set_var("ASG_STRUCTURED_FINDINGS", "0");
    std::fs::File::create(root.join("models/model.gguf")).unwrap().set_len(64 << 20).unwrap();
    let text = "Chapter One\n\nEvery reader should weigh the evidence and distrust a claim \
                that arrives without its reasons.\n".repeat(20);
    pdf_io::write_final_pdf(&root.join("unused.pdf"), &text, "{}", &root.join("book.pdf")).unwrap();

    // ── Terminated during a chapter pass. ──
    std::env::set_var("FAKE_LLAMA_BLOCK", "model1");
    let (r, run_dir, took) = run_and_cancel(&root, &llama_dir, "chapter", "model1/");
    assert_eq!(r, Err("Cancelled".to_string()), "a terminated run must say so, not report a failure");
    assert!(took < Duration::from_secs(20), "the blocked llama was not killed ({took:?})");
    assert_eq!(calls(&llama_dir), vec!["model1/chapter_001_r1.txt"], "nothing may start after a cancel");
    assert!(!has_report(&run_dir));

    // ── Terminated during the fold. ──
    std::env::set_var("FAKE_LLAMA_BLOCK", "fold");
    let (r, run_dir, _) = run_and_cancel(&root, &llama_dir, "fold", "fusion/pass");
    assert_eq!(r, Err("Cancelled".to_string()),
               "a run terminated in the fold came back as a finished report with no synthesis");
    assert!(!has_report(&run_dir));

    model_loader::cancel::clear();
    let _ = std::fs::remove_dir_all(&root);
}
