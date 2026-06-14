use std::path::PathBuf;
use manifest::{load_from_file, validate, ModelConfig};
use crate::errors::{UiError, UiResult};
use crate::types::UiConflict;

/// Load a manifest from disk and return it.
pub fn load_manifest(path: &PathBuf) -> UiResult<manifest::Manifest> {
    load_from_file(path).map_err(|e| UiError::ManifestError(format!("{:?}", e)))
}

/// Validate a manifest and return any structural conflicts.
pub fn validate_manifest(m: &manifest::Manifest) -> UiResult<()> {
    validate(m).map_err(|e| UiError::ManifestError(format!("{:?}", e)))
}

/// Detect manifest-level conflicts and return them as `UiConflict`s.
pub fn detect_manifest_conflicts(m: &manifest::Manifest) -> Vec<UiConflict> {
    let mut conflicts = Vec::new();

    // Collect all configured models (model1/model2/model3/fusion).
    let slots: [Option<&ModelConfig>; 4] = [
        m.models.model1.as_ref(),
        m.models.model2.as_ref(),
        m.models.model3.as_ref(),
        m.models.fusion.as_ref(),
    ];

    for slot in slots.iter().flatten() {
        if slot.path.is_empty() {
            conflicts.push(UiConflict::MissingModel {
                name: slot.name.clone(),
                path: String::from("<not specified>"),
            });
        }
    }

    conflicts
}
