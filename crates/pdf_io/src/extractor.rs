use std::path::Path;

use lopdf::Document;
use crate::errors::PdfIoError;
use crate::types::ExtractedPdf;

/// Remove control characters except newlines, then normalise whitespace.
fn normalize_text(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ── pdf_extract (primary) ──────────────────────────────────────────────────

/// Try the `pdf_extract` crate first — it understands font tables, CMaps,
/// ToUnicode mappings, and word spacing.  Wrapped in catch_unwind because
/// it panics on some malformed PDFs.
fn try_pdf_extract(path: &Path) -> Result<Vec<String>, String> {
    let raw = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text(path)
    })) {
        Ok(Ok(text)) => text,
        Ok(Err(e))   => return Err(format!("pdf_extract error: {}", e)),
        Err(panic)   => {
            let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            return Err(format!("pdf_extract panicked: {}", msg));
        }
    };

    let pages: Vec<String> = raw
        .split('\x0C')
        .map(normalize_text)
        .filter(|s| !s.is_empty())
        .collect();

    if pages.is_empty() {
        return Err("pdf_extract returned no text".into());
    }
    Ok(pages)
}

// ── lopdf fallback ─────────────────────────────────────────────────────────

fn get_float(obj: &lopdf::Object) -> f64 {
    match obj {
        lopdf::Object::Integer(n) => *n as f64,
        lopdf::Object::Real(n) => *n as f64,
        _ => 0.0,
    }
}

/// Decode a PDF string operand into UTF-8 text.
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // UTF-16 BE with BOM
        let mut out = String::new();
        let mut i = 2;
        while i + 1 < bytes.len() {
            let code = ((bytes[i] as u16) << 8) | bytes[i + 1] as u16;
            if let Some(c) = char::from_u32(code as u32) {
                out.push(c);
            }
            i += 2;
        }
        return out;
    }
    // PDFDocEncoding ≈ latin-1 for printable range
    bytes.iter().map(|&b| b as char).collect()
}

/// Extract text from one page using lopdf content-stream parsing.
///
/// Tracks the text matrix (Td/TD offsets) and font size (Tf) to decide
/// where word-spaces belong — instead of blindly inserting a space between
/// every Tj call.
fn extract_page_text_lopdf(doc: &Document, page_id: lopdf::ObjectId) -> String {
    let content_data = match doc.get_page_content(page_id) {
        Ok(d)  => d,
        Err(_) => return String::new(),
    };
    let operations = match lopdf::content::Content::decode(&content_data) {
        Ok(c)  => c.operations,
        Err(_) => return String::new(),
    };

    let mut text = String::new();
    let mut font_size: f64 = 12.0;
    let mut word_space: f64 = 0.0;   // Tw
    let mut pending_space = false;
    let mut last_was_text = false;

    for op in &operations {
        match op.operator.as_str() {
            // ── Font / state operators ─────────────────────────────
            "Tf" => {
                // operands: /FontName  size
                if op.operands.len() >= 2 {
                    let sz = get_float(&op.operands[1]).abs();
                    if sz > 0.5 { font_size = sz; }
                }
            }
            "Tw" => {
                // Word spacing (added to every space char 0x20)
                if !op.operands.is_empty() {
                    word_space = get_float(&op.operands[0]);
                }
            }

            // ── Text positioning ───────────────────────────────────
            "Td" | "TD" => {
                if op.operands.len() >= 2 {
                    let tx = get_float(&op.operands[0]);
                    let ty = get_float(&op.operands[1]);
                    // Vertical movement → new line
                    if ty.abs() > 0.5 {
                        if last_was_text { pending_space = true; }
                    }
                    // Horizontal displacement larger than ~25% of font size → word gap
                    else if last_was_text && tx > font_size * 0.25 {
                        pending_space = true;
                    }
                }
            }
            "T*" => {
                // Move to start of next text line (newline)
                if last_was_text { pending_space = true; }
            }
            "Tm" | "cm" => {
                // New text matrix / concat matrix — treat as potential break
                // Only insert a space, not when it's the first text op
                if last_was_text { pending_space = true; }
            }
            "BT" => {
                // Begin text object
                if last_was_text { pending_space = true; }
            }

            // ── Text showing operators ─────────────────────────────
            "Tj" => {
                // Single string
                for operand in &op.operands {
                    if let lopdf::Object::String(bytes, _) = operand {
                        let decoded = decode_pdf_string(bytes);
                        let trimmed = decoded.trim();
                        if !trimmed.is_empty() {
                            if pending_space && last_was_text {
                                text.push(' ');
                            }
                            pending_space = false;
                            text.push_str(trimmed);
                            last_was_text = true;
                        }
                    }
                }
            }
            "TJ" => {
                // Array of strings interleaved with kerning adjustments
                for operand in &op.operands {
                    if let lopdf::Object::Array(arr) = operand {
                        if pending_space && last_was_text {
                            text.push(' ');
                            pending_space = false;
                        }
                        for item in arr {
                            match item {
                                lopdf::Object::String(bytes, _) => {
                                    let decoded = decode_pdf_string(bytes);
                                    // Don't trim — preserve internal structure
                                    if !decoded.is_empty() {
                                        text.push_str(&decoded);
                                        last_was_text = true;
                                    }
                                }
                                lopdf::Object::Integer(n) => {
                                    // Kerning: large negative = word space
                                    // Typical character kerning: -20 to -80
                                    // Word space: -200 and beyond
                                    let threshold = -(font_size * 15.0) as i64;
                                    if *n < threshold.min(-120) && last_was_text {
                                        text.push(' ');
                                    }
                                }
                                lopdf::Object::Real(n) => {
                                    let threshold = -(font_size * 15.0) as f32;
                                    if *n < threshold.min(-120.0) && last_was_text {
                                        text.push(' ');
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            "'" => {
                // Move to next line and show string (like T* then Tj)
                if last_was_text { text.push(' '); }
                for operand in &op.operands {
                    if let lopdf::Object::String(bytes, _) = operand {
                        let decoded = decode_pdf_string(bytes);
                        let trimmed = decoded.trim();
                        if !trimmed.is_empty() {
                            text.push_str(trimmed);
                            last_was_text = true;
                        }
                    }
                }
                pending_space = false;
            }
            "\"" => {
                // Set word/char spacing, move to next line, show string
                if last_was_text { text.push(' '); }
                // Operands: aw  ac  string
                if op.operands.len() >= 3 {
                    word_space = get_float(&op.operands[0]);
                    if let lopdf::Object::String(bytes, _) = &op.operands[2] {
                        let decoded = decode_pdf_string(bytes);
                        let trimmed = decoded.trim();
                        if !trimmed.is_empty() {
                            text.push_str(trimmed);
                            last_was_text = true;
                        }
                    }
                }
                pending_space = false;
            }
            _ => {}
        }
    }
    let _ = word_space; // suppress unused warning
    text
}

/// Extract text from a PDF using lopdf as fallback.
fn try_lopdf(path: &Path) -> Result<Vec<String>, String> {
    let doc = Document::load(path)
        .map_err(|e| format!("lopdf load failed: {}", e))?;

    let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();
    if page_ids.is_empty() {
        return Err("PDF contains no pages".into());
    }

    let mut pages: Vec<String> = Vec::with_capacity(page_ids.len());
    for page_id in &page_ids {
        let raw = extract_page_text_lopdf(&doc, *page_id);
        let normalized = normalize_text(&raw);
        if !normalized.is_empty() {
            pages.push(normalized);
        }
    }

    if pages.is_empty() {
        return Err("lopdf extracted no text".into());
    }
    Ok(pages)
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Heuristic: detect garbled/shifted text from PDFs with broken font encoding.
/// Samples from start, middle, and end of the document to catch cases where
/// front matter is clean but body text uses a custom (unreadable) encoding.
/// Returns true if the text looks like real readable content.
fn text_looks_sane(pages: &[String]) -> bool {
    if pages.is_empty() { return false; }

    // Build three ~800-char samples: start, middle, end
    let all_chars: Vec<char> = pages.iter().flat_map(|p| p.chars()).collect();
    let total_chars = all_chars.len();
    if total_chars < 20 { return true; } // too short to judge

    let sample_size = 800.min(total_chars);
    let offsets = [
        0,
        total_chars.saturating_sub(sample_size) / 2,
        total_chars.saturating_sub(sample_size),
    ];

    for &start in &offsets {
        let end = (start + sample_size).min(total_chars);
        let slice: String = all_chars[start..end].iter().collect();
        let len = slice.len() as f64;

        let readable = slice.chars()
            .filter(|c| c.is_ascii_alphanumeric() || " .,;:!?'-\"()\n".contains(*c))
            .count() as f64;
        let ratio = readable / len;

        let lower = slice.to_lowercase();
        let common_word_hits = ["the ", " and ", " of ", " to ", " is ", " in ", " a "]
            .iter()
            .filter(|w| lower.contains(*w))
            .count();

        // Per-glyph spaced PDFs produce mostly single-character tokens.
        // Real English text has multi-character words, so a high ratio of
        // single-char tokens is a strong garble signal.
        let tokens: Vec<&str> = slice.split_whitespace().collect();
        let single_char_ratio = if tokens.is_empty() {
            0.0
        } else {
            tokens.iter().filter(|t| t.len() == 1).count() as f64 / tokens.len() as f64
        };

        // If ANY sample fails the check, flag the whole document as garbled.
        // Require 2+ common words (not just 1) to avoid false positives from
        // single-char coincidences like shifted 'y' matching " a ".
        // Also flag if >50% of tokens are single characters.
        if ratio <= 0.60 || common_word_hits < 2 || single_char_ratio > 0.50 {
            return false;
        }
    }

    true
}

/// Attempt to repair garbled text by finding a byte-shift offset.
///
/// Some PDFs encode glyphs with a fixed byte offset from their real Unicode
/// code-points (e.g. 'R' in the file actually means 'o' because every byte
/// is stored 29 lower than its true value).  We brute-force offsets 1–255,
/// applying each to printable ASCII graphic chars, and return the first
/// result that passes `text_looks_sane`.
fn try_byte_shift_repair(pages: &[String]) -> Option<Vec<String>> {
    for shift in 1u16..=255 {
        let shifted_pages: Vec<String> = pages
            .iter()
            .map(|page| {
                page.chars()
                    .map(|c| {
                        if c.is_ascii_graphic() {
                            let new_b = ((c as u16) + shift) & 0xFF;
                            let nb = new_b as u8;
                            if nb >= 0x20 && nb <= 0x7E {
                                nb as char
                            } else {
                                c
                            }
                        } else {
                            c // keep spaces, newlines, etc.
                        }
                    })
                    .collect()
            })
            .collect();

        if text_looks_sane(&shifted_pages) {
            eprintln!("[pdf_io] byte-shift repair succeeded (offset +{})", shift);
            return Some(shifted_pages);
        }
    }
    None
}

/// Collapse single-character spacing produced by per-glyph positioned PDFs.
///
/// Detects the pattern `X Y Z` (single chars separated by single spaces)
/// and collapses them into `XYZ`, preserving real word boundaries (runs of
/// 2+ spaces, newlines) and normally-spaced words.
fn collapse_char_spacing(pages: &[String]) -> Vec<String> {
    pages
        .iter()
        .map(|page| {
            // Count how much of the text is single-char-space-single-char
            let chars: Vec<char> = page.chars().collect();
            let n = chars.len();
            if n < 5 {
                return page.clone();
            }
            let mut singles = 0usize;
            let mut i = 0;
            while i + 2 < n {
                if chars[i] != ' ' && chars[i + 1] == ' ' && chars[i + 2] != ' '
                    && (i == 0 || chars[i - 1] == ' ')
                {
                    singles += 1;
                }
                i += 1;
            }
            let density = singles as f64 / (n as f64 / 2.0);
            // If less than 40% of the text is single-char spaced, leave it alone
            if density < 0.40 {
                return page.clone();
            }
            // Collapse: remove spaces between isolated single characters
            let mut out = String::with_capacity(n);
            let mut j = 0;
            while j < n {
                out.push(chars[j]);
                // If current char is graphic and next is space followed by graphic,
                // AND the current char is a single (prev was space or start)
                if j + 2 < n
                    && chars[j] != ' '
                    && chars[j + 1] == ' '
                    && chars[j + 2] != ' '
                    && (j == 0 || chars[j - 1] == ' ' || out.ends_with(|c: char| c != ' '))
                {
                    // Skip the space — it's inter-character, not inter-word
                    j += 1;
                    continue;
                }
                j += 1;
            }
            out
        })
        .collect()
}

/// Extract the text contents of a PDF.
///
/// Strategy:
///   1. Try `pdf_extract` first — it handles font tables, CMaps and spacing.
///   2. If that panics, errors, or produces garbled text, fall back to lopdf.
///   3. If both produce garbled text, attempt byte-shift repair (brute-force
///      offsets 1–255) with optional space-collapse for per-glyph positioned PDFs.
///   4. If nothing works, return an error describing the problem.
pub fn extract_text(path: &Path) -> Result<ExtractedPdf, PdfIoError> {
    let mut garbled: Option<Vec<String>> = None;

    // Primary: pdf_extract (best quality, but can panic on edge-case PDFs)
    match try_pdf_extract(path) {
        Ok(pages) if text_looks_sane(&pages) => {
            let page_count = pages.len();
            return Ok(ExtractedPdf { pages, page_count });
        }
        Ok(pages) => {
            eprintln!("[pdf_io] pdf_extract returned garbled text, trying lopdf fallback");
            garbled = Some(pages);
        }
        Err(e) => {
            eprintln!("[pdf_io] pdf_extract failed, trying lopdf fallback: {}", e);
        }
    }

    // Fallback: lopdf raw content-stream extraction
    match try_lopdf(path) {
        Ok(pages) if text_looks_sane(&pages) => {
            let page_count = pages.len();
            return Ok(ExtractedPdf { pages, page_count });
        }
        Ok(pages) => {
            if garbled.is_none() {
                garbled = Some(pages);
            }
        }
        Err(e) => {
            eprintln!("[pdf_io] lopdf also failed: {}", e);
        }
    }

    // Byte-shift repair: try offsets 1–255 on the garbled extraction
    if let Some(ref pages) = garbled {
        // Try raw shift first
        if let Some(repaired) = try_byte_shift_repair(pages) {
            let page_count = repaired.len();
            return Ok(ExtractedPdf { pages: repaired, page_count });
        }
        // Try shift + space-collapse (for per-glyph positioned PDFs)
        let collapsed = collapse_char_spacing(pages);
        if let Some(repaired) = try_byte_shift_repair(&collapsed) {
            let page_count = repaired.len();
            eprintln!("[pdf_io] byte-shift repair with space-collapse succeeded");
            return Ok(ExtractedPdf { pages: repaired, page_count });
        }
    }

    Err(PdfIoError::PdfParseError(
        "PDF uses custom font encoding that cannot be decoded. \
         All repair strategies (pdf_extract, lopdf, byte-shift) were tried. \
         Try re-saving the PDF with 'Print to PDF' or a tool like Adobe Acrobat \
         to fix the font mappings."
            .to_string(),
    ))
}
