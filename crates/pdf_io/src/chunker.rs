use crate::types::{Chunk, ExtractedPdf};

/// Split the text of all PDF pages into context-safe chunks.
///
/// `max_tokens`     — approximate token budget per chunk (4 chars ≈ 1 token)
/// `overlap_tokens` — how many tokens of context to repeat at chunk boundaries
///
/// Chunking is deterministic: same input always produces the same chunks.
pub fn chunk_text(pdf: &ExtractedPdf, max_tokens: usize, overlap_tokens: usize) -> Vec<Chunk> {
    let max_chars = max_tokens.saturating_mul(4).max(1);
    let overlap_chars = overlap_tokens.saturating_mul(4);

    // Build a flat word list, tracking which page each word came from.
    // (word_text, page_index)
    let mut words: Vec<(String, usize)> = Vec::new();
    for (page_idx, page_text) in pdf.pages.iter().enumerate() {
        for word in page_text.split_whitespace() {
            if !word.is_empty() {
                words.push((word.to_string(), page_idx));
            }
        }
    }

    if words.is_empty() {
        return Vec::new();
    }

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut chunk_id: usize = 0;
    let mut start: usize = 0;

    while start < words.len() {
        // Accumulate words up to max_chars
        let mut end = start;
        let mut char_count: usize = 0;
        while end < words.len() {
            char_count += words[end].0.len() + 1; // +1 for space separator
            if char_count > max_chars {
                break;
            }
            end += 1;
        }
        // Always advance at least one word to prevent infinite loop
        if end == start {
            end = start + 1;
        }

        let text = words[start..end]
            .iter()
            .map(|(w, _)| w.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let start_page = words[start].1;
        let end_page = words[end.saturating_sub(1)].1;

        chunks.push(Chunk { id: chunk_id, text, start_page, end_page });
        chunk_id += 1;

        if end >= words.len() {
            break;
        }

        // Advance with overlap: step back by ~`overlap_chars` worth of words
        let overlap_word_approx = (overlap_chars / 5).max(1).min(end - start);
        let new_start = end.saturating_sub(overlap_word_approx);
        // Guarantee forward progress
        start = new_start.max(end.saturating_sub(end - start - 1)).max(start + 1);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExtractedPdf;

    fn make_pdf(text: &str) -> ExtractedPdf {
        ExtractedPdf {
            pages: vec![text.to_string()],
            page_count: 1,
        }
    }

    #[test]
    fn empty_pdf_returns_empty() {
        let pdf = ExtractedPdf { pages: vec![], page_count: 0 };
        assert!(chunk_text(&pdf, 500, 50).is_empty());
    }

    #[test]
    fn small_text_produces_one_chunk() {
        let pdf = make_pdf("hello world foo bar");
        let chunks = chunk_text(&pdf, 500, 50);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("hello"));
    }

    #[test]
    fn chunk_ids_are_sequential() {
        let words: Vec<String> = (0..1000).map(|i| format!("word{}", i)).collect();
        let pdf = make_pdf(&words.join(" "));
        let chunks = chunk_text(&pdf, 100, 10);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.id, i);
        }
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let pdf = make_pdf("the quick brown fox jumps over the lazy dog repeated many times over");
        let a = chunk_text(&pdf, 50, 5);
        let b = chunk_text(&pdf, 50, 5);
        assert_eq!(a.len(), b.len());
        for (ca, cb) in a.iter().zip(b.iter()) {
            assert_eq!(ca.text, cb.text);
        }
    }
}
