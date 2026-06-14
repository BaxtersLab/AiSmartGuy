use std::fmt;

/// All errors the optimization engine can produce.
#[derive(Debug, Clone)]
pub enum OptimizationError {
    ScoringFailure(String),
    ConsensusFailure(String),
    IoError(String),
    InvalidInput(String),
}

impl fmt::Display for OptimizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptimizationError::ScoringFailure(m) => write!(f, "ScoringFailure: {}", m),
            OptimizationError::ConsensusFailure(m) => write!(f, "ConsensusFailure: {}", m),
            OptimizationError::IoError(m) => write!(f, "IoError: {}", m),
            OptimizationError::InvalidInput(m) => write!(f, "InvalidInput: {}", m),
        }
    }
}

impl std::error::Error for OptimizationError {}

pub type OptimizationResult<T> = Result<T, OptimizationError>;
