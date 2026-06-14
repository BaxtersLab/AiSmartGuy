use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use model_fetcher::{ModelFetcher, ModelFetchRequest};
use manifest::ModelConfig;
use crate::errors::{map_fetch_error, OrchestratorResult};
use crate::progress::emit_progress;
use crate::types::OrchestratorProgressEvent;

/// Ensure a model is locally available before the orchestrator loads it.
///
/// Builds a `ModelFetchRequest` from the manifest `ModelConfig`, calls
/// `ModelFetcher::ensure_model_available`, and emits a progress event.
///
/// On failure the fetcher error is mapped to `OrchestratorError::ModelFetchFailed`.
pub fn ensure_model_ready(
    model_name: &str,
    config: &ModelConfig,
    cache_root: &PathBuf,
) -> OrchestratorResult<()> {
    emit_progress(&OrchestratorProgressEvent {
        stage: "FETCHING_MODEL".to_string(),
        message: format!("checking model availability: {}", model_name),
        percent: 0.0,
    });

    // Derive a HF repo_id from the model path field.
    // Convention: config.path holds either a HF repo_id ("org/repo") or a
    // local absolute path.  If it looks like an absolute path that exists
    // on disk, skip the fetch.
    let model_path = PathBuf::from(&config.path);
    if model_path.is_absolute() && model_path.exists() {
        emit_progress(&OrchestratorProgressEvent {
            stage: "FETCHING_MODEL".to_string(),
            message: format!("model already on disk: {}", model_name),
            percent: 1.0,
        });
        return Ok(());
    }

    let request = ModelFetchRequest {
        repo_id: config.path.clone(),
        revision: config.revision.clone(),
        target_dir: cache_root.clone(),
    };

    static NO_CANCEL: AtomicBool = AtomicBool::new(false);
    ModelFetcher::ensure_model_available(request, &NO_CANCEL).map_err(map_fetch_error)?;

    emit_progress(&OrchestratorProgressEvent {
        stage: "FETCHING_MODEL".to_string(),
        message: format!("model ready: {}", model_name),
        percent: 1.0,
    });

    Ok(())
}
