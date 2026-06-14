use std::path::PathBuf;
use pdf_io::{extract_text, Chunk, Chapter, ExtractedPdf};
use crate::errors::{OrchestratorError, OrchestratorResult};

/// Bridge: extract text from a PDF file.
pub fn extract_pdf(path: &PathBuf) -> OrchestratorResult<ExtractedPdf> {
    extract_text(path).map_err(|e| OrchestratorError::PdfError(e.to_string()))
}

/// Bridge: split the extracted PDF into chapters sized to fit `max_tokens`.
pub fn chapter_split(pdf: &ExtractedPdf, max_tokens: usize, overlap: usize) -> Vec<Chapter> {
    pdf_io::split_into_chapters(pdf, max_tokens, overlap)
}

/// Bridge: chunk the extracted PDF into inference-sized pieces (legacy).
pub fn chunk_pdf(pdf: &ExtractedPdf, max_tokens: usize, overlap: usize) -> Vec<Chunk> {
    pdf_io::chunk_text(pdf, max_tokens, overlap)
}

/// Bridge: embed the final manifest JSON into the output PDF.
pub fn write_final_pdf(
    original: &PathBuf,
    results_text: &str,
    manifest_json: &str,
    out_path: &PathBuf,
) -> OrchestratorResult<()> {
    pdf_io::write_final_pdf(original, results_text, manifest_json, out_path)
        .map_err(|e| OrchestratorError::PdfError(e.to_string()))
}
