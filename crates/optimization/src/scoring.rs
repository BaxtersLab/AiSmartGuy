use std::collections::HashMap;
use crate::types::{BookScore, ModelCategoryScore};

/// Compute per-category scores for each model from raw output texts.
///
/// The scoring formula (per hit):
///   hit_score = confidence × severity_weight × ln(1 + match_count)
///
/// Since we don't have structured output at this layer, we use a simple
/// heuristic: count how many times the category keyword appears in each
/// model's output as the `match_count`, with fixed confidence=1.0 and
/// severity_weight=1.0. Callers with richer output can supply pre-parsed counts.
///
/// `model_outputs`: model_name → list of chunk output texts (raw strings)
/// `categories`:    list of category names to score
pub fn compute_scores(
    model_outputs: &HashMap<String, Vec<String>>,
    categories: &[String],
) -> BookScore {
    let mut model_scores: Vec<ModelCategoryScore> = Vec::new();

    for (model_name, outputs) in model_outputs {
        for category in categories {
            let match_count: u32 = outputs.iter().map(|text| count_mentions(text, category)).sum();
            let score = score_formula(1.0, 1.0, match_count);

            model_scores.push(ModelCategoryScore {
                model_name: model_name.clone(),
                category: category.clone(),
                score,
                false_positives: 0, // populated externally by caller if available
                hits: match_count,
            });
        }
    }

    BookScore { model_scores }
}

/// Count non-overlapping case-insensitive occurrences of `keyword` in `text`.
fn count_mentions(text: &str, keyword: &str) -> u32 {
    if keyword.is_empty() {
        return 0;
    }
    let text_lower = text.to_lowercase();
    let kw_lower = keyword.to_lowercase();
    let mut count = 0u32;
    let mut start = 0;
    while let Some(pos) = text_lower[start..].find(&kw_lower) {
        count += 1;
        start += pos + kw_lower.len();
    }
    count
}

/// hit_score = confidence × severity_weight × ln(1 + match_count)
fn score_formula(confidence: f32, severity_weight: f32, match_count: u32) -> f32 {
    confidence * severity_weight * (1.0 + match_count as f32).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_zero_matches() {
        let score = score_formula(1.0, 1.0, 0);
        // ln(1) = 0
        assert!((score - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_score_one_match() {
        let score = score_formula(1.0, 1.0, 1);
        // ln(2) ≈ 0.693
        assert!((score - 2f32.ln()).abs() < 1e-6);
    }

    #[test]
    fn test_count_mentions_case_insensitive() {
        assert_eq!(count_mentions("Fallacy fallacy FALLACY", "fallacy"), 3);
    }

    #[test]
    fn test_compute_scores_produces_entry_per_category() {
        let mut outputs = HashMap::new();
        outputs.insert("model1".to_string(), vec!["fallacy here".to_string()]);
        let cats = vec!["fallacy".to_string(), "tone".to_string()];
        let book = compute_scores(&outputs, &cats);
        assert_eq!(book.model_scores.len(), 2);
    }
}
