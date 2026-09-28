//! A whole run where structured findings cannot run: the llama build does not
//! support grammars, so the run falls back to prose (manager ruling F1). The
//! report must say, first, that prose quotes are not checked. llama.cpp is
//! replaced by a TEST DOUBLE whose `--help` names no `--grammar-file`.
//! Everything else is the product's.
//!
//! One test in its own binary: it points HOME and PATH at a temp directory,
//! which is process-wide, and llama's grammar support is probed once per
//! process.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// No grammar support. Every pass answers prose; the synthesis fails when
/// FAKE_LLAMA_MODE=synthesis-fails (it is the pass whose prompt is a fold).
const FAKE_LLAMA: &str = r#"#!/bin/sh
prompt=; prev=
for a in "$@"; do
  [ "$a" = --help ] && { echo "  -f FNAME  prompt file"; exit 0; }
  [ "$prev" = -f ] && prompt=$a
  prev=$a
done
case "$(basename "$prompt")" in
  pass*) if [ "$FAKE_LLAMA_MODE" = synthesis-fails ]; then
           echo "llama_completion: error: unable to read prompt" >&2; exit 1
         fi
         echo "SYNTHESIS: the book asks its reader to weigh the evidence." ;;
  *) echo "PROSE ANALYSIS: the chapter says \"weigh the evidence\"." ;;
esac
"#;

fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(root: &Path, tag: &str) -> (Result<(), String>, PathBuf) {
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
    model_loader::cancel::clear();
    let r = ui::commands::start_run(state, manifest_path, run_dir.clone()).map_err(|e| format!("{e:?}"));
    (r, run_dir)
}

/// The report PDF's text, read back with the app's own extractor.
fn report_text(run_dir: &Path) -> String {
    let report = std::fs::read_dir(run_dir).unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with("_AiSmartGuy_Report.pdf"))
        .unwrap_or_else(|| panic!("no report PDF in {}", run_dir.display()))
        .path();
    pdf_io::extract_text(&report).unwrap().pages.join("\n")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_run_without_grammar_support_falls_back_to_prose_and_says_so() {
    let root = std::env::temp_dir().join(format!("asg_run_prose_fallback_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let llama_dir = home.join(".aismartguy/llama-cpp");
    let rag = home.join(".aismartguy/rag_defaults");
    for d in [&llama_dir, &rag, &root.join("bin"), &root.join("models")] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_exe(&llama_dir.join("llama-completion"), FAKE_LLAMA);
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/rag_defaults");
    for e in std::fs::read_dir(&assets).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), rag.join(e.file_name())).unwrap();
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", root.join("bin").display()));
    // The default: structured is wanted, but this llama cannot do it.
    std::env::remove_var("ASG_STRUCTURED_FINDINGS");
    std::fs::File::create(root.join("models/model.gguf")).unwrap().set_len(64 << 20).unwrap();
    let text = "Chapter One\n\nThe argument of this chapter is that every reader should \
                think for themselves, weigh the evidence, and distrust any claim that \
                arrives without its reasons.\n".repeat(20);
    pdf_io::write_final_pdf(&root.join("unused.pdf"), &text, "{}", &root.join("book.pdf")).unwrap();
    let disclosure = squash(orchestrator::findings::PROSE_DISCLOSURE);
    let title = "AiSmartGuy \u{2014} Analysis Results";

    // ── With a synthesis. ──
    std::env::set_var("FAKE_LLAMA_MODE", "ok");
    let (result, run_dir) = run(&root, "with-synthesis");
    assert!(result.is_ok(), "the run failed: {result:?}");
    assert!(!run_dir.join("findings.json").exists(), "premise: the run fell back to prose");
    let report = squash(&report_text(&run_dir));
    assert!(report.starts_with(&format!("{title} {disclosure} SYNTHESIS:")), "{report}");

    // ── The synthesis fails: the per-chapter prose is the report. ──
    std::env::set_var("FAKE_LLAMA_MODE", "synthesis-fails");
    let (result, run_dir) = run(&root, "no-synthesis");
    assert!(result.is_ok(), "the run failed: {result:?}");
    let report = squash(&report_text(&run_dir));
    assert!(report.starts_with(&format!("{title} {disclosure}")), "{report}");
    assert!(report.contains("PROSE ANALYSIS:"), "{report}");

    let _ = std::fs::remove_dir_all(&root);
}
