use serde::{Deserialize, Serialize};
use manifest::Manifest;

/// Which stage the UI run is in.
/// Mirrors OrchestratorState plus UI-specific stages (Module P additions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UiStage {
    Idle,
    LoadingPdf,
    ExtractingMetadata,
    ApplyingConfiguration,
    Chunking,
    /// NEW (Module P): model is being downloaded from HuggingFace.
    FetchingModel,
    RunningModel,
    RunningFusion,
    UpdatingManifest,
    WritingFinalPdf,
    Completed,
    Error,
}

/// A conflict detected between the manifest and the runtime environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiConflict {
    MissingModel { name: String, path: String },
    MissingRagPacket { packet_id: String },
    GpuTooHigh { model: String, requested: String, available: String },
    UnknownCategory { category: String },
    IncompatibleManifestVersion { found: String, expected: String },
    /// Module P: model has no local copy and needs downloading.
    ModelNotDownloaded { name: String, repo_id: String },
    /// Module P: local model revision doesn't match manifest.
    ModelVersionMismatch { name: String, expected: String, found: String },
    /// Module P: network unavailable and model not in cache.
    OfflineModeMissingModel { name: String },
}

/// Real-time progress event emitted during a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiProgressEvent {
    pub stage: UiStage,
    pub message: String,
    pub percent: f32,
}

/// Per-model download status (Module P).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelDownloadStatus {
    pub model_name: String,
    pub filename: String,
    pub percent: f32,
    pub total_percent: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub complete: bool,
}

/// Top-level UI application state.
/// Must be Serialize so it can be returned from Tauri commands.
#[derive(Debug, Clone, Serialize)]
pub struct UiState {
    pub manifest: Option<Manifest>,
    pub pdf_loaded: bool,
    pub config_detected: bool,
    pub conflicts: Vec<UiConflict>,
    pub run_in_progress: bool,
    /// Active model download statuses (Module P).
    pub model_downloads: Vec<ModelDownloadStatus>,
    /// Path to the currently loaded PDF.
    pub pdf_path: Option<String>,
    /// Current run stage.
    pub stage: UiStage,
    /// Last error message, if any.
    pub last_error: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            manifest: None,
            pdf_loaded: false,
            config_detected: false,
            conflicts: Vec::new(),
            run_in_progress: false,
            model_downloads: Vec::new(),
            pdf_path: None,
            stage: UiStage::Idle,
            last_error: None,
        }
    }
}
