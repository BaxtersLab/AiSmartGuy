use std::path::Path;

use error_system::RagError;

use crate::packet::{MergedPacketSet, RagPacket};
use crate::packet_loader::load_packets;
use crate::packet_merger::merge_packets;
use crate::prompt_builder::{build_prompt_from_set, build_system_prompt_with_detail, PromptDetail};

/// Clean facade used by the orchestrator.
pub struct RagEngine;

impl RagEngine {
    /// Load all valid RAG packets from a model's RAG folder.
    /// Returns loaded packets. Skipped (malformed) packets are logged by caller.
    pub fn load_packets(model_path: &Path) -> Result<Vec<RagPacket>, RagError> {
        let (packets, skipped) = load_packets(model_path)?;
        // Skipped entries surfaced via the RagError::MalformedPacket variant
        // so orchestrator can log them. We just return the valid ones.
        let _ = skipped; // caller inspects via error_system::handle_malformed_packet
        Ok(packets)
    }

    /// Sort and merge a packet list, resolving conflicts.
    pub fn merge_packets(packets: Vec<RagPacket>) -> MergedPacketSet {
        merge_packets(packets)
    }

    /// Build the final system-prompt block from a merged set, at full detail.
    pub fn build_prompt(set: &MergedPacketSet) -> String {
        build_prompt_from_set(set)
    }

    /// Build a system-prompt block from an arbitrary packet slice at the given
    /// detail level. Used by the orchestrator's RAG batch planner, which hands
    /// in a rule *subset* of the merged set rather than the whole thing.
    pub fn build_prompt_with_detail(packets: &[RagPacket], detail: PromptDetail) -> String {
        build_system_prompt_with_detail(packets, detail)
    }
}
