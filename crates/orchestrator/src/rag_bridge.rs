use std::path::PathBuf;
use rag_engine::RagEngine;
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

/// Bridge: load and merge RAG packets for a model, returning the system prompt.
///
/// Priority: model-specific packets in `model_rag_dir` first, then fall back
/// to the shared defaults in `~/.aismartguy/rag_defaults/`.  If both dirs
/// have a packet for the same `rule_id`, the model-specific one wins (higher
/// packet_id takes precedence in the merger).
pub fn build_system_prompt(model_rag_dir: &PathBuf) -> OrchestratorResult<String> {
    let mut all_packets = Vec::new();

    // 1. Load shared defaults (low-priority baseline)
    if let Some(def_dir) = defaults_dir() {
        if let Ok(pkts) = RagEngine::load_packets(&def_dir) {
            all_packets.extend(pkts);
        }
    }

    // 2. Load model-specific overrides (win in merger)
    if let Ok(pkts) = RagEngine::load_packets(model_rag_dir) {
        all_packets.extend(pkts);
    }

    if all_packets.is_empty() {
        return Ok(String::new());
    }

    let merged = RagEngine::merge_packets(all_packets);
    Ok(RagEngine::build_prompt(&merged))
}
