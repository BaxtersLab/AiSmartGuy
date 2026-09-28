//! A whole structured-findings run -- the GUI's own manifest builder and
//! `ui::commands::start_run` -- with every quote checked against the chapter
//! it cites (ASG-Q3). llama.cpp is replaced by a TEST DOUBLE that answers
//! every chapter pass with two findings, one quoting the book and one
//! quoting a sentence the book does not contain. Everything else is the
//! product's.
//!
//! One test in its own binary: it points HOME and PATH at a temp directory,
//! which is process-wide.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const REAL_QUOTE: &str = "weigh the evidence, and distrust any claim";
const INVENTED_QUOTE: &str = "the numbers were cooked from the start";

/// `--help` advertises grammars, so the run takes the findings path. A pass
/// with a grammar is a chapter pass: it answers findings.json. The one pass
/// without is the synthesis, which answers prose, or fails when
/// FAKE_LLAMA_MODE=synthesis-fails.
const FAKE_LLAMA: &str = r#"#!/bin/sh
here=$(dirname "$0"); grammar=; prev=
for a in "$@"; do
  [ "$a" = --help ] && { echo "  --grammar-file FNAME  file to read grammar from"; exit 0; }
  [ "$prev" = --grammar-file ] && grammar=$a
  prev=$a
done
if [ -n "$grammar" ]; then cat "$here/findings.json"; exit 0; fi
if [ "$FAKE_LLAMA_MODE" = synthesis-fails ]; then
  echo "llama_completion: error: unable to read prompt" >&2; exit 1
fi
echo "SYNTHESIS: the book asks its reader to weigh the evidence."
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
fn every_quote_in_a_findings_run_is_checked_and_labelled() {
    let root = std::env::temp_dir().join(format!("asg_run_quote_check_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let llama_dir = home.join(".aismartguy/llama-cpp");
    let rag = home.join(".aismartguy/rag_defaults");
    for d in [&llama_dir, &rag, &root.join("bin"), &root.join("models")] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_exe(&llama_dir.join("llama-completion"), FAKE_LLAMA);
    let findings = serde_json::json!([
        {"rule_id": "FAL-01", "quote": REAL_QUOTE, "location": "p1", "severity": "low", "note": "real"},
        {"rule_id": "FAL-02", "quote": INVENTED_QUOTE, "location": "p2", "severity": "high", "note": "invented"}
    ]);
    std::fs::write(llama_dir.join("findings.json"), findings.to_string()).unwrap();
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/rag_defaults");
    for e in std::fs::read_dir(&assets).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), rag.join(e.file_name())).unwrap();
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", root.join("bin").display()));
    std::env::set_var("ASG_STRUCTURED_FINDINGS", "1");
    std::fs::File::create(root.join("models/model.gguf")).unwrap().set_len(64 << 20).unwrap();
    let text = "Chapter One\n\nThe argument of this chapter is that every reader should \
                think for themselves, weigh the evidence, and distrust any claim that \
                arrives without its reasons.\n".repeat(20);
    pdf_io::write_final_pdf(&root.join("unused.pdf"), &text, "{}", &root.join("book.pdf")).unwrap();

    // ── A run with a synthesis. ──
    std::env::set_var("FAKE_LLAMA_MODE", "ok");
    let (result, run_dir) = run(&root, "with-synthesis");
    assert!(result.is_ok(), "the run failed: {result:?}");

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("findings.json")).unwrap()).unwrap();
    let all = json["findings"].as_array().unwrap();
    assert!(!all.is_empty(), "premise: chapter passes produced findings");
    assert_eq!(json["quotes_total"], all.len());
    assert_eq!(json["quotes_verified"].as_u64().unwrap() * 2, all.len() as u64, "{json}");
    for f in all {
        match f["note"].as_str().unwrap() {
            "real" => {
                assert_eq!(f["quote_check"], "Quote verified in chapter", "{f}");
                assert!(f["source_offsets"]["end"].as_u64() > f["source_offsets"]["start"].as_u64(), "{f}");
                assert_eq!(f["quote"], REAL_QUOTE, "kept exactly as given");
            }
            _ => {
                assert_eq!(f["quote_check"], "Quote not found in chapter", "{f}");
                assert!(f["source_offsets"].is_null(), "{f}");
            }
        }
    }
    let header = format!("Quotes verified: {} of {}", json["quotes_verified"], json["quotes_total"]);
    let table = std::fs::read_to_string(run_dir.join("findings_table.md")).unwrap();
    assert!(table.starts_with(&format!("# Findings\n\n{header}\n")), "{table}");

    // The synthesis keeps the unsupported inference but is never handed the
    // invented quote.
    let fold: Vec<String> = std::fs::read_dir(run_dir.join("prompts/fusion")).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("pass"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    assert!(!fold.is_empty(), "premise: the synthesis prompt was written");
    for prompt in &fold {
        assert!(!prompt.contains(INVENTED_QUOTE), "the synthesis was handed the invented quote");
        assert!(prompt.contains(REAL_QUOTE), "the evidence must reach the synthesis");
        assert!(prompt.contains("Unsupported model inference"), "the inference must stay");
    }

    // The count heads the report, directly under the PDF writer's title.
    let report = squash(&report_text(&run_dir));
    assert!(report.starts_with(&format!("AiSmartGuy \u{2014} Analysis Results {header}")), "{report}");
    assert!(report.contains("SYNTHESIS:"), "{report}");

    // ── The synthesis fails: the checked table is the report. ──
    std::env::set_var("FAKE_LLAMA_MODE", "synthesis-fails");
    let (result, run_dir) = run(&root, "no-synthesis");
    assert!(result.is_ok(), "the run failed: {result:?}");
    let report = squash(&report_text(&run_dir));
    assert!(report.contains(&header), "{report}");
    assert!(report.contains("Quote verified in chapter 1"), "{report}");
    assert!(report.contains(&format!("Model's claimed quote (not found in chapter): \"{INVENTED_QUOTE}\"")), "{report}");
    assert!(!report.contains("\"rule_id\""), "raw findings JSON, unlabelled, reached the report: {report}");
    assert!(!report.contains("SYNTHESIS:"));

    let _ = std::fs::remove_dir_all(&root);
}
