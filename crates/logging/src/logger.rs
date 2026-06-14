use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::errors::LogError;
use crate::file_writer::write_line;
use crate::formatter::{format_entry, format_line};
use crate::types::{Logger, LogLevel};
use utils::file::ensure_dir;

/// Create a Logger for the given subsystem, writing to `<run_dir>/logs/<subsystem>.log`.
/// The log directory is created if it does not already exist.
pub fn create_logger(run_dir: &Path, subsystem: &str) -> Logger {
    let log_dir = run_dir.join("logs");
    let _ = ensure_dir(&log_dir);
    let file_path = log_dir.join(format!("{}.log", subsystem.to_lowercase()));
    Logger {
        run_dir: run_dir.to_path_buf(),
        subsystem: subsystem.to_lowercase(),
        file_path,
    }
}

impl Logger {
    pub fn log(
        &self,
        level: LogLevel,
        message: &str,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), LogError> {
        let entry = format_entry(level, &self.subsystem, message, metadata);
        let line = format_line(&entry);
        write_line(&self.file_path, &line)
    }

    /// Log at INFO level. Failures are silent — logging must never abort execution.
    pub fn info(&self, message: &str) {
        let _ = self.log(LogLevel::Info, message, None);
    }

    /// Log at WARNING level.
    pub fn warn(&self, message: &str) {
        let _ = self.log(LogLevel::Warning, message, None);
    }

    /// Log at ERROR level.
    pub fn error(&self, message: &str) {
        let _ = self.log(LogLevel::Error, message, None);
    }

    /// Log at DEBUG level.
    pub fn debug(&self, message: &str) {
        let _ = self.log(LogLevel::Debug, message, None);
    }

    /// Returns the path for metrics.log in the same run directory.
    pub fn metrics_path(&self) -> PathBuf {
        self.run_dir.join("logs").join("metrics.log")
    }
}
