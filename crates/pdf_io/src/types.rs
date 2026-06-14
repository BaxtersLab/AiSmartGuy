/// All text extracted from a PDF, one string per page.
#[derive(Debug, Clone)]
pub struct ExtractedPdf {
    pub pages: Vec<String>,
    pub page_count: usize,
}

/// A deterministic text chunk ready for model inference.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: usize,
    pub text: String,
    pub start_page: usize,
    pub end_page: usize,
}

/// Manifest data found embedded in a PDF.
#[derive(Debug, Clone, Default)]
pub struct PdfMetadata {
    /// Base64-encoded manifest JSON found in the Info dictionary.
    pub manifest_base64: Option<String>,
    /// Raw manifest JSON found in the embedded file attachment.
    pub attachment_manifest: Option<String>,
}
