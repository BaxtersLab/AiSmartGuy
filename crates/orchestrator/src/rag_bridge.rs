use std::collections::HashMap;
use std::path::{Path, PathBuf};
use rag_engine::{RagEngine, RagPacket};
use crate::errors::OrchestratorResult;

/// Resolve the bundled RAG defaults directory at `~/.aismartguy/rag_defaults/`.
fn defaults_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let home = std::env::var("USERPROFILE").ok()?;
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join(".aismartguy").join("rag_defaults");
    if dir.is_dir() { Some(dir) } else { None }
}

/// Resolve a model's own RAG packet folder from its configured path.
///
/// `ModelConfig::path` points at the **`.gguf` file**, not the folder holding
/// it. Joining `rag` straight onto it yields `…/model.gguf/rag`, which can
/// never exist, so model-specific packets silently never loaded and every
/// model got only the shared defaults — with no error to say so. Take the
/// parent when the path names a file.
pub fn model_rag_dir(model_path: &str) -> PathBuf {
    let p = Path::new(model_path);
    let base = if p.is_dir() {
        p
    } else if p.extension().is_some() || p.is_file() {
        p.parent().unwrap_or(p)
    } else {
        p
    };
    base.join("rag")
}

/// Load the shared defaults plus a model's own overrides and merge them.
///
/// Priority: model-specific packets win over shared defaults for the same
/// `rule_id` (higher packet_id takes precedence in the merger).
pub fn load_merged_packets(model_rag_dir: &Path) -> Vec<RagPacket> {
    let mut all_packets = Vec::new();

    // 1. Shared defaults (low-priority baseline).
    if let Some(def_dir) = defaults_dir() {
        if let Ok(pkts) = RagEngine::load_packets(&def_dir) {
            all_packets.extend(pkts);
        }
    }

    // 2. Model-specific overrides (win in merger).
    if let Ok(pkts) = RagEngine::load_packets(model_rag_dir) {
        all_packets.extend(pkts);
    }

    if all_packets.is_empty() {
        return Vec::new();
    }

    RagEngine::merge_packets(all_packets).packets
}

/// Bridge: load and merge RAG packets for a model, returning the system prompt
/// at full detail.
///
/// Prefer [`load_merged_packets`] plus `rag_plan::plan_rag` when the prompt has
/// to fit a known context — this renders the whole library unconditionally and
/// says nothing about whether the result fits.
pub fn build_system_prompt(model_rag_dir: &PathBuf) -> OrchestratorResult<String> {
    let packets = load_merged_packets(model_rag_dir);
    if packets.is_empty() {
        return Ok(String::new());
    }
    Ok(rag_engine::build_system_prompt_with_detail(
        &packets,
        rag_engine::PromptDetail::Full,
    ))
}

/// Build a `rule_id → category` map from the SAME packets used for the system
/// prompt (shared defaults + model-specific overrides, merged the same way).
/// Used by structured-findings scoring to convert per-rule finding counts into
/// per-category counts. Empty map if no packets are available.
pub fn build_rule_category_map(model_rag_dir: &PathBuf) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for packet in load_merged_packets(model_rag_dir) {
        for rule in &packet.rules {
            map.insert(rule.rule_id.clone(), packet.category.clone());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this replaced: `path.join("rag")` on a `.gguf` **file** produced
    /// `…/model.gguf/rag`, which cannot exist, so per-model packets were
    /// unreachable and the failure was silent.
    #[test]
    fn rag_dir_resolves_beside_the_gguf_not_inside_it() {
        let dir = model_rag_dir("/models/llama-3-8b/llama-3-8b.Q5_0.gguf");
        assert_eq!(dir, PathBuf::from("/models/llama-3-8b/rag"));
        assert!(
            !dir.to_string_lossy().contains(".gguf"),
            "rag dir must not be nested inside the model file"
        );
    }

    #[test]
    fn rag_dir_accepts_a_directory_path_unchanged() {
        // No extension and not an existing file → treated as the model folder.
        let dir = model_rag_dir("/models/llama-3-8b");
        assert_eq!(dir, PathBuf::from("/models/llama-3-8b/rag"));
    }

    #[test]
    fn rag_dir_handles_a_bare_filename() {
        let dir = model_rag_dir("model.gguf");
        assert_eq!(dir, PathBuf::from("rag"));
    }
}
