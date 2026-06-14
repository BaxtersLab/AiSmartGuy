// AiSmartGuy — orchestrator crate (Phase 6)

pub mod errors;
pub mod types;
pub mod progress;
pub mod run_modes;
pub mod sequence_plan;
pub mod manifest_bridge;
pub mod rag_bridge;
pub mod pdf_bridge;
pub mod state_bridge;
pub mod bridge_model_fetcher;
pub mod fusion;
pub mod optimization_bridge;
pub mod orchestrator;

pub use errors::{OrchestratorError, OrchestratorResult};
pub use types::{FusionInput, ModelOutputs, OrchestratorProgressEvent, OrchestratorState, RunContext, SequencePlan};
pub use progress::{set_progress_callback, clear_progress_callback};
pub use orchestrator::Orchestrator;
