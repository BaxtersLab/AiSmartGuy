use crate::packet::{MergedPacketSet, RagPacket};

/// Strip control characters (except `\n`) from a string to prevent
/// injection of terminal/protocol escape sequences into LLM prompts.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .collect()
}

/// Build a system-prompt block from a merged packet set.
/// Packets are iterated in their sorted (numeric) order.
pub fn build_system_prompt(packets: &[RagPacket]) -> String {
    let mut parts = Vec::with_capacity(packets.len());

    for packet in packets {
        let rules_text: String = packet
            .rules
            .iter()
            .map(|r| {
                format!(
                    "  [{}] {} ({:?})\n  Patterns: {}\n  Explanation: {}",
                    sanitize(&r.rule_id),
                    sanitize(&r.name),
                    r.severity,
                    sanitize(&r.patterns.join(", ")),
                    sanitize(&r.explanation)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        parts.push(format!(
            "[RAG-PACKET {:03}]\nCategory: {}\n{}\n[/RAG-PACKET]",
            packet.packet_id, sanitize(&packet.category), rules_text
        ));
    }

    parts.join("\n\n")
}

/// Convenience wrapper that accepts a `MergedPacketSet`.
pub fn build_prompt_from_set(set: &MergedPacketSet) -> String {
    build_system_prompt(&set.packets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{
        HitConditions, InjectLocation, ModelBehavior, PatternType, RagRule, RagPacket, Severity,
    };
    use std::path::PathBuf;

    fn sample_packet() -> RagPacket {
        RagPacket {
            packet_id: 1,
            category: "fallacies".to_string(),
            description: "".to_string(),
            version: "1.0".to_string(),
            rules: vec![RagRule {
                rule_id: "FAL-01".to_string(),
                name: "Strawman".to_string(),
                pattern_type: PatternType::Linguistic,
                patterns: vec!["misrepresent".to_string()],
                severity: Severity::High,
                explanation: "Classic strawman".to_string(),
            }],
            hit_conditions: HitConditions {
                min_pattern_matches: 1,
                confidence_weight: 0.8,
            },
            model_behavior: ModelBehavior {
                inject_as: InjectLocation::SystemPrompt,
                priority: 1,
            },
            source_path: PathBuf::new(),
        }
    }

    #[test]
    fn prompt_contains_packet_header() {
        let prompt = build_system_prompt(&[sample_packet()]);
        assert!(prompt.contains("[RAG-PACKET 001]"));
        assert!(prompt.contains("[/RAG-PACKET]"));
    }

    #[test]
    fn prompt_contains_rule_info() {
        let prompt = build_system_prompt(&[sample_packet()]);
        assert!(prompt.contains("FAL-01"));
        assert!(prompt.contains("Strawman"));
        assert!(prompt.contains("misrepresent"));
    }

    #[test]
    fn empty_packets_returns_empty_string() {
        assert_eq!(build_system_prompt(&[]), "");
    }
}
