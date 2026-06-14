use std::fmt;

/// All errors that can occur in the model_loader crate.
#[derive(Debug, Clone)]
pub enum ModelError {
    LoadFailure(String),
    InferenceFailure(String),
    Timeout(String),
    GpuError(String),
    IoError(String),
    InvalidState(String),
    Cancelled(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::LoadFailure(msg) => write!(f, "LoadFailure: {}", msg),
            ModelError::InferenceFailure(msg) => write!(f, "InferenceFailure: {}", msg),
            ModelError::Timeout(msg) => write!(f, "Timeout: {}", msg),
            ModelError::GpuError(msg) => write!(f, "GpuError: {}", msg),
            ModelError::IoError(msg) => write!(f, "IoError: {}", msg),
            ModelError::InvalidState(msg) => write!(f, "InvalidState: {}", msg),
            ModelError::Cancelled(msg) => write!(f, "Cancelled: {}", msg),
        }
    }
}

impl std::error::Error for ModelError {}

pub type LoaderResult<T> = Result<T, ModelError>;
