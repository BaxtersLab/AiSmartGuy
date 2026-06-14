use std::collections::HashMap;
use crate::types::CategoryWinner;

/// Build the best-model-per-category map from stable winners.
///
/// Only stable winners (where `winner.stable == true`) are included.
/// Ambiguous or unstable categories are omitted — they remain unoptimized.
pub fn build_best_model_map(winners: &[CategoryWinner]) -> HashMap<String, String> {
    winners
        .iter()
        .filter(|w| w.stable)
        .map(|w| (w.category.clone(), w.model_name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CategoryWinner;

    #[test]
    fn test_only_stable_winners_included() {
        let winners = vec![
            CategoryWinner { category: "cat1".to_string(), model_name: "m1".to_string(), margin: 0.2, stable: true },
            CategoryWinner { category: "cat2".to_string(), model_name: "m2".to_string(), margin: 0.1, stable: false },
        ];
        let map = build_best_model_map(&winners);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("cat1").unwrap(), "m1");
        assert!(!map.contains_key("cat2"));
    }

    #[test]
    fn test_empty_winners() {
        let map = build_best_model_map(&[]);
        assert!(map.is_empty());
    }
}
