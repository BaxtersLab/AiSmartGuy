use serde::Serialize;
use crate::types::{UiStage, UiProgressEvent, ModelDownloadStatus};

/// Emit a structured progress event to stdout as a JSON line.
/// The binary entrypoint (Tauri app) reads these from an IPC pipe / stdout.
pub fn emit_progress(stage: UiStage, message: impl Into<String>, percent: f32) {
    let event = UiProgressEvent {
        stage,
        message: message.into(),
        percent,
    };
    if let Ok(json) = serde_json::to_string(&event) {
        println!("{{\"event\":\"progress\",\"data\":{}}}", json);
    }
}

/// Emit an error event.
pub fn emit_error(message: impl Into<String>) {
    #[derive(Serialize)]
    struct ErrorEvent {
        message: String,
    }
    let ev = ErrorEvent { message: message.into() };
    if let Ok(json) = serde_json::to_string(&ev) {
        println!("{{\"event\":\"error\",\"data\":{}}}", json);
    }
}

/// Emit a run-completed event with the output file path.
pub fn emit_run_completed(output_path: impl Into<String>) {
    #[derive(Serialize)]
    struct CompletedEvent {
        output_path: String,
    }
    let ev = CompletedEvent { output_path: output_path.into() };
    if let Ok(json) = serde_json::to_string(&ev) {
        println!("{{\"event\":\"run_completed\",\"data\":{}}}", json);
    }
}

/// Emit a model-download-progress event (Module P).
pub fn emit_model_download_progress(status: &ModelDownloadStatus) {
    if let Ok(json) = serde_json::to_string(status) {
        println!("{{\"event\":\"model_download_progress\",\"data\":{}}}", json);
    }
}
