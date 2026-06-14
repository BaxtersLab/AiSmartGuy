use std::path::Path;

use error_system::ManifestError;
use utils::file::atomic_write;

use crate::schema::Manifest;

pub fn serialize(manifest: &Manifest) -> Result<String, ManifestError> {
    serde_json::to_string_pretty(manifest)
        .map_err(|e| ManifestError::ValidationFailure(e.to_string()))
}

pub fn write_to_file(manifest: &Manifest, path: &Path) -> Result<(), ManifestError> {
    let json = serialize(manifest)?;
    atomic_write(path, &json)
        .map_err(|e| ManifestError::IoError(e.to_string()))
}
