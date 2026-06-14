/// Integration Tests — UI + Orchestrator (Module M §4.5)
///
/// Tests (no real Tauri runtime needed — plain function calls):
/// - progress events do not panic (stdout JSON lines)
/// - shared state transitions are correct
/// - load_pdf with a missing file returns UiError
/// - apply_configuration with a missing manifest returns UiError
/// - resolve_conflict removes the correct entry from state
/// - cancel_run flips the AtomicBool
/// - cancel_model_download removes the matching download entry
/// - state.conflicts is empty for a fresh default state

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ui::{
    new_shared_state, resolve_conflict, cancel_run, cancel_model_download,
    UiConflict, UiStage, ModelDownloadStatus,
    commands::{load_pdf, apply_configuration},
};

// ---------------------------------------------------------------------------
// 4.5.1 — Default shared state has no conflicts and is Idle
// ---------------------------------------------------------------------------
#[test]
fn test_ui_default_state_is_idle_no_conflicts() {
    let state = new_shared_state();
    let s = state.lock().unwrap();
    assert_eq!(s.stage, UiStage::Idle);
    assert!(s.conflicts.is_empty());
    assert!(!s.run_in_progress);
    assert!(!s.pdf_loaded);
}

// ---------------------------------------------------------------------------
// 4.5.2 — load_pdf with a missing file returns UiError::PdfError
// ---------------------------------------------------------------------------
#[test]
fn test_ui_load_pdf_missing_file_returns_error() {
    let state = new_shared_state();
    let bad = PathBuf::from("/nonexistent/path/test.pdf");
    let result = load_pdf(state, &bad);
    assert!(result.is_err(), "load_pdf with missing file must return Err");
}

// ---------------------------------------------------------------------------
// 4.5.3 — apply_configuration with a missing manifest returns error
// ---------------------------------------------------------------------------
#[test]
fn test_ui_apply_configuration_missing_manifest_returns_error() {
    let state = new_shared_state();
    let bad = PathBuf::from("/nonexistent/manifest.json");
    let result = apply_configuration(state, &bad);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 4.5.4 — resolve_conflict removes matching conflict from state
// ---------------------------------------------------------------------------
#[test]
fn test_ui_resolve_conflict_removes_entry() {
    let state = new_shared_state();
    {
        let mut s = state.lock().unwrap();
        s.conflicts.push(UiConflict::MissingRagPacket { packet_id: "001".to_string() });
        s.conflicts.push(UiConflict::MissingModel {
            name: "model_a".to_string(),
            path: "/model/path".to_string(),
        });
    }

    resolve_conflict(
        state.clone(),
        UiConflict::MissingRagPacket { packet_id: "anything".to_string() },
    ).unwrap();

    let s = state.lock().unwrap();
    // All MissingRagPacket variants are removed by discriminant.
    assert!(
        s.conflicts.iter().all(|c| !matches!(c, UiConflict::MissingRagPacket { .. })),
        "MissingRagPacket conflict should be removed"
    );
    // MissingModel should still be present.
    assert!(
        s.conflicts.iter().any(|c| matches!(c, UiConflict::MissingModel { .. })),
        "MissingModel conflict should remain"
    );
}

// ---------------------------------------------------------------------------
// 4.5.5 — cancel_run sets the AtomicBool to true
// ---------------------------------------------------------------------------
#[test]
fn test_ui_cancel_run_sets_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    cancel_run(&flag);
    assert!(flag.load(Ordering::SeqCst), "cancel flag must be true after cancel_run");
}

// ---------------------------------------------------------------------------
// 4.5.6 — cancel_model_download removes the matching entry from state
// ---------------------------------------------------------------------------
#[test]
fn test_ui_cancel_model_download_removes_entry() {
    let state = new_shared_state();
    {
        let mut s = state.lock().unwrap();
        s.model_downloads.push(ModelDownloadStatus {
            model_name: "llama-7b".to_string(),
            ..Default::default()
        });
        s.model_downloads.push(ModelDownloadStatus {
            model_name: "mistral-7b".to_string(),
            ..Default::default()
        });
    }

    cancel_model_download(state.clone(), "llama-7b");

    let s = state.lock().unwrap();
    assert!(
        s.model_downloads.iter().all(|d| d.model_name != "llama-7b"),
        "llama-7b entry should be removed"
    );
    assert!(
        s.model_downloads.iter().any(|d| d.model_name == "mistral-7b"),
        "mistral-7b entry should remain"
    );
}

// ---------------------------------------------------------------------------
// 4.5.7 — progress event emission does not panic
// ---------------------------------------------------------------------------
#[test]
fn test_ui_emit_progress_no_panic() {
    // emit_progress writes to stdout; just ensure it does not panic.
    ui::events::emit_progress(UiStage::RunningModel, "test message", 0.5);
    ui::events::emit_error("test error message");
    ui::events::emit_run_completed("/tmp/output.pdf");
}
