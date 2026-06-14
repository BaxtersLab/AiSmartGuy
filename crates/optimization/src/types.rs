use std::collections::HashMap;

/// Score for one model on one category for a single book.
#[derive(Debug, Clone)]
pub struct ModelCategoryScore {
    pub model_name: String,
    pub category: String,
    /// Raw score before penalties.
    pub score: f32,
    pub false_positives: u32,
    pub hits: u32,
}

/// All model-category scores collected for a single book pass.
#[derive(Debug, Clone, Default)]
pub struct BookScore {
    pub model_scores: Vec<ModelCategoryScore>,
}

/// Result of consensus analysis for one category.
#[derive(Debug, Clone)]
pub struct CategoryWinner {
    pub category: String,
    pub model_name: String,
    /// Score margin over the second-best model (0.0–1.0 fraction of winner score).
    pub margin: f32,
    /// True if this model won ≥7 out of the last 10 books for this category.
    pub stable: bool,
}

/// Accumulated per-book history used by the aggregator.
pub type ScoreHistory = Vec<BookScore>;

/// Win counts per (model, category) pair — used for stability check.
pub type WinCounts = HashMap<(String, String), u32>;
