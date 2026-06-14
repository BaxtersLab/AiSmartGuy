use std::collections::HashMap;
use std::path::PathBuf;

/// The orchestrator's internal state machine states.
#[derive(Debug, Clone, PartialEq)]
pub enum OrchestratorState {
    Idle,
    LoadingPdf,
    Chunking,
    FetchingModel,
    RunningModel,
    RunningFusion,
    UpdatingManifest,
    WritingFinalPdf,
    Completed,
    Error,
}

/// Which models to run and in which order.
#[derive(Debug, Clone)]
pub struct SequencePlan {
    /// Active model names in execution order. Fusion is always last.
    pub model_order: Vec<String>,
}

/// Context for the current chunk/model iteration.
#[derive(Debug, Clone)]
pub struct RunContext {
    pub current_chunk: usize,
    pub total_chunks: usize,
    pub model_name: String,
}

/// All per-model chunk outputs collected during a run.
/// key = model name, value = ordered list of output file paths
pub type ModelOutputs = HashMap<String, Vec<PathBuf>>;

/// Payload for fusion: the collected outputs from every model, per chunk.
#[derive(Debug, Clone)]
pub struct FusionInput {
    /// model name → ordered list of chunk output texts
    pub model_outputs: HashMap<String, Vec<String>>,
}

/// A UI progress event emitted by the orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorProgressEvent {
    pub stage: String,
    pub message: String,
    pub percent: f64,
}
