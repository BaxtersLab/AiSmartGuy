// AiSmartGuy — optimization crate (Phase 7)

pub mod errors;
pub mod types;
pub mod scoring;
pub mod penalties;
pub mod aggregator;
pub mod consensus;
pub mod mapping;
pub mod state;

pub use errors::{OptimizationError, OptimizationResult};
pub use types::{BookScore, CategoryWinner, ModelCategoryScore, ScoreHistory, WinCounts};
pub use scoring::compute_scores;
pub use penalties::apply_penalties;
pub use aggregator::{aggregate_scores, compute_win_counts};
pub use consensus::determine_winner;
pub use mapping::build_best_model_map;
pub use state::update_optimization_state;
