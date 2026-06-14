use std::path::{Path, PathBuf};

use crate::state::ModelFetchState;
use crate::types::{ModelFetchRequest, ModelShard};

/// Root of the local AiSmartGuy model cache.
///
/// Resolves to `~/.aismartguy/models/` (cross-platform).
pub fn cache_root() -> PathBuf {
    let home = dirs_next_home();
    home.join(".aismartguy").join("models")
}

/// The directory for a specific repo+revision.
///
/// Layout: `<cache_root>/<repo_id>/<revision>/`
/// The repo_id slashes are kept as path separators, e.g.
/// `~/.aismartguy/models/TheBloke/Mistral-7B.../main/`.
pub fn model_cache_dir(request: &ModelFetchRequest) -> PathBuf {
    let rev = request
        .revision
        .as_deref()
        .unwrap_or("main");
    cache_root()
        .join(&request.repo_id)
        .join(rev)
}

/// Inspect the cache for a given request.
///
/// Returns `Some(state)` only if ALL shards are already fully downloaded.
/// Returns `None` if any shard is missing or incomplete.
pub fn check_cache(request: &ModelFetchRequest) -> Option<ModelFetchState> {
    let dir = model_cache_dir(request);
    if !dir.exists() {
        return None;
    }

    // We need the shard list — look for any *.gguf* files already on disk.
    let mut shards: Vec<ModelShard> = Vec::new();
    let rd = std::fs::read_dir(&dir).ok()?;
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".gguf") || contains_gguf_shard_ext(n))
        .collect();
    names.sort(); // deterministic order

    if names.is_empty() {
        return None;
    }

    for name in &names {
        let path = dir.join(name);
        // If a .part file exists, the shard is incomplete.
        let part_path = part_path_of(&path);
        if part_path.exists() {
            return None; // incomplete download present
        }
        let size = std::fs::metadata(&path).ok()?.len();
        shards.push(ModelShard {
            filename: name.clone(),
            size,
            sha256: None, // loaded from disk — verification done separately
            downloaded: true,
        });
    }

    let total = shards.iter().map(|s| s.size).sum();
    Some(ModelFetchState {
        downloaded_bytes: total,
        total_bytes: total,
        complete: true,
        shards,
    })
}

/// Compute the `.part` path for an in-progress download file.
pub fn part_path_of(final_path: &Path) -> PathBuf {
    let mut p = final_path.to_path_buf();
    let ext = format!(
        "{}.part",
        p.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
    );
    p.set_extension(ext);
    p
}

// ---- platform home dir helper ----------------------------------------------

fn dirs_next_home() -> PathBuf {
    // Prefer USERPROFILE / HOME env vars rather than pulling in `dirs` crate.
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    // Fallback: current directory (should never happen in practice)
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn contains_gguf_shard_ext(name: &str) -> bool {
    // Matches model.gguf.1, model.gguf.2, etc.
    if let Some(dot) = name.rfind('.') {
        let suffix = &name[dot + 1..];
        if suffix.parse::<u32>().is_ok() {
            return name[..dot].ends_with(".gguf");
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_gguf_shard_ext_matches() {
        assert!(contains_gguf_shard_ext("model.gguf.1"));
        assert!(contains_gguf_shard_ext("big_model.gguf.12"));
        assert!(!contains_gguf_shard_ext("model.gguf"));
        assert!(!contains_gguf_shard_ext("model.bin.1"));
    }

    #[test]
    fn part_path_appends_correctly() {
        let p = Path::new("/tmp/model.gguf");
        let pp = part_path_of(p);
        assert!(pp.to_string_lossy().ends_with(".gguf.part"));
    }
}
