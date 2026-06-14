/// Integration Tests — RAG Engine + Orchestrator (Module M §4.1)
///
/// Tests:
/// - packet loading via `RagEngine::load_packets`
/// - packet merging
/// - prompt generation
/// - malformed packet recovery (empty directory = no packets, run continues)

use std::path::PathBuf;
use rag_engine::{
    RagEngine, RagPacket, RagRule, HitConditions, ModelBehavior,
    InjectLocation, PatternType, Severity, MergedPacketSet,
};

fn make_packet(id: u16, category: &str) -> RagPacket {
    RagPacket {
        packet_id: id,
        category: category.to_string(),
        description: format!("test packet {}", id),
        version: "1.0.0".to_string(),
        rules: vec![RagRule {
            rule_id: format!("R{}", id),
            name: format!("rule-{}", id),
            pattern_type: PatternType::Linguistic,
            patterns: vec!["test".to_string()],
            severity: Severity::Medium,
            explanation: "integration test rule".to_string(),
        }],
        hit_conditions: HitConditions {
            min_pattern_matches: 1,
            confidence_weight: 0.9,
        },
        model_behavior: ModelBehavior {
            inject_as: InjectLocation::SystemPrompt,
            priority: 5,
        },
        source_path: PathBuf::new(),
    }
}

// ---------------------------------------------------------------------------
// 4.1.1 — Packet merge produces output for all packets
// ---------------------------------------------------------------------------
#[test]
fn test_rag_packet_merge_all_packets_present() {
    let packets = vec![
        make_packet(1, "grammar"),
        make_packet(2, "style"),
        make_packet(3, "factual"),
    ];

    let merged = RagEngine::merge_packets(packets);
    assert_eq!(merged.packets.len(), 3, "all 3 packets should survive merge");
}

// ---------------------------------------------------------------------------
// 4.1.2 — Prompt generation produces a non-empty string
// ---------------------------------------------------------------------------
#[test]
fn test_rag_prompt_generation_non_empty() {
    let packets = vec![make_packet(1, "style"), make_packet(2, "grammar")];
    let merged = RagEngine::merge_packets(packets);
    let prompt = RagEngine::build_prompt(&merged);
    assert!(!prompt.is_empty(), "prompt must not be empty");
}

// ---------------------------------------------------------------------------
// 4.1.3 — Empty packet list merges cleanly (no panic / recovery path)
// ---------------------------------------------------------------------------
#[test]
fn test_rag_merge_empty_list_does_not_panic() {
    let merged = RagEngine::merge_packets(vec![]);
    assert_eq!(merged.packets.len(), 0);
}

// ---------------------------------------------------------------------------
// 4.1.4 — Prompt from empty set is empty or a sensible fallback (no panic)
// ---------------------------------------------------------------------------
#[test]
fn test_rag_prompt_empty_set_no_panic() {
    let empty_set = MergedPacketSet { packets: vec![] };
    let prompt = RagEngine::build_prompt(&empty_set);
    // We only require it does not panic; content may be empty.
    let _ = prompt;
}

// ---------------------------------------------------------------------------
// 4.1.5 — Duplicate packet IDs survive merge without panic
// ---------------------------------------------------------------------------
#[test]
fn test_rag_merge_duplicate_ids_no_panic() {
    let packets = vec![make_packet(5, "style"), make_packet(5, "grammar")];
    let merged = RagEngine::merge_packets(packets);
    // Must not panic; exact dedup policy is implementation-defined.
    assert!(!merged.packets.is_empty());
}

// ---------------------------------------------------------------------------
// 4.1.6 — load_packets from a non-existent directory returns an error,
//           not a panic — simulating "missing model dir" recovery path.
// ---------------------------------------------------------------------------
#[test]
fn test_rag_load_packets_missing_dir_returns_error() {
    let bad_path = PathBuf::from("/does/not/exist/anywhere");
    let result = RagEngine::load_packets(&bad_path);
    assert!(result.is_err(), "missing dir should return Err, not panic");
}
