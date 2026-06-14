use std::sync::atomic::{AtomicBool, Ordering};

use crate::cache::check_cache;
use crate::downloader::download_shards;
use crate::errors::{FetchResult, ModelFetchError};
use crate::hf_client::fetch_repo_files;
use crate::shard_detector::detect_shards;
use crate::state::ModelFetchState;
use crate::types::ModelFetchRequest;
use crate::verifier::verify_shards;

/// High-level model acquisition facade used by the orchestrator.
pub struct ModelFetcher;

impl ModelFetcher {
    /// Ensure the requested model is fully downloaded and verified.
    ///
    /// Pass a `cancel` flag that can be set to `true` from another thread;
    /// the function checks it between major phases and returns
    /// `ModelFetchError::Cancelled` immediately.
    ///
    /// Sequence (Module N §11):
    ///   1. Check local cache — skip download if all shards already present.
    ///   2. Fetch repo file listing from Hugging Face.
    ///   3. Detect GGUF shards (including multi-shard models).
    ///   4. Download any missing shards (with resume + retry).
    ///   5. Verify integrity (size + SHA-256).
    ///   6. Return Ok — model is ready for the loader.
    pub fn ensure_model_available(
        request: ModelFetchRequest,
        cancel: &AtomicBool,
    ) -> FetchResult<()> {
        // 1. Cache hit — nothing to do.
        if let Some(cached_state) = check_cache(&request) {
            if cached_state.complete {
                // Still verify to catch bit-rot.
                return verify_shards(&cached_state, &request);
            }
        }

        if cancel.load(Ordering::Relaxed) {
            return Err(ModelFetchError::Cancelled("download cancelled".into()));
        }

        // 2. Fetch repo file list.
        let files = fetch_repo_files(&request.repo_id, request.revision.as_deref())?;

        if cancel.load(Ordering::Relaxed) {
            return Err(ModelFetchError::Cancelled("download cancelled".into()));
        }

        // 3. Detect shards.
        let shards = detect_shards(&files)?;

        // 4. Download.
        let mut state = ModelFetchState::from_shards(shards);
        download_shards(&request, &mut state)?;

        if cancel.load(Ordering::Relaxed) {
            return Err(ModelFetchError::Cancelled("download cancelled".into()));
        }

        if !state.complete {
            return Err(ModelFetchError::MissingShard(
                "download completed but state is not marked complete".to_string(),
            ));
        }

        // 5. Verify.
        verify_shards(&state, &request)?;

        Ok(())
    }
}
