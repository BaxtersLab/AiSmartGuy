#[derive(Debug)]
pub enum LogError {
    IoError(String),
    FormatError(String),
    SerializationError(String),
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::IoError(msg)           => write!(f, "Log IO error: {}", msg),
            LogError::FormatError(msg)       => write!(f, "Log format error: {}", msg),
            LogError::SerializationError(msg) => write!(f, "Log serialization error: {}", msg),
        }
    }
}

impl std::error::Error for LogError {}
