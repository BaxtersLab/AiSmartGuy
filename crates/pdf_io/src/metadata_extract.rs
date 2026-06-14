use std::path::Path;

use lopdf::{Document, Object};

use crate::errors::PdfIoError;
use crate::types::PdfMetadata;

/// Attempt to extract AiSmartGuy manifest metadata from a PDF.
///
/// Tries, in order:
/// 1. Info dictionary key `AiSmartGuyManifest` (Base64 JSON)
/// 2. Embedded file attachment named `aismartguy_manifest.json`
pub fn extract_manifest(path: &Path) -> Result<PdfMetadata, PdfIoError> {
    let doc = Document::load(path)
        .map_err(|e| PdfIoError::PdfParseError(e.to_string()))?;

    let manifest_base64 = read_info_key(&doc, b"AiSmartGuyManifest");
    let attachment_manifest = read_attachment_text(&doc, "aismartguy_manifest.json");

    Ok(PdfMetadata {
        manifest_base64,
        attachment_manifest,
    })
}

/// Read a string value from the document's Info dictionary by key.
/// `Dictionary::get` returns `Result` in lopdf 0.32 — use `.ok()` to convert.
pub(crate) fn read_info_key(doc: &Document, key: &[u8]) -> Option<String> {
    // All get/as_* methods return Result in lopdf 0.32; use .ok() to get Option.
    let info_id = doc.trailer.get(b"Info").ok()?.as_reference().ok()?;
    let info_obj = doc.get_object(info_id).ok()?;
    let info_dict = info_obj.as_dict().ok()?;
    let value = info_dict.get(key).ok()?;
    match value {
        Object::String(bytes, _) => String::from_utf8(bytes.clone()).ok(),
        _ => None,
    }
}

/// Read the content of an embedded file attachment by filename.
fn read_attachment_text(doc: &Document, filename: &str) -> Option<String> {
    let bytes = extract_embedded_file(doc, filename).ok()??;
    String::from_utf8(bytes).ok()
}

/// Navigate the EmbeddedFiles name tree and return the raw bytes of the
/// named attachment, or None if not found.
/// lopdf 0.32: `get_object` returns `Result` — call `.ok()` to get `Option`.
pub(crate) fn extract_embedded_file(
    doc: &Document,
    filename: &str,
) -> Result<Option<Vec<u8>>, PdfIoError> {
    // Catalog → /Names → /EmbeddedFiles → /Names array
    // as_reference() and as_dict() return Result in lopdf 0.32 — use .ok().
    let root_id = match doc.trailer.get(b"Root").ok().and_then(|o| o.as_reference().ok()) {
        Some(id) => id,
        None => return Ok(None),
    };

    let names_id = match doc.get_object(root_id).ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Names").ok())
        .and_then(|o| o.as_reference().ok())
    {
        Some(id) => id,
        None => return Ok(None),
    };

    let ef_id = match doc.get_object(names_id).ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"EmbeddedFiles").ok())
        .and_then(|o| o.as_reference().ok())
    {
        Some(id) => id,
        None => return Ok(None),
    };

    // as_array() returns Result in lopdf 0.32 — use .ok() to get Option.
    let names_array: Vec<Object> = match doc.get_object(ef_id).ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Names").ok())
        .and_then(|o| o.as_array().ok())
    {
        Some(a) => a.clone(),
        None => return Ok(None),
    };

    // Array is alternating: [name, filespec_ref, name, filespec_ref, ...]
    let mut i = 0;
    while i + 1 < names_array.len() {
        let name_matches = match &names_array[i] {
            Object::String(bytes, _) => bytes.as_slice() == filename.as_bytes(),
            _ => false,
        };

        if name_matches {
            // as_reference() returns Result in lopdf 0.32 — match Ok/Err.
            let filespec_id = match names_array[i + 1].as_reference() {
                Ok(id) => id,
                Err(_) => { i += 2; continue; }
            };

            // get_object returns Result — use .ok() to convert to Option,
            // then chain with Option-returning as_dict / as_reference.
            let stream_id = doc.get_object(filespec_id).ok()
                .and_then(|o| o.as_dict().ok())
                .and_then(|d| d.get(b"EF").ok())
                .and_then(|o| o.as_dict().ok())
                .and_then(|d| d.get(b"F").ok())
                .and_then(|o| o.as_reference().ok());

            if let Some(sid) = stream_id {
                // as_stream() returns Result in lopdf 0.32 — use .ok()
                let bytes = doc.get_object(sid).ok()
                    .and_then(|o| o.as_stream().ok())
                    .map(|s| s.content.clone());
                return Ok(bytes);
            }
        }
        i += 2;
    }

    Ok(None)
}
