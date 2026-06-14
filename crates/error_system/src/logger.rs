use std::path::Path;

use crate::errors::EngineError;
use crate::types::{ErrorClass, RecoveryAction};
use logging::file_writer::write_line;
use utils::time::now_iso8601;

/// Append a structured error record to the engine error log file.
/// Failures are silently ignored — the error logger must never abort execution.
pub fn log_error(
    error_log_path: &Path,
    error: &EngineError,
    class: &ErrorClass,
    action: &RecoveryAction,
) {
    let line = format!(
        "[{}]\nERROR_CLASS: {:?}\nERROR_TYPE: {}\nACTION: {:?}\n",
        now_iso8601(),
        class,
        error,
        action
    );
    let _ = write_line(error_log_path, &line);
}
