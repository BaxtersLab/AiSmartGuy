/// Phase 10 — Determinism Tests (Module M §7)
///
/// AiSmartGuy must be fully deterministic:
/// - same PDF text → same chunks
/// - same RAG packets → same prompt
/// - same manifest → same serialization
/// - same errors → same recovery actions
/// - same book scores → same optimization result

use std::collections::HashMap;
use std::path::PathBuf;

use manifest::{default_manifest, serialize, ModelConfig, SourcePdf};
use optimization::{compute_scores, update_optimization_state, BookScore, ModelCategoryScore, ScoreHistory};
use pdf_io::{chunk_text, types::ExtractedPdf};
use rag_engine::{
    RagEngine, RagPacket, RagRule, HitConditions, ModelBehavior,
    InjectLocation, PatternType, Severity,
};
use error_system::{
    classify, recover, EngineError, ModelError, RecoveryContext,
};

// ---------------------------------------------------------------------------
// §7.1 — Same PDF text always produces identical chunks
// ---------------------------------------------------------------------------
#[test]
fn test_determinism_chunking_identical_output() {
    let pages = vec![
        "The quick brown fox jumps over the lazy dog. ".repeat(50),
        "Grammar is the foundation of all language. ".repeat(40),
        "Style guides determine tone and voice. ".repeat(60),
    ];
    let pdf = ExtractedPdf { page_count: pages.len(), pages };

    let run_a = chunk_text(&pdf, 512, 32);
    let run_b = chunk_text(&pdf, 512, 32);

    assert_eq!(run_a.len(), run_b.len(), "chunk count must be identical across runs");
    for (a, b) in run_a.iter().zip(run_b.iter()) {
        assert_eq!(a.text, b.text, "chunk text must be identical across runs");
        assert_eq!(a.id, b.id);
        assert_eq!(a.start_page, b.start_page);
        assert_eq!(a.end_page, b.end_page);
    }
}

// ---------------------------------------------------------------------------
// §7.2 — Same RAG packets always produce identical prompt
// ---------------------------------------------------------------------------
fn make_packet(id: u16, category: &str) -> RagPacket {
    RagPacket {
        packet_id: id,
        category: category.to_string(),
        description: format!("packet-{}", id),
        version: "1.0.0".to_string(),
        rules: vec![RagRule {
            rule_id: format!("R{}", id),
            name: format!("rule-{}", id),
            pattern_type: PatternType::Linguistic,
            patterns: vec!["sample".to_string()],
            severity: Severity::Medium,
            explanation: "determinism test".to_string(),
        }],
        hit_conditions: HitConditions { min_pattern_matches: 1, confidence_weight: 0.8 },
        model_behavior: ModelBehavior { inject_as: InjectLocation::SystemPrompt, priority: 3 },
        source_path: PathBuf::new(),
    }
}

#[test]
fn test_determinism_rag_prompt_identical_output() {
    let packets = vec![
        make_packet(1, "grammar"),
        make_packet(2, "style"),
        make_packet(3, "factual"),
    ];

    let merged_a = RagEngine::merge_packets(packets.clone());
    let merged_b = RagEngine::merge_packets(packets.clone());

    let prompt_a = RagEngine::build_prompt(&merged_a);
    let prompt_b = RagEngine::build_prompt(&merged_b);

    assert_eq!(prompt_a, prompt_b, "same packets must always produce identical prompt");
}

// ---------------------------------------------------------------------------
// §7.3 — Same manifest always serializes to identical JSON
// ---------------------------------------------------------------------------
#[test]
fn test_determinism_manifest_serialization() {
    let mut m = default_manifest();
    // Fix time-dependent fields so they are stable across runs.
    m.run_id = "Run_determinism_test_001".to_string();
    m.timestamp = "2026-04-02T00:00:00Z".to_string();
    m.source_pdf = SourcePdf {
        filename: "test.pdf".to_string(),
        hash_sha256: None,
        page_count: Some(100),
    };
    m.models.model1 = Some(ModelConfig {
        name: "model_a".to_string(),
        path: "/models/model_a.gguf".to_string(),
        quantization: "q4_k_m".to_string(),
        active: true,
        ..Default::default()
    });

    let json_a = serialize(&m).unwrap();
    let json_b = serialize(&m).unwrap();
    assert_eq!(json_a, json_b, "same manifest must always serialize to identical JSON");
}

// ---------------------------------------------------------------------------
// §7.4 — Same error always produces same classification
// ---------------------------------------------------------------------------
#[test]
fn test_determinism_error_classification() {
    let errors = vec![
        EngineError::ModelError(ModelError::GpuError("oom".to_string())),
        EngineError::ModelError(ModelError::Timeout("30s".to_string())),
        EngineError::ModelError(ModelError::LoadFailure("missing".to_string())),
        EngineError::IoError("disk".to_string()),
    ];

    for error in &errors {
        let class_a = classify(error);
        let class_b = classify(error);
        assert_eq!(
            class_a, class_b,
            "classify({:?}) must be deterministic", error
        );
    }
}

// ---------------------------------------------------------------------------
// §7.5 — Same recovery context + same error class → same action
// ---------------------------------------------------------------------------
#[test]
fn test_determinism_recovery_action() {
    use error_system::ErrorClass;

    let classes = vec![
        ErrorClass::Recoverable,
        ErrorClass::Retryable,
        ErrorClass::GpuFallback,
        ErrorClass::RagSkip,
        ErrorClass::PartialRun,
        ErrorClass::Fatal,
    ];

    for class in &classes {
        let mut ctx_a = RecoveryContext::default();
        let mut ctx_b = RecoveryContext::default();
        let action_a = recover(class, &mut ctx_a);
        let action_b = recover(class, &mut ctx_b);
        assert_eq!(
            action_a, action_b,
            "recover({:?}) must be deterministic", class
        );
    }
}

// ---------------------------------------------------------------------------
// §7.6 — Same optimization input always produces the same books_completed count
// ---------------------------------------------------------------------------
#[test]
fn test_determinism_optimization_books_completed() {
    let make_score = || BookScore {
        model_scores: vec![ModelCategoryScore {
            model_name: "model_a".to_string(),
            category: "grammar".to_string(),
            score: 7.5,
            false_positives: 0,
            hits: 3,
        }],
    };

    let run = |n: u32| {
        let mut m = default_manifest();
        let mut h: ScoreHistory = Vec::new();
        for _ in 0..n {
            update_optimization_state(&mut m, make_score(), &mut h).unwrap();
        }
        m.optimization_state.books_completed
    };

    assert_eq!(run(5), run(5), "books_completed must be identical for same run count");
    assert_eq!(run(10), run(10));
}

// ---------------------------------------------------------------------------
// §7.7 — compute_scores is deterministic for identical model outputs
// ---------------------------------------------------------------------------
#[test]
fn test_determinism_compute_scores() {
    let mut outputs: HashMap<String, Vec<String>> = HashMap::new();
    outputs.insert(
        "model_a".to_string(),
        vec!["grammar error grammar style issue".to_string()],
    );
    let categories = vec!["grammar".to_string(), "style".to_string()];

    let score_a = compute_scores(&outputs, &categories);
    let score_b = compute_scores(&outputs, &categories);

    assert_eq!(score_a.model_scores.len(), score_b.model_scores.len());
    for (a, b) in score_a.model_scores.iter().zip(score_b.model_scores.iter()) {
        assert_eq!(a.model_name, b.model_name);
        assert_eq!(a.category, b.category);
        assert_eq!(a.hits, b.hits);
    }
}
