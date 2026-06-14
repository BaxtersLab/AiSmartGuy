use std::fmt;
use model_fetcher::ModelFetchError;

/// All errors the orchestrator can produce.
#[derive(Debug, Clone)]
pub enum OrchestratorError {
    ManifestError(String),
    PdfError(String),
    RagError(String),
    ModelFetchFailed(String),
    ModelLoadFailed(String),
    InferenceFailed(String),
    FusionFailed(String),
    OptimizationError(String),
    IoError(String),
    InvalidState(String),
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrchestratorError::ManifestError(m) => write!(f, "ManifestError: {}", m),
            OrchestratorError::PdfError(m) => write!(f, "PdfError: {}", m),
            OrchestratorError::RagError(m) => write!(f, "RagError: {}", m),
            OrchestratorError::ModelFetchFailed(m) => write!(f, "ModelFetchFailed: {}", m),
            OrchestratorError::ModelLoadFailed(m) => write!(f, "ModelLoadFailed: {}", m),
            OrchestratorError::InferenceFailed(m) => write!(f, "InferenceFailed: {}", m),
            OrchestratorError::FusionFailed(m) => write!(f, "FusionFailed: {}", m),
            OrchestratorError::OptimizationError(m) => write!(f, "OptimizationError: {}", m),
            OrchestratorError::IoError(m) => write!(f, "IoError: {}", m),
            OrchestratorError::InvalidState(m) => write!(f, "InvalidState: {}", m),
        }
    }
}

impl std::error::Error for OrchestratorError {}

/// Map a model_fetcher error into an orchestrator error.
pub fn map_fetch_error(err: ModelFetchError) -> OrchestratorError {
    OrchestratorError::ModelFetchFailed(err.to_string())
}

pub type OrchestratorResult<T> = Result<T, OrchestratorError>;
