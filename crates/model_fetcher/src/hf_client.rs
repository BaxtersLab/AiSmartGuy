use serde::Deserialize;

use crate::errors::{FetchResult, ModelFetchError};
use crate::types::HfFile;

/// HF Hub API base URL.
const HF_API_BASE: &str = "https://huggingface.co/api";

/// Default revision when none is specified.
const DEFAULT_REVISION: &str = "main";

/// Maximum wait time (seconds) before treating any request as a timeout.
const REQUEST_TIMEOUT_SECS: u64 = 60;

// ---- internal deserialization types ----------------------------------------

/// One file entry from the /api/models/{id}/tree endpoint.
#[derive(Debug, Deserialize)]
struct HfTreeEntry {
    /// Relative path inside the repo.
    #[serde(rename = "rfilename")]
    rfilename: String,
    /// File size in bytes (absent for directories).
    size: Option<u64>,
    /// LFS metadata block, present for large files.
    lfs: Option<HfLfsMeta>,
}

#[derive(Debug, Deserialize)]
struct HfLfsMeta {
    /// SHA-256 of the actual file content, provided by HF LFS.
    sha256: Option<String>,
    /// The "oid" field is the SHA-256 without the "sha256:" prefix.
    oid: Option<String>,
    size: Option<u64>,
}

// ---- public API ------------------------------------------------------------

/// Percent-encode a path segment for use in a URL.
/// Preserves `/` (valid in repo_ids and nested filenames), but encodes
/// everything else that isn't an unreserved character (RFC 3986).
fn encode_path(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// Return the list of files in a Hugging Face repo.
///
/// Uses the undocumented but stable `/api/models/{id}/tree/{revision}` endpoint.
/// Rate-limit responses (HTTP 429) are surfaced as `ModelFetchError::RateLimit`.
pub fn fetch_repo_files(
    repo_id: &str,
    revision: Option<&str>,
) -> FetchResult<Vec<HfFile>> {
    let rev = revision.unwrap_or(DEFAULT_REVISION);
    let url = format!(
        "{}/models/{}/tree/{}",
        HF_API_BASE,
        encode_path(repo_id),
        encode_path(rev),
    );

    let response = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .call()
        .map_err(|e| {
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
        return Err(ModelFetchError::RateLimit(format!(
            "HF API rate limit on {}",
            url
        )));
    }
    if status != 200 {
        return Err(ModelFetchError::HfApiError(format!(
            "HTTP {} from {}",
            status, url
        )));
    }

    let entries: Vec<HfTreeEntry> = response
        .into_json()
        .map_err(|e| ModelFetchError::HfApiError(e.to_string()))?;

    // The HF tree endpoint does not paginate — large repos may be silently
    // truncated.  Warn when the count looks suspiciously like a page limit.
    if entries.len() >= 1000 {
        eprintln!(
            "[model_fetcher] WARNING: repo {repo_id} returned {} entries — \
             listing may be truncated",
            entries.len()
        );
    }

    let files = entries
        .into_iter()
        .filter_map(|entry| {
            // Skip directories (no size) and non-GGUF files.
            let size = entry.size.or_else(|| entry.lfs.as_ref().and_then(|l| l.size))?;
            let sha256 = entry
                .lfs
                .as_ref()
                .and_then(|l| l.sha256.clone().or_else(|| l.oid.clone()));

            Some(HfFile {
                rfilename: entry.rfilename,
                size: Some(size),
                sha256,
            })
        })
        .collect();

    Ok(files)
}

/// Return the CDN download URL for a file in a specific repo+revision.
pub fn file_download_url(repo_id: &str, revision: Option<&str>, filename: &str) -> String {
    let rev = revision.unwrap_or(DEFAULT_REVISION);
    format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        encode_path(repo_id),
        encode_path(rev),
        encode_path(filename),
    )
}
