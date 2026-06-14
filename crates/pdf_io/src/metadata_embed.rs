use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::errors::PdfIoError;

/// Embed the manifest JSON into the PDF's Info dictionary (as Base64),
/// writing the result to `output_path`.
pub fn embed_manifest(
    pdf_path: &Path,
    manifest_json: &str,
    output_path: &Path,
) -> Result<(), PdfIoError> {
    let mut doc = Document::load(pdf_path)
        .map_err(|e| PdfIoError::PdfParseError(e.to_string()))?;

    let base64_manifest = STANDARD.encode(manifest_json.as_bytes());

    set_info_key(&mut doc, b"AiSmartGuyManifest", &base64_manifest)
        .map_err(|e| PdfIoError::MetadataError(e.to_string()))?;

    doc.save(output_path)
        .map(|_| ())
        .map_err(|e| PdfIoError::IoError(e.to_string()))?;

    Ok(())
}

/// Set a custom key in the document's Info dictionary.
/// Creates the Info dictionary if it does not already exist.
/// Key insight for lopdf 0.32: `get_object_mut` returns `Result`, not `Option`.
pub(crate) fn set_info_key(
    doc: &mut Document,
    key: &[u8],
    value: &str,
) -> Result<(), String> {
    let value_obj = Object::String(
        value.as_bytes().to_vec(),
        lopdf::StringFormat::Literal,
    );

    // `Dictionary::get` and `as_reference` both return Result in lopdf 0.32.
    // Use .ok() to convert Result -> Option for chaining.
    let info_id_opt: Option<ObjectId> = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| o.as_reference().ok());

    if let Some(info_id) = info_id_opt {
        // `get_object_mut` and `as_dict_mut` both return Result in lopdf 0.32.
        if let Ok(obj) = doc.get_object_mut(info_id) {
            if let Ok(dict) = obj.as_dict_mut() {
                dict.set(key.to_vec(), value_obj);
                return Ok(());
            }
        }
    }

    // Create a new Info dictionary.
    let mut info_dict = Dictionary::new();
    info_dict.set(key.to_vec(), value_obj);
    let new_id = doc.add_object(Object::Dictionary(info_dict));
    doc.trailer.set(b"Info".to_vec(), Object::Reference(new_id));

    Ok(())
}
