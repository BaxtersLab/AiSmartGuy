use std::collections::HashMap;
use std::path::PathBuf;

use manifest::Manifest;
use optimization::{
    compute_scores, compute_scores_from_counts, update_optimization_state, BookScore, ScoreHistory,
};
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
/// `findings_rule_category`: when `Some(map)` (structured-findings mode),
/// per-category hit counts are derived from the parsed findings via the
/// `rule_id → category` map and scored with `compute_scores_from_counts` —
/// replacing the keyword-mention heuristic. `None` = the keyword path (default).
pub fn run_optimization_pass(
    manifest: &mut Manifest,
    model_output_paths: &HashMap<String, Vec<PathBuf>>,
    history: &mut ScoreHistory,
    findings_rule_category: Option<&HashMap<String, String>>,
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

    // Compute per-category scores for this book. In structured-findings mode,
    // use the REAL per-category hit counts (parsed findings mapped via
    // rule_id→category); otherwise fall back to the keyword-mention heuristic.
    let book_score: BookScore = if let Some(rule_cat) = findings_rule_category {
        let mut model_counts: HashMap<String, HashMap<String, u32>> = HashMap::new();
        for (model_name, texts) in &model_outputs {
            let chapters: Vec<(String, String)> = texts
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("chapter {}", i + 1), t.clone()))
                .collect();
            let merged = crate::findings::merge_findings(&chapters);
            model_counts.insert(
                model_name.clone(),
                merged.aggregate.per_category_counts(rule_cat),
            );
        }
        compute_scores_from_counts(&model_counts, &categories)
    } else {
        compute_scores(&model_outputs, &categories)
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    /// One model output in structured-findings form: two fallacy rules and
    /// one rule with no category. No category name appears in the text, so
    /// the keyword heuristic would score it zero.
    const FINDINGS_OUTPUT: &str = r#"[
        {"rule_id":"FAL-01","quote":"q1","location":"p1","severity":"high","note":"n"},
        {"rule_id":"FAL-02","quote":"q2","location":"p2","severity":"low","note":"n"},
        {"rule_id":"ZZZ-99","quote":"q3","location":"p3","severity":"low","note":"n"}
    ]"#;

    fn hits(history: &ScoreHistory, category: &str) -> u32 {
        history
            .last()
            .unwrap()
            .model_scores
            .iter()
            .find(|s| s.model_name == "model1" && s.category == category)
            .map(|s| s.hits)
            .unwrap()
    }

    /// In findings mode the score must come from the parsed findings, mapped
    /// rule → category, not from counting category keywords in the text.
    #[test]
    fn findings_mode_scores_real_rule_counts_by_category() {
        let dir = std::env::temp_dir().join(format!("asg_optbridge_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("chapter_001_output.txt");
        std::fs::write(&out, FINDINGS_OUTPUT).unwrap();
        let mut outputs = HashMap::new();
        outputs.insert("model1".to_string(), vec![out.clone()]);

        let mut rule_category = HashMap::new();
        rule_category.insert("FAL-01".to_string(), "fallacies".to_string());
        rule_category.insert("FAL-02".to_string(), "fallacies".to_string());

        let mut manifest = manifest::default_manifest();
        manifest.categories_active = vec!["fallacies".to_string(), "nlp_techniques".to_string()];

        let mut findings_history = ScoreHistory::new();
        let findings = run_optimization_pass(
            &mut manifest.clone(), &outputs, &mut findings_history, Some(&rule_category));
        // Control: the same output on the keyword path finds nothing, so a 2
        // below can only have come from the findings.
        let mut keyword_history = ScoreHistory::new();
        let keyword = run_optimization_pass(&mut manifest, &outputs, &mut keyword_history, None);
        let _ = std::fs::remove_dir_all(&dir);

        findings.unwrap();
        keyword.unwrap();
        assert_eq!(hits(&findings_history, "fallacies"), 2);
        assert_eq!(hits(&findings_history, "nlp_techniques"), 0);
        assert_eq!(hits(&keyword_history, "fallacies"), 0);
    }
}
