// AiSmartGuy — pdf_io crate  (Phase 3)

pub mod attachment;
pub mod chapter_detect;
pub mod chunker;
pub mod errors;
pub mod extractor;
pub mod metadata_embed;
pub mod metadata_extract;
pub mod pdf_writer;
pub mod types;

pub use attachment::{embed_attachment, extract_attachment};
pub use chapter_detect::{split_into_chapters, Chapter};
pub use chunker::chunk_text;
pub use errors::PdfIoError;
pub use extractor::extract_text;
pub use metadata_embed::embed_manifest;
pub use metadata_extract::extract_manifest;
pub use pdf_writer::write_final_pdf;
pub use types::{Chunk, ExtractedPdf, PdfMetadata};
