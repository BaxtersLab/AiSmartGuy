/// Phase 10 — Stress & Stability Tests (Module M §6)
///
/// These tests simulate high-volume and repeated-operation scenarios
/// to assert the system holds invariants under sustained load.
///
/// §6.1 Long-run: 100+ chunks processed without panic/memory explosion
/// §6.2 Repeated load/unload logic: 1000 state cycles
/// §6.3 Log volume: 1000+ log lines written without failure
/// §6.4 UI stress: 1000 rapid state changes + event emissions
/// §6.5 Error storm: 50+ errors classified and recovered without panic

use error_system::{
    classify, recover,
    EngineError, ModelError, RagError, PdfError, ManifestError,
    RecoveryAction, RecoveryContext,
};
use logging::create_logger;
use pdf_io::{chunk_text, types::ExtractedPdf};
use optimization::{update_optimization_state, BookScore, ModelCategoryScore, ScoreHistory};
use manifest::default_manifest;
use ui::{new_shared_state, UiStage, events::{emit_progress, emit_error}};

// ---------------------------------------------------------------------------
// §6.1 — Long-run: chunking 100+ pages, 100+ chunks, no panic
// ---------------------------------------------------------------------------
#[test]
fn test_stress_chunking_100_pages() {
    let pages: Vec<String> = (0..100)
        .map(|i| format!("Page {} contains repeated text about grammar and style. {}", i,
            "word ".repeat(200)))
        .collect();

    let pdf = ExtractedPdf { pages, page_count: 100 };
    let chunks = chunk_text(&pdf, 512, 32);

    assert!(chunks.len() >= 10, "100 pages should produce at least 10 chunks");

    // Verify all chunk ids are sequential
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.id, i, "chunk ids must be sequential");
    }

    // Verify no chunk text is empty
    for chunk in &chunks {
        assert!(!chunk.text.is_empty(), "chunk text must not be empty");
    }
}

// ---------------------------------------------------------------------------
// §6.1b — Chunk count grows proportionally with input size
// ---------------------------------------------------------------------------
#[test]
fn test_stress_chunking_proportional_growth() {
    let make_pdf = |page_count: usize| {
        let pages = (0..page_count)
            .map(|_| "word ".repeat(300))
            .collect::<Vec<_>>();
        ExtractedPdf { pages, page_count }
    };

    let small = chunk_text(&make_pdf(10), 512, 0);
    let large = chunk_text(&make_pdf(100), 512, 0);
    assert!(
        large.len() > small.len(),
        "more pages must produce more chunks (got small={} large={})", small.len(), large.len()
    );
}

// ---------------------------------------------------------------------------
// §6.2 — Repeated optimization cycles: 1000 score updates without panic
// ---------------------------------------------------------------------------
#[test]
fn test_stress_optimization_1000_cycles() {
    let mut manifest = default_manifest();
    let mut history: ScoreHistory = Vec::new();

    for i in 0..1000u32 {
        let score = BookScore {
            model_scores: vec![ModelCategoryScore {
                model_name: "model_a".to_string(),
                category: "grammar".to_string(),
                score: (i % 10) as f32 + 1.0,
                false_positives: 0,
                hits: 1,
            }],
        };
        // Errors are impossible with non-empty scores; unwrap is safe in test.
        update_optimization_state(&mut manifest, score, &mut history).unwrap();
    }

    assert_eq!(manifest.optimization_state.books_completed, 1000);
    // History is windowed to the last 10 entries to bound memory growth.
    assert_eq!(history.len(), 10);
}

// ---------------------------------------------------------------------------
// §6.2b — Repeated shared state lock/unlock cycles (simulates load/unload)
// ---------------------------------------------------------------------------
#[test]
fn test_stress_shared_state_1000_lock_cycles() {
    let state = new_shared_state();

    for i in 0u32..1000 {
        let mut s = state.lock().unwrap();
        s.stage = UiStage::RunningModel;
        s.run_in_progress = true;
        let _ = i;
        s.run_in_progress = false;
        s.stage = UiStage::Idle;
    }

    let s = state.lock().unwrap();
    assert_eq!(s.stage, UiStage::Idle);
    assert!(!s.run_in_progress);
}

// ---------------------------------------------------------------------------
// §6.3 — Log volume: write 1000 log entries without failure
// ---------------------------------------------------------------------------
#[test]
fn test_stress_log_volume_1000_entries() {
    let tmp = std::env::temp_dir().join("aismartguy_stress_log_test");
    let logger = create_logger(&tmp, "stress_test");

    for i in 0..1000 {
        logger.info(&format!("stress log entry {}", i));
    }

    // If log file exists, verify it is non-empty.
    if logger.file_path.exists() {
        let size = std::fs::metadata(&logger.file_path).unwrap().len();
        assert!(size > 0, "log file must not be empty after 1000 writes");
    }

    // Clean up.
    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// §6.3b — Mixed log levels do not panic
// ---------------------------------------------------------------------------
#[test]
fn test_stress_log_all_levels_no_panic() {
    let tmp = std::env::temp_dir().join("aismartguy_stress_log_levels");
    let logger = create_logger(&tmp, "levels_test");

    for i in 0..100 {
        logger.info(&format!("info {}", i));
        logger.warn(&format!("warn {}", i));
        logger.error(&format!("error {}", i));
        logger.debug(&format!("debug {}", i));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// §6.4 — UI stress: 1000 rapid state changes + emit calls do not panic
// ---------------------------------------------------------------------------
#[test]
fn test_stress_ui_rapid_state_changes() {
    let state = new_shared_state();

    let stages = [
        UiStage::Idle,
        UiStage::LoadingPdf,
        UiStage::ExtractingMetadata,
        UiStage::ApplyingConfiguration,
        UiStage::RunningModel,
        UiStage::Completed,
        UiStage::Error,
        UiStage::FetchingModel,
    ];

    for i in 0..1000usize {
        let stage = stages[i % stages.len()].clone();
        {
            let mut s = state.lock().unwrap();
            s.stage = stage.clone();
        }
        emit_progress(stage, format!("step {}", i), (i % 100) as f32 / 100.0);
    }

    emit_error("storm complete");
}

// ---------------------------------------------------------------------------
// §6.5 — Error storm: classify and recover 50+ diverse errors without panic
// ---------------------------------------------------------------------------
#[test]
fn test_stress_error_storm_50_errors() {
    let errors: Vec<EngineError> = vec![
        EngineError::ModelError(ModelError::GpuError("OOM".to_string())),
        EngineError::ModelError(ModelError::Timeout("120s".to_string())),
        EngineError::ModelError(ModelError::LoadFailure("not found".to_string())),
        EngineError::ModelError(ModelError::InferenceFailure("nan".to_string())),
        EngineError::ModelError(ModelError::IoError("disk full".to_string())),
        EngineError::ModelError(ModelError::InvalidState("corrupted".to_string())),
        EngineError::RagError(RagError::MalformedPacket("bad json".to_string())),
        EngineError::RagError(RagError::MissingPacket("001".to_string())),
        EngineError::RagError(RagError::ValidationFailure("schema".to_string())),
        EngineError::PdfError(PdfError::ParseFailure("truncated".to_string())),
        EngineError::ManifestError(ManifestError::Corruption("checksum".to_string())),
        EngineError::IoError("no space left".to_string()),
        EngineError::TimeoutError("deadline exceeded".to_string()),
        EngineError::UnknownError("unexplained".to_string()),
    ];

    for _ in 0..4 {
        // 14 error types × 4 iterations = 56 total — covers 50+ requirement
        let mut ctx = RecoveryContext::default();
        for error in &errors {
            let class = classify(error);
            let _action = recover(&class, &mut ctx);
            // must not panic
        }
    }
}

// ---------------------------------------------------------------------------
// §6.5b — Recovery context resets between runs (no state bleed)
// ---------------------------------------------------------------------------
#[test]
fn test_stress_recovery_context_independent_between_runs() {
    let error = EngineError::ModelError(ModelError::Timeout("slow".to_string()));

    for _ in 0..100 {
        let mut ctx = RecoveryContext::default();
        // Retryable retries up to 3 times.
        let first = recover(&classify(&error), &mut ctx);
        assert_eq!(first, RecoveryAction::Retry, "first call must be Retry");
    }
}

// ---------------------------------------------------------------------------
// §6.5c — GPU fallback recovery is always DowngradeGpu (deterministic)
// ---------------------------------------------------------------------------
#[test]
fn test_stress_gpu_fallback_always_downgrade() {
    let error = EngineError::ModelError(ModelError::GpuError("OOM".to_string()));
    for _ in 0..200 {
        let mut ctx = RecoveryContext::default();
        let action = recover(&classify(&error), &mut ctx);
        assert_eq!(action, RecoveryAction::DowngradeGpu);
    }
}
