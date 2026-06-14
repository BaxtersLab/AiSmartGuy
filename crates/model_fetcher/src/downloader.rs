use std::io::{Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::cache::{model_cache_dir, part_path_of};
use crate::errors::{FetchResult, ModelFetchError};
use crate::hf_client::file_download_url;
use crate::progress::emit_progress;
use crate::state::ModelFetchState;
use crate::types::{FetchProgressEvent, ModelFetchRequest};

/// Download chunk size (4 MB).
const CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// Maximum retry attempts per shard.
const MAX_RETRIES: u32 = 5;

/// Base backoff delay in milliseconds (doubles each retry).
const BASE_BACKOFF_MS: u64 = 500;

/// HTTP timeout per request (seconds).
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Download all shards for a request, updating `state` as bytes arrive.
///
/// Shards are downloaded sequentially to keep complexity low and avoid
/// saturating the user's connection.  Each shard is written to a `.part`
/// file and atomically renamed to the final name on success.
pub fn download_shards(
    request: &ModelFetchRequest,
    state: &mut ModelFetchState,
) -> FetchResult<()> {
    let dir = model_cache_dir(request);
    utils::file::ensure_dir(&dir)
        .map_err(|e| ModelFetchError::IoError(e.to_string()))?;

    // Take a snapshot of the shard list so we can iterate without borrowing state.
    let shards: Vec<(String, u64)> = state
        .shards
        .iter()
        .filter(|s| !s.downloaded)
        .map(|s| (s.filename.clone(), s.size))
        .collect();

    for (filename, expected_size) in shards {
        let final_path = dir.join(&filename);

        // Skip if already complete.
        if final_path.exists() && !part_path_of(&final_path).exists() {
            state.mark_shard_done(&filename, expected_size);
            continue;
        }

        download_shard_with_retry(request, &filename, expected_size, &final_path, state)?;
    }

    Ok(())
}

// ---- per-shard download with retry -----------------------------------------

fn download_shard_with_retry(
    request: &ModelFetchRequest,
    filename: &str,
    expected_size: u64,
    final_path: &Path,
    state: &mut ModelFetchState,
) -> FetchResult<()> {
    let part_path = part_path_of(final_path);
    let url = file_download_url(
        &request.repo_id,
        request.revision.as_deref(),
        filename,
    );

    let mut last_err: Option<ModelFetchError> = None;

    for attempt in 0..MAX_RETRIES {
        if attempt > 0 {
            let backoff = BASE_BACKOFF_MS * (1 << attempt.min(6)); // cap at 64×
            thread::sleep(Duration::from_millis(backoff));
        }

        // Resume: check how many bytes we already have in the .part file.
        let already_downloaded: u64 = if part_path.exists() {
            std::fs::metadata(&part_path)
                .map(|m| m.len())
                .unwrap_or(0)
        } else {
            0
        };

        let result = attempt_download(
            &url,
            filename,
            &part_path,
            already_downloaded,
            expected_size,
            state,
        );

        match result {
            Ok(()) => {
                // Atomic rename: .part → final
                std::fs::rename(&part_path, final_path)
                    .map_err(|e| ModelFetchError::IoError(e.to_string()))?;
                state.mark_shard_done(filename, expected_size);
                return Ok(());
            }
            Err(ModelFetchError::RateLimit(msg)) => {
                // Rate limit: short back-off then retry (don't freeze the caller).
                eprintln!("[model_fetcher] rate-limited on {filename}, waiting 15 s before retry");
                thread::sleep(Duration::from_secs(15));
                last_err = Some(ModelFetchError::RateLimit(msg));
            }
            Err(e) => {
                last_err = Some(e);
                // Continue to next retry.
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        ModelFetchError::NetworkError(format!("failed to download {} after {MAX_RETRIES} attempts", filename))
    }))
}

// ---- single download attempt -----------------------------------------------

fn attempt_download(
    url: &str,
    filename: &str,
    part_path: &Path,
    resume_from: u64,
    total_size: u64,
    state: &mut ModelFetchState,
) -> FetchResult<()> {
    let mut request = ureq::get(url)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS));

    if resume_from > 0 {
        request = request.set("Range", &format!("bytes={}-", resume_from));
    }

    let response = request.call().map_err(|e| {
        let msg = e.to_string();
        if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
            ModelFetchError::RateLimit(msg)
        } else if msg.contains("timed out") || msg.contains("timeout") {
            ModelFetchError::Timeout(msg)
        } else {
            ModelFetchError::NetworkError(msg)
        }
    })?;

    let status = response.status();
    if status == 429 {
        return Err(ModelFetchError::RateLimit("HTTP 429".to_string()));
    }
    // 200 (full) or 206 (partial / resume) are both acceptable.
    if status != 200 && status != 206 {
        return Err(ModelFetchError::NetworkError(format!(
            "HTTP {} for {}",
            status, url
        )));
    }

    // Open the .part file for appending.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(part_path)
        .map_err(|e| ModelFetchError::IoError(e.to_string()))?;

    let mut reader = response.into_reader();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut written: u64 = resume_from;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| ModelFetchError::NetworkError(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| ModelFetchError::IoError(e.to_string()))?;
        written += n as u64;

        // Emit progress event after each chunk.
        emit_progress(&FetchProgressEvent {
            filename: filename.to_string(),
            percent: if total_size > 0 {
                written as f32 / total_size as f32
            } else {
                0.0
            },
            total_percent: if state.total_bytes > 0 {
                (state.downloaded_bytes + written) as f32 / state.total_bytes as f32
            } else {
                0.0
            },
            downloaded_bytes: written,
            total_bytes: total_size,
        });
    }

    Ok(())
}
