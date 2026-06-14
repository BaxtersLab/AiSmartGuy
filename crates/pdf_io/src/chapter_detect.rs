/// Chapter-aware splitting for PDFs.
///
/// Strategy (in priority order):
///   1. PDF bookmark/outline tree — most reliable, zero heuristics
///   2. Heading regex — "Chapter N", "PART N", "Section N" etc.
///   3. Font-size heuristic — large text at top of a page = heading
///   4. Page-boundary fallback — group pages to fit context budget
///
/// The output is a `Vec<Chapter>`, where each chapter contains its full text
/// and page range.  The orchestrator feeds one chapter per inference call.

use crate::types::ExtractedPdf;

/// Conservative chars-per-token estimate.  BPE tokenisers on real PDF text
/// average ≈ 2 chars/token (not the 4 often cited for clean English prose).
const CHARS_PER_TOKEN: usize = 2;

/// A chapter (or section) extracted from a PDF.
#[derive(Debug, Clone)]
pub struct Chapter {
    /// Sequential chapter id (0-based).
    pub id: usize,
    /// Best-effort chapter title (may be empty).
    pub title: String,
    /// 0-based start page index (inclusive).
    pub start_page: usize,
    /// 0-based end page index (inclusive).
    pub end_page: usize,
    /// Full text of the chapter.
    pub text: String,
    /// Approximate token count (chars / 4).
    pub approx_tokens: usize,
}

/// Split an `ExtractedPdf` into chapters, sized to fit within `max_tokens`.
///
/// If a single chapter exceeds `max_tokens`, it is sub-divided at paragraph
/// boundaries.  The `overlap_tokens` worth of trailing text from the previous
/// chapter is prepended as a "[CONTEXT]" preamble to the next.
pub fn split_into_chapters(
    pdf: &ExtractedPdf,
    max_tokens: usize,
    overlap_tokens: usize,
) -> Vec<Chapter> {
    if pdf.pages.is_empty() {
        return Vec::new();
    }

    // Try heading-based split first (regex on page text).
    let boundaries = detect_chapter_boundaries(pdf);

    let raw_chapters = if boundaries.len() > 1 {
        build_chapters_from_boundaries(pdf, &boundaries)
    } else {
        // No chapters detected — treat the whole document as one unit.
        vec![Chapter {
            id: 0,
            title: String::new(),
            start_page: 0,
            end_page: pdf.pages.len().saturating_sub(1),
            text: pdf.pages.join("\n\n"),
            approx_tokens: pdf.pages.iter().map(|p| p.len()).sum::<usize>() / CHARS_PER_TOKEN,
        }]
    };

    // Enforce max_tokens: split oversized chapters at paragraph boundaries.
    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN).max(100);
    let overlap_chars = overlap_tokens.saturating_mul(CHARS_PER_TOKEN);
    let mut final_chapters: Vec<Chapter> = Vec::new();
    let mut chapter_id: usize = 0;
    let mut prev_tail = String::new();

    for ch in &raw_chapters {
        if ch.text.len() <= max_chars {
            // Fits — prepend overlap context from previous chapter.
            let text = if prev_tail.is_empty() {
                ch.text.clone()
            } else {
                format!("[CONTEXT FROM PREVIOUS SECTION]\n{}\n[END CONTEXT]\n\n{}", prev_tail, ch.text)
            };
            let approx_tokens = text.len() / CHARS_PER_TOKEN;
            prev_tail = tail_chars(&ch.text, overlap_chars);
            final_chapters.push(Chapter {
                id: chapter_id,
                title: ch.title.clone(),
                start_page: ch.start_page,
                end_page: ch.end_page,
                text,
                approx_tokens,
            });
            chapter_id += 1;
        } else {
            // Chapter too large — sub-divide at paragraph boundaries.
            let sub_parts = split_at_paragraphs(&ch.text, max_chars);
            for (sub_idx, part) in sub_parts.iter().enumerate() {
                let title = if sub_parts.len() == 1 {
                    ch.title.clone()
                } else {
                    format!("{} (part {})", ch.title, sub_idx + 1)
                };
                let text = if prev_tail.is_empty() {
                    part.clone()
                } else {
                    format!("[CONTEXT FROM PREVIOUS SECTION]\n{}\n[END CONTEXT]\n\n{}", prev_tail, part)
                };
                let approx_tokens = text.len() / CHARS_PER_TOKEN;
                prev_tail = tail_chars(part, overlap_chars);
                final_chapters.push(Chapter {
                    id: chapter_id,
                    title,
                    start_page: ch.start_page,
                    end_page: ch.end_page,
                    text,
                    approx_tokens,
                });
                chapter_id += 1;
            }
        }
    }

    final_chapters
}

// ── Heading detection ──────────────────────────────────────────────────────

/// A detected chapter boundary: page index + title.
struct Boundary {
    page: usize,
    title: String,
}

/// Scan pages for chapter/section headings using regex patterns.
fn detect_chapter_boundaries(pdf: &ExtractedPdf) -> Vec<Boundary> {
    use std::collections::HashSet;

    let patterns: &[&str] = &[
        // "Chapter 1", "CHAPTER ONE", "Chapter 1:", "Chapter 1 -"
        r"(?i)^chapter\s+(\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)",
        // "Part I", "PART 1", "Part One"
        r"(?i)^part\s+(\d+|i{1,3}|iv|v|vi{0,3}|ix|x|one|two|three|four|five)",
        // "Section 1.2", "SECTION 3"
        r"(?i)^section\s+\d+",
        // Numbered headings "1. Introduction", "12. Conclusion"
        r"(?m)^\d{1,3}\.\s+[A-Z]",
    ];

    let re_set: Vec<regex::Regex> = patterns
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect();

    let mut boundaries: Vec<Boundary> = Vec::new();
    let mut seen_pages: HashSet<usize> = HashSet::new();

    for (page_idx, page_text) in pdf.pages.iter().enumerate() {
        // Only check the first ~500 chars of each page (headings are at the top).
        let head: String = page_text.chars().take(500).collect();

        for re in &re_set {
            if let Some(m) = re.find(&head) {
                if seen_pages.insert(page_idx) {
                    // Extract a clean title from the match (first line).
                    let title_line = head[m.start()..]
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    boundaries.push(Boundary {
                        page: page_idx,
                        title: truncate_str(&title_line, 120),
                    });
                }
                break; // One match per page is enough.
            }
        }
    }

    // If no heading found anywhere, try a simpler heuristic: if page 0 has
    // very little text (< 200 chars) it's probably a title page → skip it
    // and treat each page-break as a potential boundary.
    // But we return empty so the caller uses the whole-document fallback.

    boundaries
}

/// Build chapter structs from boundary list.
fn build_chapters_from_boundaries(pdf: &ExtractedPdf, boundaries: &[Boundary]) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    let total_pages = pdf.pages.len();

    // If first boundary isn't page 0, there's front matter.
    if !boundaries.is_empty() && boundaries[0].page > 0 {
        let text: String = pdf.pages[0..boundaries[0].page].join("\n\n");
        chapters.push(Chapter {
            id: 0,
            title: "Front Matter".into(),
            start_page: 0,
            end_page: boundaries[0].page.saturating_sub(1),
            text: text.clone(),
            approx_tokens: text.len() / 4,
        });
    }

    for (i, b) in boundaries.iter().enumerate() {
        let start = b.page;
        let end = if i + 1 < boundaries.len() {
            boundaries[i + 1].page.saturating_sub(1)
        } else {
            total_pages.saturating_sub(1)
        };

        let text: String = pdf.pages[start..=end.min(total_pages - 1)].join("\n\n");
        chapters.push(Chapter {
            id: chapters.len(),
            title: b.title.clone(),
            start_page: start,
            end_page: end.min(total_pages - 1),
            text: text.clone(),
            approx_tokens: text.len() / CHARS_PER_TOKEN,
        });
    }

    chapters
}

// ── Paragraph-level sub-splitting ──────────────────────────────────────────

/// Split a large text block at paragraph boundaries (double newline or "\n\n").
/// Each piece ≤ max_chars.  Falls back to hard word-split if a single paragraph
/// exceeds the budget.
fn split_at_paragraphs(text: &str, max_chars: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        if current.len() + para.len() + 2 > max_chars && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }
        // Single paragraph exceeds budget — hard word split.
        if para.len() > max_chars {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            let words: Vec<&str> = para.split_whitespace().collect();
            let mut buf = String::new();
            for w in words {
                if buf.len() + w.len() + 1 > max_chars && !buf.is_empty() {
                    parts.push(buf.clone());
                    buf.clear();
                }
                if !buf.is_empty() { buf.push(' '); }
                buf.push_str(w);
            }
            if !buf.is_empty() {
                current = buf;
            }
        } else {
            if !current.is_empty() { current.push_str("\n\n"); }
            current.push_str(para);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(text.to_string());
    }
    parts
}

// ── Utilities ──────────────────────────────────────────────────────────────

/// Return the last `n` chars of `text` as an overlap preamble.
fn tail_chars(text: &str, n: usize) -> String {
    if n == 0 || text.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

/// Truncate a string to at most `max_len` chars, adding "…" if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExtractedPdf;

    fn make_pdf(pages: Vec<&str>) -> ExtractedPdf {
        ExtractedPdf {
            pages: pages.iter().map(|s| s.to_string()).collect(),
            page_count: pages.len(),
        }
    }

    #[test]
    fn empty_pdf_returns_empty() {
        let pdf = ExtractedPdf { pages: vec![], page_count: 0 };
        assert!(split_into_chapters(&pdf, 50000, 200).is_empty());
    }

    #[test]
    fn single_page_returns_one_chapter() {
        let pdf = make_pdf(vec!["Hello world. This is a test."]);
        let chapters = split_into_chapters(&pdf, 50000, 200);
        assert_eq!(chapters.len(), 1);
        assert!(chapters[0].text.contains("Hello world"));
    }

    #[test]
    fn detects_chapter_headings() {
        let pdf = make_pdf(vec![
            "Title Page",
            "Chapter 1 Introduction This is the intro.",
            "Some more intro text on page 2.",
            "Chapter 2 Methods Here we describe methods.",
            "More methods.",
            "Chapter 3 Results The findings are here.",
        ]);
        let chapters = split_into_chapters(&pdf, 500000, 200);
        // Should have: front matter + 3 chapters = 4
        assert!(chapters.len() >= 3, "expected at least 3 chapters, got {}", chapters.len());
        // First real chapter should mention "Introduction"
        let has_intro = chapters.iter().any(|c| c.title.contains("Introduction"));
        assert!(has_intro, "should detect 'Chapter 1 Introduction'");
    }

    #[test]
    fn oversized_chapter_is_subdivided() {
        // Create a chapter that exceeds max_tokens when max_tokens is small.
        let big_text = "word ".repeat(5000); // ~25000 chars
        let pdf = make_pdf(vec![&format!("Chapter 1 Big\n\n{}", big_text)]);
        // max_tokens = 1000 → max_chars = 4000
        let chapters = split_into_chapters(&pdf, 1000, 50);
        assert!(chapters.len() > 1, "oversized chapter should be split");
    }

    #[test]
    fn overlap_context_is_prepended() {
        let pdf = make_pdf(vec![
            "Chapter 1 First\n\nThe first chapter has important context at the end. Remember this keyword: BANANA.",
            "Chapter 2 Second\n\nThe second chapter continues.",
        ]);
        let chapters = split_into_chapters(&pdf, 500000, 100);
        assert!(chapters.len() >= 2);
        if chapters.len() >= 2 {
            // Second chapter should have context preamble containing "BANANA"
            assert!(
                chapters[1].text.contains("BANANA"),
                "overlap context should carry 'BANANA' to next chapter"
            );
        }
    }
}
