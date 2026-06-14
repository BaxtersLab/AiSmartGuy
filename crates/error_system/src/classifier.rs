use crate::errors::{EngineError, ModelError};
use crate::types::ErrorClass;

/// Map a raw engine error to its recovery class.
pub fn classify(error: &EngineError) -> ErrorClass {
    match error {
        EngineError::ModelError(ModelError::GpuError(_))        => ErrorClass::GpuFallback,
        EngineError::ModelError(ModelError::Timeout(_))         => ErrorClass::Retryable,
        EngineError::ModelError(ModelError::LoadFailure(_))     => ErrorClass::Recoverable,
        EngineError::ModelError(ModelError::InferenceFailure(_))=> ErrorClass::PartialRun,
        EngineError::ModelError(ModelError::IoError(_))         => ErrorClass::Recoverable,
        EngineError::ModelError(ModelError::InvalidState(_))    => ErrorClass::Fatal,
        EngineError::RagError(_)                                => ErrorClass::RagSkip,
        EngineError::PdfError(_)                                => ErrorClass::Fatal,
        EngineError::ManifestError(_)                           => ErrorClass::Fatal,
        EngineError::TimeoutError(_)                            => ErrorClass::Retryable,
        EngineError::IoError(_)                                 => ErrorClass::Recoverable,
        EngineError::OptimizationError(_)                       => ErrorClass::Recoverable,
        EngineError::UiError(_)                                 => ErrorClass::Recoverable,
        EngineError::UnknownError(_)                            => ErrorClass::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{ModelError, RagError};

    #[test]
    fn gpu_error_classified_as_fallback() {
        let e = EngineError::ModelError(ModelError::GpuError("OOM".to_string()));
        assert_eq!(classify(&e), ErrorClass::GpuFallback);
    }

    #[test]
    fn rag_error_classified_as_skip() {
        let e = EngineError::RagError(RagError::MalformedPacket("bad".to_string()));
        assert_eq!(classify(&e), ErrorClass::RagSkip);
    }

    #[test]
    fn pdf_error_classified_as_fatal() {
        let e = EngineError::PdfError(crate::errors::PdfError::ParseFailure("x".to_string()));
        assert_eq!(classify(&e), ErrorClass::Fatal);
    }
}
