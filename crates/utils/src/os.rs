use std::process::{Child, Command};

use crate::errors::UtilsError;

/// Returns "windows", "macos", or "linux" depending on compile target.
pub fn detect_os() -> String {
    if cfg!(target_os = "windows") {
        "windows".to_string()
    } else if cfg!(target_os = "macos") {
        "macos".to_string()
    } else {
        "linux".to_string()
    }
}

/// Spawn a subprocess. Returns the child handle or an error.
/// Caller is responsible for waiting on or killing the child.
pub fn spawn_process(cmd: &mut Command) -> Result<Child, UtilsError> {
    cmd.spawn()
        .map_err(|e| UtilsError::OsError(e.to_string()))
}

/// Kill a running child process.
pub fn kill_process(child: &mut Child) -> Result<(), UtilsError> {
    child
        .kill()
        .map_err(|e| UtilsError::OsError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_os_returns_known_value() {
        let os = detect_os();
        assert!(
            os == "windows" || os == "macos" || os == "linux",
            "unexpected OS: {}",
            os
        );
    }
}
