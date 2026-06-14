use crate::types::ModelCategoryScore;

/// Penalty formula: penalty = 0.1 × false_positive_count
/// Cap: max penalty = 30% of original score.
///
/// Modifies `score` in-place.
pub fn apply_penalties(score: &mut ModelCategoryScore) {
    if score.false_positives == 0 {
        return;
    }
    let raw_penalty = 0.1 * score.false_positives as f32;
    let max_penalty = 0.30 * score.score;
    let capped_penalty = raw_penalty.min(max_penalty);
    score.score = (score.score - capped_penalty).max(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_score(score: f32, fp: u32) -> ModelCategoryScore {
        ModelCategoryScore {
            model_name: "m".to_string(),
            category: "c".to_string(),
            score,
            false_positives: fp,
            hits: 1,
        }
    }

    #[test]
    fn test_no_penalty_when_no_fp() {
        let mut s = make_score(1.0, 0);
        apply_penalties(&mut s);
        assert!((s.score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_penalty_applied() {
        // raw_penalty = 0.1 * 2 = 0.2, max_penalty = 0.3 * 1.0 = 0.3 → penalty = 0.2
        let mut s = make_score(1.0, 2);
        apply_penalties(&mut s);
        assert!((s.score - 0.8).abs() < 1e-5);
    }

    #[test]
    fn test_penalty_capped_at_30_percent() {
        // raw_penalty = 0.1 * 10 = 1.0, max_penalty = 0.3 * 1.0 = 0.3 → capped at 0.3
        let mut s = make_score(1.0, 10);
        apply_penalties(&mut s);
        assert!((s.score - 0.7).abs() < 1e-5);
    }

    #[test]
    fn test_score_never_negative() {
        let mut s = make_score(0.05, 10);
        apply_penalties(&mut s);
        assert!(s.score >= 0.0);
    }
}
