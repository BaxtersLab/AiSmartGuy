use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::errors::LogError;

/// Append a single line (with trailing newline) to a log file.
/// The file is created if it does not exist.
pub fn write_line(path: &Path, line: &str) -> Result<(), LogError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| LogError::IoError(e.to_string()))?;
    writeln!(file, "{}", line)
        .map_err(|e| LogError::IoError(e.to_string()))?;
    file.flush()
        .map_err(|e| LogError::IoError(e.to_string()))
}
