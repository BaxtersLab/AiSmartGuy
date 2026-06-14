use std::fmt;
use serde::Serialize;

/// All errors the UI layer can produce.
/// Must be Serialize so they can be returned from Tauri commands.
#[derive(Debug, Clone, Serialize)]
pub enum UiError {
    IoError(String),
    ManifestError(String),
    PdfError(String),
    OrchestratorError(String),
    ConflictError(String),
    ModelFetchError(String),
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::IoError(m) => write!(f, "IoError: {}", m),
            UiError::ManifestError(m) => write!(f, "ManifestError: {}", m),
            UiError::PdfError(m) => write!(f, "PdfError: {}", m),
            UiError::OrchestratorError(m) => write!(f, "OrchestratorError: {}", m),
            UiError::ConflictError(m) => write!(f, "ConflictError: {}", m),
            UiError::ModelFetchError(m) => write!(f, "ModelFetchError: {}", m),
        }
    }
}

impl std::error::Error for UiError {}

pub type UiResult<T> = Result<T, UiError>;
