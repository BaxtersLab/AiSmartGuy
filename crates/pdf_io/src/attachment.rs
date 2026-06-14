use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::errors::PdfIoError;
use crate::metadata_extract::extract_embedded_file;

/// Embed a file as a PDF attachment (EmbeddedFiles name tree entry),
/// saving the modified document to `output_path`.
pub fn embed_attachment(
    pdf_path: &Path,
    filename: &str,
    data: &[u8],
    output_path: &Path,
) -> Result<(), PdfIoError> {
    let mut doc = Document::load(pdf_path)
        .map_err(|e| PdfIoError::PdfParseError(e.to_string()))?;

    add_embedded_file(&mut doc, filename, data)
        .map_err(|e| PdfIoError::AttachmentError(e.to_string()))?;

    doc.save(output_path)
        .map(|_| ())
        .map_err(|e| PdfIoError::IoError(e.to_string()))?;

    Ok(())
}

/// Extract a named file attachment from a PDF.
/// Returns None if the file is not found.
pub fn extract_attachment(
    pdf_path: &Path,
    filename: &str,
) -> Result<Option<Vec<u8>>, PdfIoError> {
    let doc = Document::load(pdf_path)
        .map_err(|e| PdfIoError::PdfParseError(e.to_string()))?;
    extract_embedded_file(&doc, filename)
}

// ---- internal helpers -------------------------------------------------------

/// lopdf 0.32 notes:
///   - `Document::get_object`     → Result<&Object, Error>
///   - `Document::get_object_mut` → Result<&mut Object, Error>
///   - `Dictionary::get`          → Result<&Object, Error>
///   - `Object::as_dict/as_array/as_reference/…` → Option<…>
///
/// Strategy: convert every Result to Option with `.ok()` before chaining
/// `.and_then()` with Option-returning closures.  For mutable access use
/// `if let Ok(…) = get_object_mut(…)`.
fn add_embedded_file(
    doc: &mut Document,
    filename: &str,
    data: &[u8],
) -> Result<(), String> {
    // 1. Create EmbeddedFile stream
    let mut ef_stream_dict = Dictionary::new();
    ef_stream_dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    let ef_stream = Stream::new(ef_stream_dict, data.to_vec());
    let ef_id = doc.add_object(Object::Stream(ef_stream));

    // 2. Create Filespec dictionary
    let mut ef_ref_dict = Dictionary::new();
    ef_ref_dict.set("F", Object::Reference(ef_id));

    let mut filespec = Dictionary::new();
    filespec.set("Type", Object::Name(b"Filespec".to_vec()));
    filespec.set(
        "F",
        Object::String(filename.as_bytes().to_vec(), lopdf::StringFormat::Literal),
    );
    filespec.set("EF", Object::Dictionary(ef_ref_dict));
    let filespec_id = doc.add_object(Object::Dictionary(filespec));

    // 3. Navigate / create the EmbeddedFiles name tree
    let root_id = get_root_id(doc).ok_or("no catalog root")?;

    // as_dict() and as_reference() return Result in lopdf 0.32 — use .ok().
    // as_array() returns Option — use directly.
    let names_ref_opt: Option<ObjectId> = doc
        .get_object(root_id).ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Names").ok())
        .and_then(|o| o.as_reference().ok());

    if let Some(names_ref) = names_ref_opt {
        // /Names dict exists; check for /EmbeddedFiles
        let ef_ref_opt: Option<ObjectId> = doc
            .get_object(names_ref).ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"EmbeddedFiles").ok())
            .and_then(|o| o.as_reference().ok());

        if let Some(ef_node_ref) = ef_ref_opt {
            // as_array() returns Result in lopdf 0.32 — use .ok() to get Option.
            let mut existing: Vec<Object> = doc
                .get_object(ef_node_ref).ok()
                .and_then(|o| o.as_dict().ok())
                .and_then(|d| d.get(b"Names").ok())
                .and_then(|o| o.as_array().ok())
                .cloned()
                .unwrap_or_default();

            existing.push(Object::String(
                filename.as_bytes().to_vec(),
                lopdf::StringFormat::Literal,
            ));
            existing.push(Object::Reference(filespec_id));

            // as_dict_mut() returns Result in lopdf 0.32 — propagate error.
            let obj = doc.get_object_mut(ef_node_ref)
                .map_err(|_| "cannot get EmbeddedFiles node".to_string())?;
            let dict = obj.as_dict_mut()
                .map_err(|_| "EmbeddedFiles node is not a dictionary".to_string())?;
            dict.set("Names", Object::Array(existing));
        } else {
            // /EmbeddedFiles not present; add it to the existing /Names dict
            let ef_node_id = make_ef_node(doc, filename, filespec_id);
            let obj = doc.get_object_mut(names_ref)
                .map_err(|_| "cannot get Names dict".to_string())?;
            let dict = obj.as_dict_mut()
                .map_err(|_| "Names is not a dictionary".to_string())?;
            dict.set("EmbeddedFiles", Object::Reference(ef_node_id));
        }
    } else {
        // Neither /Names nor /EmbeddedFiles exists; create both from scratch
        let ef_node_id = make_ef_node(doc, filename, filespec_id);

        let mut names_dict = Dictionary::new();
        names_dict.set("EmbeddedFiles", Object::Reference(ef_node_id));
        let names_id = doc.add_object(Object::Dictionary(names_dict));

        let obj = doc.get_object_mut(root_id)
            .map_err(|_| "cannot get catalog root".to_string())?;
        let dict = obj.as_dict_mut()
            .map_err(|_| "catalog root is not a dictionary".to_string())?;
        dict.set("Names", Object::Reference(names_id));
    }

    Ok(())
}

/// Build a simple flat EmbeddedFiles name-tree node containing one entry.
fn make_ef_node(doc: &mut Document, filename: &str, filespec_id: ObjectId) -> ObjectId {
    let mut node = Dictionary::new();
    node.set(
        "Names",
        Object::Array(vec![
            Object::String(
                filename.as_bytes().to_vec(),
                lopdf::StringFormat::Literal,
            ),
            Object::Reference(filespec_id),
        ]),
    );
    doc.add_object(Object::Dictionary(node))
}

/// Get the ObjectId of the document catalog root.
/// `Dictionary::get` returns `Result` in lopdf 0.32, so use `.ok()` first.
fn get_root_id(doc: &Document) -> Option<ObjectId> {
    // as_reference() returns Result in lopdf 0.32 — use .ok() to get Option.
    doc.trailer.get(b"Root").ok().and_then(|o| o.as_reference().ok())
}
