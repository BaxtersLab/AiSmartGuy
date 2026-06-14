use error_system::ManifestError;

/// Local ManifestError alias re-exported for crate-internal use.
pub type ManifestResult<T> = Result<T, ManifestError>;
