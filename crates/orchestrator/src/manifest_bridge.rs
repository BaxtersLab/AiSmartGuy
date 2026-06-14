use std::path::PathBuf;
use manifest::Manifest;
use crate::errors::{OrchestratorError, OrchestratorResult};

/// Bridge: load manifest from a JSON file path.
pub fn load_manifest(path: &PathBuf) -> OrchestratorResult<Manifest> {
    manifest::load_from_file(path).map_err(|e| OrchestratorError::ManifestError(format!("{:?}", e)))
}

/// Bridge: write manifest back to its source JSON file.
pub fn save_manifest(manifest: &Manifest, path: &PathBuf) -> OrchestratorResult<()> {
    manifest::write_to_file(manifest, path)
        .map_err(|e| OrchestratorError::ManifestError(format!("{:?}", e)))
}

/// Bridge: validate manifest fields.
pub fn validate_manifest(manifest: &Manifest) -> OrchestratorResult<()> {
    manifest::validate(manifest).map_err(|e| OrchestratorError::ManifestError(format!("{:?}", e)))
}
