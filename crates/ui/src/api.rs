use std::path::{Path, PathBuf};
use crate::state::{new_shared_state, SharedUiState};

/// Application configuration, supplied at startup.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Directory containing all model caches.
    pub model_cache_dir: PathBuf,
    /// Directory where run outputs are written.
    pub run_output_dir: PathBuf,
    /// Path to the active manifest file.
    pub manifest_path: PathBuf,
}

impl AppConfig {
    pub fn new(
        model_cache_dir: impl AsRef<Path>,
        run_output_dir: impl AsRef<Path>,
        manifest_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            model_cache_dir: model_cache_dir.as_ref().to_path_buf(),
            run_output_dir: run_output_dir.as_ref().to_path_buf(),
            manifest_path: manifest_path.as_ref().to_path_buf(),
        }
    }
}

/// Create and return a fresh shared state handle bound to an `AppConfig`.
pub fn setup_app(config: &AppConfig) -> SharedUiState {
    let _ = config; // Config used at command call sites; state is independent.
    new_shared_state()
}

/// Build the run directory for a given run ID (e.g. timestamp string).
pub fn build_run_dir(config: &AppConfig, run_id: &str) -> PathBuf {
    config.run_output_dir.join(run_id)
}
