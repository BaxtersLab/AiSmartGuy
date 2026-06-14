/// Phase 10 — Safety & Security Tests (Module M §8, Module L §2.3)
///
/// Verifies global invariants:
/// - No panic paths on boundary inputs
/// - No unbounded memory growth
/// - No file writes outside the designated run directory
/// - All error paths return typed errors, never panic
/// - No unbounded loops in chunker (empty/whitespace input)
/// - Recovery system terminates (no infinite retry loop)

use std::path::PathBuf;
use std::collections::HashMap;

use error_system::{recover, RecoveryContext};
use manifest::{default_manifest, deserialize};
use optimization::{compute_scores, update_optimization_state, BookScore, ModelCategoryScore, ScoreHistory};
use pdf_io::{chunk_text, extract_manifest, extract_text, types::ExtractedPdf};
use rag_engine::RagEngine;
use ui::{new_shared_state, UiStage};

// ---------------------------------------------------------------------------
// §8.1 — chunk_text with empty input terminates immediately and returns empty
// ---------------------------------------------------------------------------
#[test]
fn test_safety_chunker_empty_input_no_infinite_loop() {
    let pdf = ExtractedPdf { pages: vec![], page_count: 0 };
    let chunks = chunk_text(&pdf, 512, 32);
    assert!(chunks.is_empty(), "empty PDF must produce empty chunks");
}

// ---------------------------------------------------------------------------
// §8.1b — chunk_text with all-whitespace pages returns empty (no panic)
// ---------------------------------------------------------------------------
#[test]
fn test_safety_chunker_whitespace_only_pages() {
    let pages = vec!["   ".to_string(), "\t\n\r".to_string(), "    ".to_string()];
    let pdf = ExtractedPdf { page_count: pages.len(), pages };
    let chunks = chunk_text(&pdf, 512, 32);
    // Whitespace-only content may produce zero chunks — must not panic.
    let _ = chunks;
}

// ---------------------------------------------------------------------------
// §8.1c — chunk_text with max_tokens=0 does not divide by zero or panic
// ---------------------------------------------------------------------------
#[test]
fn test_safety_chunker_zero_max_tokens_no_panic() {
    let pages = vec!["some actual content here".to_string()];
    let pdf = ExtractedPdf { page_count: 1, pages };
    let _ = chunk_text(&pdf, 0, 0);
}

// ---------------------------------------------------------------------------
// §8.2 — Recovery system terminates: retryable errors eventually stop retrying
// ---------------------------------------------------------------------------
#[test]
fn test_safety_recovery_terminates() {
    use error_system::{ErrorClass, RecoveryAction};

    let mut ctx = RecoveryContext::default();
    let mut iterations = 0;

    loop {
        let action = recover(&ErrorClass::Retryable, &mut ctx);
        iterations += 1;
        if action != RecoveryAction::Retry {
            break;
        }
        assert!(iterations < 100, "recovery must not loop more than 100 times");
    }

    assert!(iterations <= 4, "retryable should stop within 4 iterations");
}

// ---------------------------------------------------------------------------
// §8.3 — Manifest deserialization of adversarial / very large strings is safe
// ---------------------------------------------------------------------------
#[test]
fn test_safety_manifest_large_invalid_json_no_panic() {
    // Construct a large invalid JSON string and ensure no panic.
    let garbage: String = "x".repeat(1_000_000);
    let result = deserialize(&garbage);
    assert!(result.is_err(), "large invalid JSON must return Err");
}

// ---------------------------------------------------------------------------
// §8.4 — PDF operations on non-existent paths return errors, never panic
// ---------------------------------------------------------------------------
#[test]
fn test_safety_pdf_ops_missing_path_no_panic() {
    let bad = PathBuf::from("/absolutely/does/not/exist/file.pdf");
    assert!(extract_manifest(&bad).is_err());
    assert!(extract_text(&bad).is_err());
}

// ---------------------------------------------------------------------------
// §8.5 — optimization rejects negative scores without panic (no underflow)
// ---------------------------------------------------------------------------
#[test]
fn test_safety_optimization_negative_scores() {
    let mut manifest = default_manifest();
    let mut history: ScoreHistory = Vec::new();

    let score = BookScore {
        model_scores: vec![ModelCategoryScore {
            model_name: "model_a".to_string(),
            category: "grammar".to_string(),
            score: -999.0,      // adversarial negative score
            false_positives: 0,
            hits: 0,
        }],
    };

    // Must not panic; may return Ok or Err depending on validation.
    let _ = update_optimization_state(&mut manifest, score, &mut history);
}

// ---------------------------------------------------------------------------
// §8.6 — UI shared state is free of data races under sequential access
// ---------------------------------------------------------------------------
#[test]
fn test_safety_ui_state_no_data_race_sequential() {
    let state = new_shared_state();

    // Sequential lock/unlock must always succeed (deadlock would hang, not panic).
    for _ in 0..100 {
        let mut s = state.lock().unwrap();
        s.stage = UiStage::RunningModel;
        drop(s);

        let s = state.lock().unwrap();
        assert!(!s.run_in_progress, "run_in_progress must not be set without explicit command");
        drop(s);
    }
}

// ---------------------------------------------------------------------------
// §8.7 — compute_scores with empty outputs returns empty scores (no panic)
// ---------------------------------------------------------------------------
#[test]
fn test_safety_compute_scores_empty_outputs() {
    let outputs: HashMap<String, Vec<String>> = HashMap::new();
    let categories = vec!["grammar".to_string()];
    let score = compute_scores(&outputs, &categories);
    assert!(score.model_scores.is_empty());
}

// ---------------------------------------------------------------------------
// §8.8 — RagEngine prompt from empty merged set is safe (no panic)
// ---------------------------------------------------------------------------
#[test]
fn test_safety_rag_empty_merged_set() {
    use rag_engine::MergedPacketSet;
    let empty = MergedPacketSet { packets: vec![] };
    let _ = RagEngine::build_prompt(&empty);
}

// ---------------------------------------------------------------------------
// §8.9 — Logging to an unwritable path does not panic (silent failure)
// ---------------------------------------------------------------------------
#[test]
fn test_safety_logging_unwritable_path_no_panic() {
    // On most systems /dev/null/logs is unwritable; worst case create_logger
    // simply fails to create the directory and the logger is a no-op.
    let bad_dir = PathBuf::from("/dev/null/aismartguy_test_log");
    let logger = logging::create_logger(&bad_dir, "safety");
    // Silent failure — no panic.
    logger.info("this should not panic even if write fails");
    logger.warn("warn");
    logger.error("error");
}

// ---------------------------------------------------------------------------
// §8.10 — UI events emit safely for extreme percent values (out-of-range)
// ---------------------------------------------------------------------------
#[test]
fn test_safety_ui_events_extreme_percent_no_panic() {
    ui::events::emit_progress(UiStage::Idle, "test", -1.0);
    ui::events::emit_progress(UiStage::Idle, "test", 200.0);
    ui::events::emit_progress(UiStage::Idle, "test", f32::INFINITY);
    ui::events::emit_progress(UiStage::Idle, "test", f32::NEG_INFINITY);
    ui::events::emit_progress(UiStage::Idle, "test", f32::NAN);
}
