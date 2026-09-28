use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream};

use crate::errors::PdfIoError;
use crate::metadata_embed::set_info_key;

/// Conservative max characters per drawn line at Helvetica 10pt within the
/// page's usable width (595pt page, 50pt left margin, ~495pt usable — this
/// text has a lot of capitals/brackets, which are wider than average, so
/// this stays well under the ~99-char point where real lines were observed
/// getting cut off).
const MAX_CHARS_PER_LINE: usize = 90;

/// Greedy word-wrap: breaks `line` into pieces no longer than `max_chars`,
/// breaking on whitespace where possible. A single "word" longer than
/// `max_chars` (rare — e.g. a long URL) is hard-broken rather than left to
/// overflow the page, since that's exactly the bug this fixes.
fn wrap_line(line: &str, max_chars: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let mut word = word;
        loop {
            let sep = if current.is_empty() { 0 } else { 1 };
            if current.len() + sep + word.len() <= max_chars {
                if sep == 1 {
                    current.push(' ');
                }
                current.push_str(word);
                break;
            }
            if current.is_empty() {
                // The word alone doesn't fit — hard-break it. Width stays in
                // bytes, never fewer than the glyphs drawn (win_ansi draws one
                // per character), but the cut must land on a char boundary: a
                // byte-indexed split inside a multi-byte character (an em
                // dash, say) panics.
                let mut cut = max_chars.min(word.len());
                while !word.is_char_boundary(cut) {
                    cut -= 1;
                }
                if cut == 0 {
                    // The first character alone is wider than max_chars bytes:
                    // take it whole so the loop always makes progress.
                    cut = word.chars().next().map_or(word.len(), char::len_utf8);
                }
                let (head, tail) = word.split_at(cut);
                out.push(head.to_string());
                word = tail;
                if word.is_empty() {
                    break;
                }
                continue;
            }
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Write the final output PDF as a standalone report (no original book pages),
/// with the manifest embedded in the Info dictionary.
pub fn write_final_pdf(
    original_pdf: &Path,
    results_text: &str,
    manifest_json: &str,
    output_path: &Path,
) -> Result<(), PdfIoError> {
    write_final_pdf_versioned(original_pdf, results_text, manifest_json, output_path, "1.5")
}

/// Same as [`write_final_pdf`], with the PDF version made explicit. The
/// cross-reference table is always the classic kind, whatever the version
/// (see below).
pub fn write_final_pdf_versioned(
    _original_pdf: &Path,
    results_text: &str,
    manifest_json: &str,
    output_path: &Path,
    version: &str,
) -> Result<(), PdfIoError> {
    let mut doc = Document::with_version(version);
    // A classic cross-reference table, for every version. lopdf 0.32 writes
    // an xref STREAM for a new document (1.4 included) indexed from object 1,
    // and with the manifest in /Info poppler then said "Syntax Error: Invalid
    // XRef entry 0" for every report (2026-09-28). The classic table always
    // carries the free entry 0.
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;

    // Built-in Helvetica font — no embedding required. WinAnsiEncoding, so
    // the bytes win_ansi writes draw curly quotes, dashes and accented
    // letters; with no /Encoding they fell back to StandardEncoding, which
    // dropped them or drew other glyphs.
    let font_id = doc.add_object(Object::Dictionary({
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Font".to_vec()));
        d.set("Subtype", Object::Name(b"Type1".to_vec()));
        d.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        d.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
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

    // Split the report into pages of ~55 lines each.
    //
    // `build_text_content` draws each line as a single `Tj` at a fixed X
    // with no word-wrap. The model's real output puts a whole paragraph of
    // analysis on one line (no internal newlines), regularly 200+
    // characters — Helvetica 10pt only fits ~90-95 chars in the page's
    // usable width (595pt page, 50pt margins), so anything past that ran
    // off the page and was silently lost (confirmed: every extracted line
    // from a real report was truncated to ~100 chars, with the very last
    // line cut off mid-word — not a parser/xref issue, just missing wrap).
    // Wrapping BEFORE chunking into pages so `lines_per_page` counts the
    // same physical lines that will actually be drawn.
    let title = "AiSmartGuy \u{2014} Analysis Results";
    let wrapped_lines: Vec<String> = results_text
        .lines()
        .flat_map(|l| wrap_line(l, MAX_CHARS_PER_LINE))
        .collect();
    let lines: Vec<&str> = wrapped_lines.iter().map(|s| s.as_str()).collect();
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

/// Text as WinAnsiEncoding bytes, the encoding the report font declares: one
/// byte per character. A character outside it becomes '?', visibly.
///
/// Nothing is escaped here. lopdf escapes backslashes and parentheses when it
/// writes a literal string, so escaping here as well drew a backslash before
/// every parenthesis of every report ("Model's claimed quote \(not found in
/// chapter\)").
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            ' '..='~' => c as u8,
            '\u{A0}'..='\u{FF}' => c as u8,
            '\t' => b' ',
            '\u{20AC}' => 0x80, '\u{201A}' => 0x82, '\u{0192}' => 0x83, '\u{201E}' => 0x84,
            '\u{2026}' => 0x85, '\u{2020}' => 0x86, '\u{2021}' => 0x87, '\u{02C6}' => 0x88,
            '\u{2030}' => 0x89, '\u{0160}' => 0x8A, '\u{2039}' => 0x8B, '\u{0152}' => 0x8C,
            '\u{017D}' => 0x8E, '\u{2018}' => 0x91, '\u{2019}' => 0x92, '\u{201C}' => 0x93,
            '\u{201D}' => 0x94, '\u{2022}' => 0x95, '\u{2013}' => 0x96, '\u{2014}' => 0x97,
            '\u{02DC}' => 0x98, '\u{2122}' => 0x99, '\u{0161}' => 0x9A, '\u{203A}' => 0x9B,
            '\u{0153}' => 0x9C, '\u{017E}' => 0x9E, '\u{0178}' => 0x9F,
            _ => b'?',
        })
        .collect()
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
        vec![Object::String(win_ansi(title), lopdf::StringFormat::Literal)],
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
            vec![Object::String(win_ansi(line), lopdf::StringFormat::Literal)],
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

#[cfg(test)]
mod wrap_tests {
    use super::*;

    /// The bug this regresses: report generation drew each raw line as one
    /// unwrapped `Tj`, so any line past ~99 chars ran off the page's right
    /// edge and was lost when the PDF was read back — confirmed against a
    /// real report, where every long analysis line was cut to ~100 chars,
    /// the last one mid-word.
    #[test]
    fn wrap_line_never_exceeds_the_max_width() {
        let long = "word ".repeat(100); // way over MAX_CHARS_PER_LINE
        for piece in wrap_line(&long, MAX_CHARS_PER_LINE) {
            assert!(
                piece.len() <= MAX_CHARS_PER_LINE,
                "piece {:?} is {} chars, over the {}-char page-width budget",
                piece, piece.len(), MAX_CHARS_PER_LINE
            );
        }
    }

    #[test]
    fn wrap_line_short_line_is_unchanged() {
        let short = "a short analysis line";
        assert_eq!(wrap_line(short, MAX_CHARS_PER_LINE), vec![short.to_string()]);
    }

    #[test]
    fn wrap_line_preserves_every_word_no_content_lost() {
        let original = "The passage includes pacing ('forty years'), embedded commands \
            ('Contributions to the Project Gutenberg Literary Archive Foundation are tax \
            deductible'), and future pace ('for generations to come').";
        let wrapped = wrap_line(original, MAX_CHARS_PER_LINE);
        let rejoined = wrapped.join(" ");
        for word in original.split_whitespace() {
            assert!(
                rejoined.contains(word),
                "word {:?} from the original line is missing after wrapping — this is the \
                 exact real-world sentence that was cut off mid-word in a real report",
                word
            );
        }
    }

    /// A single "word" longer than the max width on its own (e.g. a long
    /// token with no spaces) must still be hard-broken to fit, not left to
    /// overflow the page.
    #[test]
    fn wrap_line_hard_breaks_an_unbreakable_long_word() {
        let long_word = "x".repeat(250);
        for piece in wrap_line(&long_word, MAX_CHARS_PER_LINE) {
            assert!(piece.len() <= MAX_CHARS_PER_LINE);
        }
    }

    #[test]
    fn wrap_line_empty_line_stays_a_single_empty_line() {
        assert_eq!(wrap_line("", MAX_CHARS_PER_LINE), vec![String::new()]);
    }

    /// The hard break used a byte index, and model output is UTF-8: a
    /// spaceless run longer than the line with a multi-byte character across
    /// the cut panicked with "byte index is not a char boundary".
    #[test]
    fn wrap_line_hard_break_never_splits_a_multibyte_character() {
        // 89 ASCII bytes, then an em dash at bytes 89..92: byte 90 is inside it.
        let word = format!("{}\u{2014}tail", "a".repeat(89));
        let pieces = wrap_line(&word, MAX_CHARS_PER_LINE);
        assert_eq!(pieces.concat(), word, "hard break must not lose or alter text");
        for piece in &pieces {
            assert!(piece.len() <= MAX_CHARS_PER_LINE);
        }
    }

    /// Same defect at the real entry point: the panic fired inside
    /// write_final_pdf, the last step of a run, after every model had finished.
    /// The call is caught so the temp dir is removed even if that panic
    /// comes back.
    #[test]
    fn write_final_pdf_survives_a_multibyte_character_at_the_wrap_point() {
        let dir = std::env::temp_dir().join(format!("asg_pdf_wrap_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("report.pdf");
        let body = format!("{}\u{2014}tail\nsecond line", "a".repeat(89));
        let result = std::panic::catch_unwind(|| {
            write_final_pdf(Path::new("unused.pdf"), &body, "{}", &out)
        });
        let written = out.is_file();
        let _ = std::fs::remove_dir_all(&dir);
        result
            .expect("write_final_pdf panicked on UTF-8 text")
            .expect("write_final_pdf must not fail on UTF-8 text");
        assert!(written, "report PDF must be written");
    }

    /// A report shaped like a real one: two pages and a manifest of a few KB
    /// in /Info (the size that made poppler complain, 2026-09-28).
    fn real_shaped_report(tag: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("asg_xref_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text: String = (0..130).map(|i| format!("Finding {i}: a quoted line from the book.\n")).collect();
        let manifest = format!("{{\"run_id\":\"Run_2026-09-28_00-00-00\",\"note\":\"{}\"}}", "x".repeat(4000));
        let out = dir.join("report.pdf");
        write_final_pdf(Path::new("unused.pdf"), &text, &manifest, &out).unwrap();
        (out, manifest)
    }

    /// A classic cross-reference table, which always carries the free entry
    /// 0. lopdf's default xref stream is indexed from object 1.
    #[test]
    fn the_report_has_a_classic_xref_table_with_entry_zero() {
        let (out, _) = real_shaped_report("classic");
        let bytes = std::fs::read(&out).unwrap();
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\nxref\n0 "), "no classic xref table");
        assert!(text.contains("0000000000 65535 f"), "no free entry 0");
        assert!(!text.contains("/Type/XRef"), "an xref stream is still written");
    }

    /// poppler reads a real-shaped report with no warning. It used to print
    /// "Syntax Error: Invalid XRef entry 0" for every report. Fails closed
    /// if pdftotext (poppler-utils, installed by default on Ubuntu desktop)
    /// is missing.
    #[test]
    fn poppler_reads_a_real_shaped_report_without_a_warning() {
        let (out, _) = real_shaped_report("poppler");
        let run = std::process::Command::new("pdftotext").arg(&out).arg("-").output()
            .expect("pdftotext (poppler-utils) is required for this test");
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success() && stderr.trim().is_empty(), "poppler: {stderr}");
        assert!(String::from_utf8_lossy(&run.stdout).contains("Finding 129"), "text lost");
    }

    /// Control: the app still reads its own report's manifest back.
    #[test]
    fn the_app_reads_its_own_report_back() {
        use base64::Engine;
        let (out, manifest) = real_shaped_report("roundtrip");
        let meta = crate::metadata_extract::extract_manifest(&out).unwrap();
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(meta.manifest_base64.expect("manifest in /Info")).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), manifest);
    }

    const AS_WRITTEN: [&str; 4] = [
        "Model's claimed quote (not found in chapter): \"x\" and a\\b",
        "curly \u{201C}quoted\u{201D} it\u{2019}s \u{2018}so\u{2019}",
        "accents caf\u{E9} na\u{EF}ve \u{C5}ngstr\u{F6}m",
        "dashes a\u{2014}b c\u{2013}d, euro \u{20AC}5, ellipsis\u{2026}",
    ];

    /// What a reader sees, read back by poppler and by the app's own reader:
    /// parentheses and backslashes once, and curly quotes, dashes and accented
    /// letters as written. The writer escaped twice (every "(" drawn as "\(")
    /// and declared no encoding (curly quotes and dashes dropped, "café"
    /// drawn as "cafˆ'"). Fails closed if pdftotext is missing.
    #[test]
    fn report_text_reads_back_as_written() {
        let dir = std::env::temp_dir().join(format!("asg_pdf_text_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("report.pdf");
        write_final_pdf(Path::new("unused.pdf"), &AS_WRITTEN.join("\n"), "{}", &out).unwrap();
        let poppler = std::process::Command::new("pdftotext").arg(&out).arg("-").output()
            .expect("pdftotext (poppler-utils) is required for this test");
        let own = crate::extract_text(&out).map(|p| p.pages.join("\n"));
        let _ = std::fs::remove_dir_all(&dir);
        let poppler = String::from_utf8(poppler.stdout).unwrap();
        let own = own.unwrap();
        for line in AS_WRITTEN {
            assert!(poppler.contains(line), "poppler read {poppler:?}, not {line:?}");
            assert!(own.contains(line), "the app read {own:?}, not {line:?}");
        }
    }

    #[test]
    fn characters_outside_win_ansi_become_a_visible_question_mark() {
        assert_eq!(win_ansi("Greek \u{3B1}\u{3B2} ok"), b"Greek ?? ok".to_vec());
        assert_eq!(win_ansi("tab\there"), b"tab here".to_vec());
        assert_eq!(win_ansi("(a) b\\c"), b"(a) b\\c".to_vec(), "nothing escaped here");
        // Each mapped character has its own byte (tab shares the space's).
        let mapped: Vec<u8> = ('\u{0}'..='\u{FFFF}')
            .filter(|&c| c != '?')
            .map(|c| win_ansi(&c.to_string()))
            .filter(|b| b != b"?")
            .map(|b| b[0])
            .collect();
        let mut unique = mapped.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), mapped.len() - 1);
    }
}
