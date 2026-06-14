#[derive(Debug)]
pub enum UtilsError {
    IoError(String),
    JsonError(String),
    PathError(String),
    HashError(String),
    TimeError(String),
    OsError(String),
}

impl std::fmt::Display for UtilsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UtilsError::IoError(msg)   => write!(f, "IO error: {}", msg),
            UtilsError::JsonError(msg) => write!(f, "JSON error: {}", msg),
            UtilsError::PathError(msg) => write!(f, "Path error: {}", msg),
            UtilsError::HashError(msg) => write!(f, "Hash error: {}", msg),
            UtilsError::TimeError(msg) => write!(f, "Time error: {}", msg),
            UtilsError::OsError(msg)   => write!(f, "OS error: {}", msg),
        }
    }
}

impl std::error::Error for UtilsError {}
