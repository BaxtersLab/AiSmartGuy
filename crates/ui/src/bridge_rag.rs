use rag_engine::api::RagEngine;
use crate::types::UiConflict;

/// Detect RAG-related conflicts by checking which packet IDs referenced
/// in the manifest are absent from the loaded packets.
///
/// `expected_packet_ids` — string IDs from `manifest.rag_packets_used`
///                          (values of the `HashMap<String, Vec<String>>`).
/// `model_path` — directory containing GGUF shards for the model whose
///                 packets should be loaded.
pub fn detect_rag_conflicts(
    expected_packet_ids: &[String],
    model_path: &std::path::Path,
) -> Vec<UiConflict> {
    let mut conflicts = Vec::new();

    let loaded = match RagEngine::load_packets(model_path) {
        Ok(packets) => packets,
        Err(e) => {
            eprintln!("[ui/bridge_rag] failed to load rag packets: {:?}", e);
            for id in expected_packet_ids {
                conflicts.push(UiConflict::MissingRagPacket { packet_id: id.clone() });
            }
            return conflicts;
        }
    };

    // Build a set of loaded packet_ids as strings for easy comparison.
    let loaded_ids: std::collections::HashSet<String> =
        loaded.iter().map(|p| p.packet_id.to_string()).collect();

    for id in expected_packet_ids {
        if !loaded_ids.contains(id.as_str()) {
            conflicts.push(UiConflict::MissingRagPacket { packet_id: id.clone() });
        }
    }

    conflicts
}
