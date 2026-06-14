use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream};

use crate::errors::PdfIoError;
use crate::metadata_embed::set_info_key;

/// Write the final output PDF as a standalone report (no original book pages),
/// with the manifest embedded in the Info dictionary.
pub fn write_final_pdf(
    _original_pdf: &Path,
    results_text: &str,
    manifest_json: &str,
    output_path: &Path,
) -> Result<(), PdfIoError> {
    let mut doc = Document::with_version("1.5");

    // Built-in Helvetica font — no embedding required
    let font_id = doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Font".to_vec()));
        d.set("Subtype", Object::Name(b"Type1".to_vec()));
        d.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        d
    }));

    let resources_id = doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        let mut font_sub = Dictionary::new();
        font_sub.set("F1", Object::Reference(font_id));
        d.set("Font", Object::Dictionary(font_sub));
        d
    }));

    // Create Pages node with empty Kids (will update after building pages)
    let pages_id = doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Pages".to_vec()));
        d.set("Count", Object::Integer(0));
        d.set("Kids", Object::Array(vec![]));
        d
    }));

    // Split the report into pages of ~55 lines each
    let title = "AiSmartGuy \u{2014} Analysis Results";
    let lines: Vec<&str> = results_text.lines().collect();
    let lines_per_page: usize = 55;

    let mut page_ids = Vec::new();

    if lines.is_empty() {
        // At least one page even when results are empty
        let content_bytes = build_text_content(title, "")?;
        let content_id = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            content_bytes,
        )));
        let page_id = doc.add_object(make_page(pages_id, resources_id, content_id));
        page_ids.push(page_id);
    } else {
        for (i, chunk) in lines.chunks(lines_per_page).enumerate() {
            let cont_title = format!("{} (continued)", title);
            let page_title = if i == 0 { title } else { &cont_title };
            let page_body = chunk.join("\n");
            let content_bytes = build_text_content(page_title, &page_body)?;
            let content_id = doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                content_bytes,
            )));
            let page_id = doc.add_object(make_page(pages_id, resources_id, content_id));
            page_ids.push(page_id);
        }
    }

    // Update Pages node with the real Kids array and Count
    let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
    let pages_obj = doc.get_object_mut(pages_id)
        .map_err(|_| PdfIoError::PdfParseError("cannot get Pages object".to_string()))?;
    let pages_dict = pages_obj.as_dict_mut()
        .map_err(|_| PdfIoError::PdfParseError("Pages is not a dictionary".to_string()))?;
    pages_dict.set("Kids", Object::Array(kids));
    pages_dict.set("Count", Object::Integer(page_ids.len() as i64));

    // Catalog
    let catalog_id = doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Catalog".to_vec()));
        d.set("Pages", Object::Reference(pages_id));
        d
    }));
    doc.trailer.set("Root", Object::Reference(catalog_id));

    // Embed manifest as base64 in the Info dictionary
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let b64 = STANDARD.encode(manifest_json.as_bytes());
    set_info_key(&mut doc, b"AiSmartGuyManifest", &b64)
        .map_err(|e| PdfIoError::MetadataError(e.to_string()))?;

    doc.save(output_path)
        .map(|_| ())
        .map_err(|e| PdfIoError::IoError(e.to_string()))?;

    Ok(())
}

// ---- helpers ----------------------------------------------------------------

/// Build a Page dictionary object.
fn make_page(
    pages_id: lopdf::ObjectId,
    resources_id: lopdf::ObjectId,
    content_id: lopdf::ObjectId,
) -> Object {
    Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Page".to_vec()));
        d.set("Parent", Object::Reference(pages_id));
        d.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(595),
                Object::Integer(842),
            ]),
        );
        d.set("Resources", Object::Reference(resources_id));
        d.set("Contents", Object::Reference(content_id));
        d
    })
}

fn escape_pdf_str(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'\\' => { out.push(b'\\'); out.push(b'\\'); }
            b'('  => { out.push(b'\\'); out.push(b'('); }
            b')'  => { out.push(b'\\'); out.push(b')'); }
            other => out.push(other),
        }
    }
    out
}

fn build_text_content(title: &str, body: &str) -> Result<Vec<u8>, PdfIoError> {
    let mut ops: Vec<Operation> = Vec::new();

    ops.push(Operation::new("BT", vec![]));

    // Title in 14pt
    ops.push(Operation::new(
        "Tf",
        vec![Object::Name(b"F1".to_vec()), Object::Integer(14)],
    ));
    ops.push(Operation::new(
        "Td",
        vec![Object::Integer(50), Object::Integer(790)],
    ));
    ops.push(Operation::new(
        "Tj",
        vec![Object::String(escape_pdf_str(title), lopdf::StringFormat::Literal)],
    ));

    // Switch to 10pt for body
    ops.push(Operation::new(
        "Tf",
        vec![Object::Name(b"F1".to_vec()), Object::Integer(10)],
    ));
    ops.push(Operation::new(
        "Td",
        vec![Object::Integer(0), Object::Integer(-22)],
    ));

    for line in body.lines() {
        ops.push(Operation::new(
            "Tj",
            vec![Object::String(escape_pdf_str(line), lopdf::StringFormat::Literal)],
        ));
        ops.push(Operation::new(
            "Td",
            vec![Object::Integer(0), Object::Integer(-13)],
        ));
    }

    ops.push(Operation::new("ET", vec![]));

    Content { operations: ops }
        .encode()
        .map_err(|e| PdfIoError::PdfParseError(e.to_string()))
}


