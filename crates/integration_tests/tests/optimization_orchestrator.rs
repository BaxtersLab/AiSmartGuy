/// Integration Tests — Optimization + Orchestrator (Module M §4.4)
///
/// Tests:
/// - 10-book cycle increments books_completed correctly
/// - consensus logic: stable winner is identified after 10 identical outcomes
/// - optimized_available flag is set after a complete 10-book run
/// - update_optimization_state rejects empty book_score

use std::collections::HashMap;
use manifest::default_manifest;
use optimization::{
    compute_scores, update_optimization_state, BookScore, ModelCategoryScore, ScoreHistory,
};

fn make_book_score(model: &str, category: &str, hits: u32) -> BookScore {
    BookScore {
        model_scores: vec![ModelCategoryScore {
            model_name: model.to_string(),
            category: category.to_string(),
            score: hits as f32,
            false_positives: 0,
            hits,
        }],
    }
}

// ---------------------------------------------------------------------------
// 4.4.1 — books_completed increments on each call
// ---------------------------------------------------------------------------
#[test]
fn test_optimization_books_completed_increments() {
    let mut manifest = default_manifest();
    let mut history: ScoreHistory = Vec::new();

    for i in 0..5u32 {
        let score = make_book_score("model_a", "grammar", i + 1);
        update_optimization_state(&mut manifest, score, &mut history).unwrap();
    }

    assert_eq!(manifest.optimization_state.books_completed, 5);
}

// ---------------------------------------------------------------------------
// 4.4.2 — optimized_available becomes true after 10 books with a stable winner
// ---------------------------------------------------------------------------
#[test]
fn test_optimization_optimized_flag_set_after_10_books() {
    let mut manifest = default_manifest();
    manifest.categories_active = vec!["grammar".to_string()];
    let mut history: ScoreHistory = Vec::new();

    for _ in 0..10 {
        // model_a consistently wins grammar with score 10.0
        let score = BookScore {
            model_scores: vec![
                ModelCategoryScore {
                    model_name: "model_a".to_string(),
                    category: "grammar".to_string(),
                    score: 10.0,
                    false_positives: 0,
                    hits: 10,
                },
                ModelCategoryScore {
                    model_name: "model_b".to_string(),
                    category: "grammar".to_string(),
                    score: 2.0,
                    false_positives: 0,
                    hits: 2,
                },
            ],
        };
        update_optimization_state(&mut manifest, score, &mut history).unwrap();
    }

    assert_eq!(manifest.optimization_state.books_completed, 10);
    // After 10 books with a dominant winner, optimized_available should be set.
    assert!(
        manifest.optimization_state.optimized_available,
        "optimized_available must be true after 10 books with a stable winner"
    );
}

// ---------------------------------------------------------------------------
// 4.4.3 — Empty book_score returns an error (not a panic)
// ---------------------------------------------------------------------------
#[test]
fn test_optimization_empty_book_score_returns_error() {
    let mut manifest = default_manifest();
    let mut history: ScoreHistory = Vec::new();
    let empty = BookScore { model_scores: vec![] };
    let result = update_optimization_state(&mut manifest, empty, &mut history);
    assert!(result.is_err(), "empty book_score must return Err");
}

// ---------------------------------------------------------------------------
// 4.4.4 — compute_scores produces non-empty output for valid inputs
// ---------------------------------------------------------------------------
#[test]
fn test_compute_scores_produces_output() {
    let mut outputs: HashMap<String, Vec<String>> = HashMap::new();
    outputs.insert(
        "model_a".to_string(),
        vec!["grammar error detected here grammar".to_string()],
    );
    let categories = vec!["grammar".to_string()];

    let book_score = compute_scores(&outputs, &categories);
    assert!(!book_score.model_scores.is_empty());
    let score = &book_score.model_scores[0];
    assert_eq!(score.model_name, "model_a");
    assert_eq!(score.category, "grammar");
    assert!(score.hits >= 2, "should detect 2+ occurrences of 'grammar'");
}

// ---------------------------------------------------------------------------
// 4.4.5 — compute_scores with no categories returns empty score list
// ---------------------------------------------------------------------------
#[test]
fn test_compute_scores_no_categories_returns_empty() {
    let mut outputs: HashMap<String, Vec<String>> = HashMap::new();
    outputs.insert("model_a".to_string(), vec!["some text".to_string()]);

    let book_score = compute_scores(&outputs, &[]);
    assert!(book_score.model_scores.is_empty());
}

// ---------------------------------------------------------------------------
// 4.4.6 — score history grows by 1 per call
// ---------------------------------------------------------------------------
#[test]
fn test_optimization_history_grows() {
    let mut manifest = default_manifest();
    let mut history: ScoreHistory = Vec::new();

    for i in 0..3 {
        let score = make_book_score("model_a", "style", (i + 1) as u32);
        update_optimization_state(&mut manifest, score, &mut history).unwrap();
    }

    assert_eq!(history.len(), 3);
}
