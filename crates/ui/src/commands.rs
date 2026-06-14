use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use model_fetcher::api::ModelFetcher;
use model_fetcher::types::ModelFetchRequest;

use crate::bridge_manifest::{detect_manifest_conflicts, load_manifest, validate_manifest};
use crate::bridge_orchestrator::run_orchestrator;
use crate::bridge_pdf::extract_manifest_from_pdf;
use crate::errors::{UiError, UiResult};
use crate::events::{emit_error, emit_model_download_progress, emit_progress, emit_run_completed};
use crate::state::SharedUiState;
use crate::types::{ModelDownloadStatus, UiConflict, UiStage};

// ---------------------------------------------------------------------------
// load_pdf
// ---------------------------------------------------------------------------

/// Load a PDF, optionally extracting an embedded manifest.
/// Returns a clone of the updated `UiState`.
pub fn load_pdf(state: SharedUiState, pdf_path: impl AsRef<Path>) -> UiResult<()> {
    let path = pdf_path.as_ref();

    emit_progress(UiStage::LoadingPdf, "Loading PDF…", 0.05);

    {
        let mut s = state.lock().unwrap();
        s.stage = UiStage::LoadingPdf;
        s.pdf_path = Some(path.to_string_lossy().into_owned());
        s.pdf_loaded = false;
        s.manifest = None;
        s.config_detected = false;
        s.conflicts.clear();
    }

    emit_progress(UiStage::ExtractingMetadata, "Extracting embedded manifest…", 0.15);

    let maybe_manifest = extract_manifest_from_pdf(path)?;

    let mut s = state.lock().unwrap();
    s.pdf_loaded = true;

    if let Some(m) = maybe_manifest {
        s.config_detected = true;
        let conflicts = detect_manifest_conflicts(&m);
        s.manifest = Some(m);
        s.conflicts = conflicts;
    }

    s.stage = UiStage::Idle;
    Ok(())
}

// ---------------------------------------------------------------------------
// apply_configuration
// ---------------------------------------------------------------------------

/// Load and validate the manifest from a given path, updating shared state.
pub fn apply_configuration(state: SharedUiState, manifest_path: impl AsRef<Path>) -> UiResult<()> {
    let path: PathBuf = manifest_path.as_ref().to_path_buf();

    emit_progress(UiStage::ApplyingConfiguration, "Applying configuration…", 0.20);

    let manifest = load_manifest(&path)?;
    validate_manifest(&manifest)?;
    let conflicts = detect_manifest_conflicts(&manifest);

    let mut s = state.lock().unwrap();
    s.manifest = Some(manifest);
    s.config_detected = true;
    s.conflicts = conflicts;
    s.stage = UiStage::Idle;
    Ok(())
}

// ---------------------------------------------------------------------------
// resolve_conflict
// ---------------------------------------------------------------------------

/// Apply a user-supplied resolution for a single conflict.
/// For now this records the resolution decision in the state's conflict list
/// by removing the resolved entry.
pub fn resolve_conflict(state: SharedUiState, conflict: UiConflict) -> UiResult<()> {
    let mut s = state.lock().unwrap();

    // Remove any conflict that matches the supplied variant by discriminant.
    s.conflicts.retain(|c| {
        std::mem::discriminant(c) != std::mem::discriminant(&conflict)
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// start_run
// ---------------------------------------------------------------------------

/// Start the full orchestrator pipeline.
/// The shared state must already have a loaded manifest and PDF path.
pub fn start_run(
    state: SharedUiState,
    manifest_path: PathBuf,
    run_dir: PathBuf,
) -> UiResult<()> {
    let pdf_path = {
        let s = state.lock().unwrap();
        let raw = s
            .pdf_path
            .as_ref()
            .ok_or_else(|| UiError::IoError("No PDF loaded".to_string()))?
            .clone();
        PathBuf::from(raw)
    };

    {
        let mut s = state.lock().unwrap();
        s.run_in_progress = true;
        s.stage = UiStage::RunningModel;
        s.last_error = None;
    }

    emit_progress(UiStage::RunningModel, "Running pipeline…", 0.30);

    match run_orchestrator(manifest_path, run_dir, pdf_path) {
        Ok(output) => {
            let output_str = output.to_string_lossy().into_owned();
            emit_run_completed(&output_str);
            let mut s = state.lock().unwrap();
            s.run_in_progress = false;
            s.stage = UiStage::Completed;
            Ok(())
        }
        Err(e) => {
            let msg = format!("{:?}", e);
            emit_error(&msg);
            let mut s = state.lock().unwrap();
            s.run_in_progress = false;
            s.stage = UiStage::Error;
            s.last_error = Some(msg.clone());
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// cancel_run
// ---------------------------------------------------------------------------

/// Signal a running pipeline to stop at the next cancellation checkpoint.
/// The caller holds the `AtomicBool`; this function flips it.
pub fn cancel_run(cancel_flag: &Arc<AtomicBool>) {
    cancel_flag.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// download_model  (Module P)
// ---------------------------------------------------------------------------

/// Begin downloading a model by its repo/name into the given cache directory.
pub fn download_model(
    state: SharedUiState,
    model_name: impl Into<String>,
    repo_id: impl Into<String>,
    cache_dir: PathBuf,
) -> UiResult<()> {
    let name = model_name.into();
    let repo = repo_id.into();

    emit_progress(UiStage::FetchingModel, format!("Fetching {}…", name), 0.10);

    {
        let mut s = state.lock().unwrap();
        s.stage = UiStage::FetchingModel;
        let status = ModelDownloadStatus {
            model_name: name.clone(),
            complete: false,
            ..Default::default()
        };
        // Replace existing entry or push a new one.
        if let Some(entry) = s.model_downloads.iter_mut().find(|d| d.model_name == name) {
            *entry = status;
        } else {
            s.model_downloads.push(status);
        }
    }

    let request = ModelFetchRequest {
        repo_id: repo,
        revision: None,
        target_dir: cache_dir,
    };

    static NO_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    ModelFetcher::ensure_model_available(request, &NO_CANCEL)
        .map_err(|e| UiError::ModelFetchError(format!("{:?}", e)))?;

    {
        let mut s = state.lock().unwrap();
        if let Some(entry) = s.model_downloads.iter_mut().find(|d| d.model_name == name) {
            entry.complete = true;
            entry.percent = 100.0;
            entry.total_percent = 100.0;
            emit_model_download_progress(entry);
        }
        s.stage = UiStage::Idle;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// retry_model_download  (Module P)
// ---------------------------------------------------------------------------

/// Retry a previously failed model download.
pub fn retry_model_download(
    state: SharedUiState,
    model_name: impl Into<String>,
    repo_id: impl Into<String>,
    cache_dir: PathBuf,
) -> UiResult<()> {
    let name = model_name.into();
    // Reset the status entry so the UI reflects a fresh start.
    {
        let mut s = state.lock().unwrap();
        if let Some(entry) = s.model_downloads.iter_mut().find(|d| d.model_name == name) {
            entry.complete = false;
            entry.percent = 0.0;
            entry.total_percent = 0.0;
            entry.downloaded_bytes = 0;
        }
    }
    download_model(state, name, repo_id, cache_dir)
}

// ---------------------------------------------------------------------------
// cancel_model_download  (Module P)
// ---------------------------------------------------------------------------

/// Mark a model download as cancelled in the UI state.
/// Note: the underlying HTTP request cannot be interrupted mid-flight with
/// the current synchronous `model_fetcher`; this marks the intent so the
/// next scheduled retry will be skipped.
pub fn cancel_model_download(state: SharedUiState, model_name: impl Into<String>) {
    let name = model_name.into();
    let mut s = state.lock().unwrap();
    s.model_downloads.retain(|d| d.model_name != name);
    s.stage = UiStage::Idle;
}
