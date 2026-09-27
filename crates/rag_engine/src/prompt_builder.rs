use crate::packet::{MergedPacketSet, RagPacket};

/// How much of each rule is rendered into the system prompt.
///
/// A packet library that does not fit the model's context is the normal case
/// on consumer hardware, not an error: the shipped default library renders to
/// ~20.8k tokens, more than twice an 8K model's entire window.
///
/// `Compact` drops the per-rule `explanation` — 54% of the rendered bytes in
/// the default library — while keeping every rule id, name, severity and
/// pattern list. Rule *coverage* is therefore identical; only the prose gloss
/// is gone, so the model still knows what to look for and can still name what
/// it found by rule id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDetail {
    /// Every field, including explanations. Preferred whenever it fits.
    Full,
    /// Explanations omitted. Same rule coverage at roughly 46% of the size.
    Compact,
}

/// Strip control characters (except `\n`) from a string to prevent
/// injection of terminal/protocol escape sequences into LLM prompts.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .collect()
}

/// Build a system-prompt block from a merged packet set, at full detail.
/// Packets are iterated in their sorted (numeric) order.
pub fn build_system_prompt(packets: &[RagPacket]) -> String {
    build_system_prompt_with_detail(packets, PromptDetail::Full)
}

/// Build a system-prompt block at the requested detail level.
///
/// Packets whose rule list is empty render nothing at all — a batching caller
/// splits a packet's rules across several prompts and must not emit a bare
/// header for the batches that carry none of them.
pub fn build_system_prompt_with_detail(packets: &[RagPacket], detail: PromptDetail) -> String {
    let mut parts = Vec::with_capacity(packets.len());

    for packet in packets {
        if packet.rules.is_empty() {
            continue;
        }

        let rules_text: String = packet
            .rules
            .iter()
            .map(|r| {
                let head = format!(
                    "  [{}] {} ({:?})\n  Patterns: {}",
                    sanitize(&r.rule_id),
                    sanitize(&r.name),
                    r.severity,
                    sanitize(&r.patterns.join(", ")),
                );
                match detail {
                    PromptDetail::Full => {
                        format!("{}\n  Explanation: {}", head, sanitize(&r.explanation))
                    }
                    PromptDetail::Compact => head,
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        parts.push(format!(
            "[RAG-PACKET {:03}]\nCategory: {}\n{}\n[/RAG-PACKET]",
            packet.packet_id,
            sanitize(&packet.category),
            rules_text
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
        HitConditions, InjectLocation, ModelBehavior, PatternType, RagPacket, RagRule, Severity,
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

    // ── PromptDetail ────────────────────────────────────────────────────────

    #[test]
    fn compact_keeps_rule_identity_and_patterns() {
        let prompt = build_system_prompt_with_detail(&[sample_packet()], PromptDetail::Compact);
        // Everything needed to detect and *name* a hit survives.
        assert!(prompt.contains("FAL-01"));
        assert!(prompt.contains("Strawman"));
        assert!(prompt.contains("misrepresent"));
        assert!(prompt.contains("High"));
        assert!(prompt.contains("[RAG-PACKET 001]"));
    }

    #[test]
    fn compact_drops_only_the_explanation() {
        let prompt = build_system_prompt_with_detail(&[sample_packet()], PromptDetail::Compact);
        assert!(!prompt.contains("Explanation:"));
        assert!(!prompt.contains("Classic strawman"));
    }

    #[test]
    fn compact_is_smaller_than_full() {
        let p = sample_packet();
        let full = build_system_prompt_with_detail(&[p.clone()], PromptDetail::Full);
        let compact = build_system_prompt_with_detail(&[p], PromptDetail::Compact);
        assert!(
            compact.len() < full.len(),
            "compact {} should be smaller than full {}",
            compact.len(),
            full.len()
        );
    }

    #[test]
    fn full_detail_is_the_default_rendering() {
        let p = sample_packet();
        assert_eq!(
            build_system_prompt(&[p.clone()]),
            build_system_prompt_with_detail(&[p], PromptDetail::Full)
        );
    }

    /// A batched caller hands each prompt only the rules in that batch. Packets
    /// left with no rules must vanish entirely — a bare `[RAG-PACKET nnn]`
    /// header with no rules under it is wasted context and reads to the model
    /// as an empty instruction.
    #[test]
    fn packet_with_no_rules_renders_nothing() {
        let mut empty = sample_packet();
        empty.rules.clear();
        assert_eq!(build_system_prompt(&[empty.clone()]), "");

        // ...and does not leave a stray separator when mixed with a real packet.
        let prompt = build_system_prompt(&[empty, sample_packet()]);
        assert!(prompt.starts_with("[RAG-PACKET 001]"));
        assert_eq!(prompt.matches("[RAG-PACKET").count(), 1);
    }
}
