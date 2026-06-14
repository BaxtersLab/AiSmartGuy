use std::path::Path;

use error_system::ManifestError;
use utils::file::read_to_string;
// (read_to_string used in load_from_file below)

use crate::schema::Manifest;

pub fn deserialize(json: &str) -> Result<Manifest, ManifestError> {
    serde_json::from_str(json)
        .map_err(|e| ManifestError::ValidationFailure(e.to_string()))
}

pub fn load_from_file(path: &Path) -> Result<Manifest, ManifestError> {
    let json = read_to_string(path)
        .map_err(|e| ManifestError::IoError(e.to_string()))?;
    deserialize(&json)
}
