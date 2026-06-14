pub mod errors;
pub mod types;
pub mod classifier;
pub mod recovery;
pub mod gpu_recovery;
pub mod rag_recovery;
pub mod partial_run;
pub mod logger;

pub use errors::{
    EngineError, ManifestError, ModelError, OptimizationError, PdfError, RagError,
};
pub use types::{ErrorClass, RecoveryAction, RecoveryContext};
pub use classifier::classify;
pub use recovery::recover;
pub use gpu_recovery::downgrade_gpu;
pub use rag_recovery::handle_malformed_packet;
pub use partial_run::mark_partial_run;
