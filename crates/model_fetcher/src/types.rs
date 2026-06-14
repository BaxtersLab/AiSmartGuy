use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Caller-supplied request: which repo to fetch and where to store it.
#[derive(Debug, Clone)]
pub struct ModelFetchRequest {
    /// Hugging Face repo id, e.g. "TheBloke/Mistral-7B-Instruct-v0.2-GGUF"
    pub repo_id: String,
    /// Specific commit hash or tag; None means latest main branch.
    pub revision: Option<String>,
    /// Root directory for the local model cache.
    pub target_dir: PathBuf,
}

/// A single GGUF shard (or the only file when the model is not sharded).
#[derive(Debug, Clone)]
pub struct ModelShard {
    pub filename: String,
    pub size: u64,
    pub sha256: Option<String>,
    pub downloaded: bool,
}

/// One file entry as returned by the HF repo API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfFile {
    /// Relative path inside the repo, e.g. "model.gguf" or "model.gguf.1"
    pub rfilename: String,
    /// File size in bytes (may be None for LFS pointer files).
    #[serde(rename = "size")]
    pub size: Option<u64>,
    /// SHA-256 hex string from the HF API's "lfs" metadata, if present.
    pub sha256: Option<String>,
}

/// UI progress event emitted after each chunk write.
#[derive(Debug, Clone)]
pub struct FetchProgressEvent {
    pub filename: String,
    pub percent: f32,
    pub total_percent: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}
