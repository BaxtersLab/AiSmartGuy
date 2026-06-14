use std::collections::HashMap;
use crate::types::{BookScore, ModelCategoryScore, WinCounts};

/// Aggregate per-book scores across a history of books.
///
/// Returns a map from category → list of per-model scores (averaged across books).
pub fn aggregate_scores(history: &[BookScore]) -> HashMap<String, Vec<ModelCategoryScore>> {
    if history.is_empty() {
        return HashMap::new();
    }

    // Accumulate: (model, category) → (sum_score, sum_fp, sum_hits, count)
    let mut acc: HashMap<(String, String), (f32, u32, u32, u32)> = HashMap::new();

    for book in history {
        for s in &book.model_scores {
            let entry = acc
                .entry((s.model_name.clone(), s.category.clone()))
                .or_insert((0.0, 0, 0, 0));
            entry.0 += s.score;
            entry.1 += s.false_positives;
            entry.2 += s.hits;
            entry.3 += 1;
        }
    }

    // Build averaged ModelCategoryScore entries, grouped by category.
    let mut result: HashMap<String, Vec<ModelCategoryScore>> = HashMap::new();
    for ((model_name, category), (sum_score, sum_fp, sum_hits, count)) in acc {
        let avg = ModelCategoryScore {
            model_name: model_name.clone(),
            category: category.clone(),
            score: sum_score / count as f32,
            false_positives: sum_fp / count.max(1),
            hits: sum_hits / count.max(1),
        };
        result.entry(category).or_default().push(avg);
    }

    result
}

/// Count how many books (out of `history`) each (model, category) pair "won"
/// (had the highest post-penalty score for that category in that book).
pub fn compute_win_counts(history: &[BookScore]) -> WinCounts {
    let mut win_counts: WinCounts = HashMap::new();

    for book in history {
        // Group this book's scores by category.
        let mut by_cat: HashMap<String, Vec<&ModelCategoryScore>> = HashMap::new();
        for s in &book.model_scores {
            by_cat.entry(s.category.clone()).or_default().push(s);
        }
        // For each category, find the model with the highest score.
        for (cat, scores) in by_cat {
            if let Some(winner) = scores.iter().max_by(|a, b| {
                a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal)
            }) {
                *win_counts
                    .entry((winner.model_name.clone(), cat))
                    .or_insert(0) += 1;
            }
        }
    }

    win_counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelCategoryScore;

    fn mcs(model: &str, cat: &str, score: f32) -> ModelCategoryScore {
        ModelCategoryScore {
            model_name: model.to_string(),
            category: cat.to_string(),
            score,
            false_positives: 0,
            hits: 1,
        }
    }

    #[test]
    fn test_aggregate_averages_scores() {
        let book1 = BookScore { model_scores: vec![mcs("m1", "cat", 2.0)] };
        let book2 = BookScore { model_scores: vec![mcs("m1", "cat", 4.0)] };
        let result = aggregate_scores(&[book1, book2]);
        let scores = result.get("cat").unwrap();
        assert_eq!(scores.len(), 1);
        assert!((scores[0].score - 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_win_counts() {
        let book = BookScore {
            model_scores: vec![mcs("m1", "cat", 3.0), mcs("m2", "cat", 2.0)],
        };
        let wins = compute_win_counts(&[book]);
        assert_eq!(*wins.get(&("m1".to_string(), "cat".to_string())).unwrap_or(&0), 1);
        assert_eq!(*wins.get(&("m2".to_string(), "cat".to_string())).unwrap_or(&0), 0);
    }
}
