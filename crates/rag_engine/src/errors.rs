use error_system::RagError;

/// Re-export RagError for crate-internal convenience.
pub type RagResult<T> = Result<T, RagError>;
