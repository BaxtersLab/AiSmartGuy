use crate::types::{CategoryWinner, ModelCategoryScore, WinCounts};

/// Minimum margin (as fraction of winner's score) required for a clear winner.
const MIN_MARGIN_FRACTION: f32 = 0.05;

/// Minimum win fraction out of STABILITY_WINDOW books for a stable winner.
const MIN_WIN_FRACTION: f32 = 0.70; // 7/10

/// Number of books in the stability window.
const STABILITY_WINDOW: u32 = 10;

/// Tie threshold: if two models are within 1% of each other, it's a tie.
const TIE_THRESHOLD: f32 = 0.01;

/// Determine the winner for a single category from a list of averaged scores.
///
/// Returns `None` when:
/// - fewer than 2 models have scores > 0
/// - the margin is below 5%
/// - the scores are within the 1% tie threshold
///
/// `win_counts`: how many books each (model_name, category) pair won.
/// `books_completed`: total books processed so far.
pub fn determine_winner(
    category: &str,
    scores: &[ModelCategoryScore],
    win_counts: &WinCounts,
    books_completed: u32,
) -> Option<CategoryWinner> {
    if scores.is_empty() {
        return None;
    }

    // Sort descending by score.
    let mut sorted: Vec<&ModelCategoryScore> = scores.iter().collect();
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let best = sorted[0];
    if best.score <= 0.0 {
        return None;
    }

    // Tie check: if second model is within 1% of best → tie, no winner.
    if sorted.len() >= 2 {
        let second = sorted[1];
        let diff = (best.score - second.score).abs();
        if best.score > 0.0 && diff / best.score <= TIE_THRESHOLD {
            return None; // tie — both remain active
        }
    }

    // Margin check: winner must beat second by ≥5%.
    let margin = if sorted.len() >= 2 {
        let second = sorted[1];
        if best.score > 0.0 {
            (best.score - second.score) / best.score
        } else {
            0.0
        }
    } else {
        1.0 // only one model — margin is 100%
    };

    if margin < MIN_MARGIN_FRACTION {
        return None; // ambiguous category
    }

    // Stability check: must have won ≥7/10 books for this category.
    let win_count = *win_counts
        .get(&(best.model_name.clone(), category.to_string()))
        .unwrap_or(&0);

    let required_wins = (MIN_WIN_FRACTION * STABILITY_WINDOW as f32).ceil() as u32;
    let effective_window = books_completed.min(STABILITY_WINDOW);
    let stable = effective_window >= STABILITY_WINDOW && win_count >= required_wins;

    Some(CategoryWinner {
        category: category.to_string(),
        model_name: best.model_name.clone(),
        margin,
        stable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::types::ModelCategoryScore;

    fn mcs(model: &str, score: f32) -> ModelCategoryScore {
        ModelCategoryScore {
            model_name: model.to_string(),
            category: "cat".to_string(),
            score,
            false_positives: 0,
            hits: 1,
        }
    }

    fn wins(model: &str, cat: &str, n: u32) -> WinCounts {
        let mut m = HashMap::new();
        m.insert((model.to_string(), cat.to_string()), n);
        m
    }

    #[test]
    fn test_clear_winner() {
        let scores = vec![mcs("m1", 2.0), mcs("m2", 1.0)];
        let wc = wins("m1", "cat", 8);
        let result = determine_winner("cat", &scores, &wc, 10);
        let w = result.unwrap();
        assert_eq!(w.model_name, "m1");
        assert!(w.stable);
    }

    #[test]
    fn test_margin_below_threshold_returns_none() {
        // margin = (2.0 - 1.97) / 2.0 = 0.015 < 0.05
        let scores = vec![mcs("m1", 2.0), mcs("m2", 1.97)];
        let wc = wins("m1", "cat", 8);
        let result = determine_winner("cat", &scores, &wc, 10);
        assert!(result.is_none());
    }

    #[test]
    fn test_tie_returns_none() {
        let scores = vec![mcs("m1", 1.0), mcs("m2", 1.005)];
        let wc = wins("m1", "cat", 8);
        let result = determine_winner("cat", &scores, &wc, 10);
        assert!(result.is_none());
    }

    #[test]
    fn test_unstable_winner() {
        let scores = vec![mcs("m1", 2.0), mcs("m2", 1.0)];
        let wc = wins("m1", "cat", 5); // only 5 wins out of 10
        let result = determine_winner("cat", &scores, &wc, 10);
        let w = result.unwrap();
        assert!(!w.stable);
    }
}
