/// Fine-grained error types for the model loader subsystem.
#[derive(Debug, Clone)]
pub enum ModelError {
    LoadFailure(String),
    InferenceFailure(String),
    Timeout(String),
    GpuError(String),
    IoError(String),
    InvalidState(String),
}

#[derive(Debug, Clone)]
pub enum RagError {
    MalformedPacket(String),
    MissingPacket(String),
    ValidationFailure(String),
}

#[derive(Debug, Clone)]
pub enum PdfError {
    ParseFailure(String),
    ExtractionFailure(String),
    WriteFailure(String),
}

#[derive(Debug, Clone)]
pub enum ManifestError {
    Corruption(String),
    ValidationFailure(String),
    VersionMismatch(String),
    IoError(String),
}

#[derive(Debug, Clone)]
pub enum OptimizationError {
    ScoringFailure(String),
    ConsensusFailure(String),
}

/// Universal error type used across all AiSmartGuy crates.
#[derive(Debug, Clone)]
pub enum EngineError {
    ModelError(ModelError),
    RagError(RagError),
    PdfError(PdfError),
    ManifestError(ManifestError),
    OptimizationError(OptimizationError),
    UiError(String),
    IoError(String),
    TimeoutError(String),
    UnknownError(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::ModelError(e)       => write!(f, "ModelError: {:?}", e),
            EngineError::RagError(e)         => write!(f, "RagError: {:?}", e),
            EngineError::PdfError(e)         => write!(f, "PdfError: {:?}", e),
            EngineError::ManifestError(e)    => write!(f, "ManifestError: {:?}", e),
            EngineError::OptimizationError(e)=> write!(f, "OptimizationError: {:?}", e),
            EngineError::UiError(msg)        => write!(f, "UiError: {}", msg),
            EngineError::IoError(msg)        => write!(f, "IoError: {}", msg),
            EngineError::TimeoutError(msg)   => write!(f, "TimeoutError: {}", msg),
            EngineError::UnknownError(msg)   => write!(f, "UnknownError: {}", msg),
        }
    }
}

impl std::error::Error for EngineError {}
