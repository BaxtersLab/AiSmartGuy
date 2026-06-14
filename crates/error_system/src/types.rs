/// Categorises an error for the recovery decision tree.
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorClass {
    Recoverable,
    Retryable,
    Fatal,
    GpuFallback,
    RagSkip,
    PartialRun,
}

/// The action the orchestrator must take in response to a classified error.
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryAction {
    Retry,
    SkipModel,
    SkipChunk,
    DowngradeGpu,
    SkipRagPacket,
    MarkPartialRun,
    AbortRun,
}

/// Mutable context threaded through recovery calls so retry counts
/// and partial-run state accumulate across the lifecycle of a single run.
#[derive(Debug, Default)]
pub struct RecoveryContext {
    pub model_failures: Vec<String>,
    pub failed_chunks: Vec<String>,
    pub fusion_partial: bool,
    /// Per-class retry counters so interleaved errors don't corrupt budgets.
    pub recoverable_retries: u32,
    pub retryable_retries: u32,
}
