#[derive(Debug)]
pub enum PdfIoError {
    IoError(String),
    PdfParseError(String),
    MetadataError(String),
    AttachmentError(String),
    ChunkingError(String),
}

impl std::fmt::Display for PdfIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfIoError::IoError(m)         => write!(f, "PDF IO error: {}", m),
            PdfIoError::PdfParseError(m)   => write!(f, "PDF parse error: {}", m),
            PdfIoError::MetadataError(m)   => write!(f, "PDF metadata error: {}", m),
            PdfIoError::AttachmentError(m) => write!(f, "PDF attachment error: {}", m),
            PdfIoError::ChunkingError(m)   => write!(f, "PDF chunking error: {}", m),
        }
    }
}

impl std::error::Error for PdfIoError {}
