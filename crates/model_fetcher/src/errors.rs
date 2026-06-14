/// All errors that may arise during model acquisition.
#[derive(Debug)]
pub enum ModelFetchError {
    NetworkError(String),
    RateLimit(String),
    IntegrityError(String),
    MissingShard(String),
    IoError(String),
    HfApiError(String),
    Timeout(String),
    Cancelled(String),
}

impl std::fmt::Display for ModelFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelFetchError::NetworkError(m)   => write!(f, "Network error: {}", m),
            ModelFetchError::RateLimit(m)      => write!(f, "Rate limit: {}", m),
            ModelFetchError::IntegrityError(m) => write!(f, "Integrity error: {}", m),
            ModelFetchError::MissingShard(m)   => write!(f, "Missing shard: {}", m),
            ModelFetchError::IoError(m)        => write!(f, "IO error: {}", m),
            ModelFetchError::HfApiError(m)     => write!(f, "HF API error: {}", m),
            ModelFetchError::Timeout(m)        => write!(f, "Timeout: {}", m),
            ModelFetchError::Cancelled(m)      => write!(f, "Cancelled: {}", m),
        }
    }
}

impl std::error::Error for ModelFetchError {}

/// Convenience Result alias.
pub type FetchResult<T> = Result<T, ModelFetchError>;
