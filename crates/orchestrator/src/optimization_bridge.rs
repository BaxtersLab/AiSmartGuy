use std::collections::HashMap;
use std::path::PathBuf;

use manifest::Manifest;
use optimization::{compute_scores, update_optimization_state, BookScore, ScoreHistory};
use rag_engine::active_slugs;

use crate::errors::{OrchestratorError, OrchestratorResult};
use crate::progress::emit_progress;
use crate::types::OrchestratorProgressEvent;

/// Bridge: score model outputs against hitlist categories, then feed results
/// to the optimization engine.
///
/// `model_output_paths` maps model names to their ordered output file paths.
/// Reads the output texts, computes per-category scores, and updates
/// `manifest.optimization_state` through the optimisation lifecycle.
pub fn run_optimization_pass(
    manifest: &mut Manifest,
    model_output_paths: &HashMap<String, Vec<PathBuf>>,
    history: &mut ScoreHistory,
) -> OrchestratorResult<()> {
    emit_progress(&OrchestratorProgressEvent {
        stage: "OPTIMIZING".into(),
        message: "scoring model outputs against hitlist categories".into(),
        percent: 0.93,
    });

    // Use the manifest's active categories, falling back to the default catalog.
    let categories: Vec<String> = if manifest.categories_active.is_empty() {
        let slugs = active_slugs();
        manifest.categories_active = slugs.clone();
        slugs
    } else {
        manifest.categories_active.clone()
    };

    if categories.is_empty() {
        eprintln!("[optimization][WARN] no active hitlist categories — skipping optimisation pass");
        return Ok(());
    }

    // Build model_outputs: model_name → chunk output texts.
    let mut model_outputs: HashMap<String, Vec<String>> = HashMap::new();
    for (model_name, paths) in model_output_paths {
        let texts: Vec<String> = paths
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .collect();
        model_outputs.insert(model_name.clone(), texts);
    }

    if model_outputs.is_empty() {
        eprintln!("[optimization][WARN] no model outputs to score — skipping optimisation pass");
        return Ok(());
    }

    // Compute per-category scores for this book.
    let book_score: BookScore = compute_scores(&model_outputs, &categories);

    // Feed into the optimization lifecycle (handles aggregation, consensus, mapping).
    update_optimization_state(manifest, book_score, history)
        .map_err(|e| OrchestratorError::OptimizationError(format!("{:?}", e)))?;

    emit_progress(&OrchestratorProgressEvent {
        stage: "OPTIMIZING".into(),
        message: format!(
            "optimization pass complete — books completed: {}",
            manifest.optimization_state.books_completed
        ),
        percent: 0.94,
    });

    Ok(())
}
