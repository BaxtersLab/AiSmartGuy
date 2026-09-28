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

/// What structured-findings scoring needs: the rule → category map, every
/// output with the chapter it analysed, and each chapter's own text. A finding
/// scores only when its quote is in the chapter it cites (ASG-Q3).
pub struct FindingsScoring<'a> {
    pub rule_category: &'a HashMap<String, String>,
    /// model → (0-based chapter index, output path), as the run produced them.
    pub outputs: &'a HashMap<String, Vec<(usize, PathBuf)>>,
    /// Each chapter's own text (`pdf_io::Chapter::body`), by chapter index.
    pub sources: &'a [&'a str],
}

/// Bridge: score model outputs against hitlist categories, then feed results
/// to the optimization engine.
///
/// `model_output_paths` maps model names to their ordered output file paths.
/// Reads the output texts, computes per-category scores, and updates
/// `manifest.optimization_state` through the optimisation lifecycle.
/// `findings`: when `Some` (structured-findings mode), per-category hit counts
/// are derived from the parsed findings whose quotes verified, via the
/// `rule_id → category` map, and scored with `compute_scores_from_counts` —
/// replacing the keyword-mention heuristic. `None` = the keyword path (default).
pub fn run_optimization_pass(
    manifest: &mut Manifest,
    model_output_paths: &HashMap<String, Vec<PathBuf>>,
    history: &mut ScoreHistory,
    findings: Option<&FindingsScoring<'_>>,
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
    let book_score: BookScore = if let Some(fs) = findings {
        let mut model_counts: HashMap<String, HashMap<String, u32>> = HashMap::new();
        for (model_name, list) in fs.outputs {
            // The chapter each output analysed, never its position in the
            // list: with several RAG passes, position 2 can be chapter 1.
            let outputs: Vec<(usize, String)> = list
                .iter()
                .map(|(ch, p)| (*ch, std::fs::read_to_string(p).unwrap_or_default()))
                .collect();
            let merged = crate::findings::merge_findings(&outputs, fs.sources);
            model_counts.insert(
                model_name.clone(),
                merged.aggregate.per_category_counts(fs.rule_category),
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
        {"rule_id":"FAL-01","quote":"here is the first claim","location":"p1","severity":"high","note":"n"},
        {"rule_id":"FAL-02","quote":"then the second claim","location":"p2","severity":"low","note":"n"},
        {"rule_id":"ZZZ-99","quote":"last, the third claim","location":"p3","severity":"low","note":"n"}
    ]"#;
    /// The chapter those quotes come from, and one they do not.
    const CHAPTER: &str = "Here is the first claim. Then the second claim. Last, the third claim.";
    const OTHER: &str = "A chapter about something else entirely.";

    /// Score FINDINGS_OUTPUT as the analysis of chapter `ch`, against `sources`.
    fn score(ch: usize, sources: &[&str], tag: &str) -> (ScoreHistory, ScoreHistory) {
        let dir = std::env::temp_dir().join(format!("asg_optbridge_{}_{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("chapter_001_output.txt");
        std::fs::write(&out, FINDINGS_OUTPUT).unwrap();
        let mut paths = HashMap::new();
        paths.insert("model1".to_string(), vec![out.clone()]);
        let mut by_chapter = HashMap::new();
        by_chapter.insert("model1".to_string(), vec![(ch, out.clone())]);

        let mut rule_category = HashMap::new();
        rule_category.insert("FAL-01".to_string(), "fallacies".to_string());
        rule_category.insert("FAL-02".to_string(), "fallacies".to_string());

        let mut manifest = manifest::default_manifest();
        manifest.categories_active = vec!["fallacies".to_string(), "nlp_techniques".to_string()];

        let scoring = FindingsScoring { rule_category: &rule_category, outputs: &by_chapter, sources };
        let mut findings_history = ScoreHistory::new();
        let findings = run_optimization_pass(
            &mut manifest.clone(), &paths, &mut findings_history, Some(&scoring));
        let mut keyword_history = ScoreHistory::new();
        let keyword = run_optimization_pass(&mut manifest, &paths, &mut keyword_history, None);
        let _ = std::fs::remove_dir_all(&dir);
        findings.unwrap();
        keyword.unwrap();
        (findings_history, keyword_history)
    }

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
        let (findings, keyword) = score(0, &[CHAPTER], "real");
        assert_eq!(hits(&findings, "fallacies"), 2);
        assert_eq!(hits(&findings, "nlp_techniques"), 0);
        // Control: the same output on the keyword path finds nothing, so the
        // 2 above can only have come from the findings.
        assert_eq!(hits(&keyword, "fallacies"), 0);
    }

    /// ASG-Q3: a finding whose quote is not in the chapter it cites scores
    /// nothing, whether the book lacks it or it sits in another chapter.
    #[test]
    fn findings_with_unverified_quotes_score_nothing() {
        let (not_in_book, _) = score(0, &[OTHER], "absent");
        assert_eq!(hits(&not_in_book, "fallacies"), 0);
        let (wrong_chapter, _) = score(1, &[CHAPTER, OTHER], "elsewhere");
        assert_eq!(hits(&wrong_chapter, "fallacies"), 0, "the quotes are in chapter 1, cited as 2");
        let (right_chapter, _) = score(0, &[CHAPTER, OTHER], "cited");
        assert_eq!(hits(&right_chapter, "fallacies"), 2, "control: cited correctly, they score");
    }
}
