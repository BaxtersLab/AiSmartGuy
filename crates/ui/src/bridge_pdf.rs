use std::path::Path;
use base64::Engine;
use pdf_io::extract_manifest as pdf_extract_manifest;
use manifest::deserialize;
use crate::errors::{UiError, UiResult};

/// Extract and decode a manifest embedded in a PDF.
///
/// Returns `Ok(Some(manifest))` if a manifest is embedded,
/// `Ok(None)` if none is found, or `Err` if extraction/decoding fails.
pub fn extract_manifest_from_pdf(path: &Path) -> UiResult<Option<manifest::Manifest>> {
    let metadata = pdf_extract_manifest(path)
        .map_err(|e| UiError::PdfError(format!("{:?}", e)))?;

    // Prefer the Base64-encoded Info dictionary entry.
    if let Some(b64) = metadata.manifest_base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| UiError::ManifestError(format!("base64 decode: {}", e)))?;
        let json = String::from_utf8(bytes)
            .map_err(|e| UiError::ManifestError(format!("utf8 decode: {}", e)))?;
        let manifest = deserialize(&json)
            .map_err(|e| UiError::ManifestError(format!("{:?}", e)))?;
        return Ok(Some(manifest));
    }

    // Fall back to the attached JSON file.
    if let Some(json) = metadata.attachment_manifest {
        let manifest = deserialize(&json)
            .map_err(|e| UiError::ManifestError(format!("{:?}", e)))?;
        return Ok(Some(manifest));
    }

    Ok(None)
}
