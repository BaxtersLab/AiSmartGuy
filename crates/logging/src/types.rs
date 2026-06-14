use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Debug,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Info    => write!(f, "INFO"),
            LogLevel::Warning => write!(f, "WARNING"),
            LogLevel::Error   => write!(f, "ERROR"),
            LogLevel::Debug   => write!(f, "DEBUG"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub subsystem: String,
    pub level: LogLevel,
    pub message: String,
    pub metadata: Option<HashMap<String, String>>,
}

pub struct Logger {
    pub run_dir: PathBuf,
    pub subsystem: String,
    pub file_path: PathBuf,
}
