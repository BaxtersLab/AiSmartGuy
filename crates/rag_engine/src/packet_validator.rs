use error_system::RagError;

use crate::packet::RagPacket;

/// Validate a packet's required fields. Returns an error describing the first
/// violation found. Non-fatal — callers should log and skip on failure.
pub fn validate_packet(packet: &RagPacket) -> Result<(), RagError> {
    if packet.rules.is_empty() {
        return Err(RagError::ValidationFailure(format!(
            "packet {} has no rules",
            packet.packet_id
        )));
    }

    // rule_id must be unique within the packet
    let mut seen_ids = std::collections::HashSet::new();
    for rule in &packet.rules {
        if rule.rule_id.is_empty() {
            return Err(RagError::ValidationFailure(format!(
                "packet {} has a rule with empty rule_id",
                packet.packet_id
            )));
        }
        if !seen_ids.insert(rule.rule_id.clone()) {
            return Err(RagError::ValidationFailure(format!(
                "packet {} has duplicate rule_id: {}",
                packet.packet_id, rule.rule_id
            )));
        }
        if rule.patterns.is_empty() {
            return Err(RagError::ValidationFailure(format!(
                "packet {} rule {} has no patterns",
                packet.packet_id, rule.rule_id
            )));
        }
    }

    // hit_conditions sanity
    if packet.hit_conditions.confidence_weight <= 0.0 {
        return Err(RagError::ValidationFailure(format!(
            "packet {} has non-positive confidence_weight",
            packet.packet_id
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{HitConditions, InjectLocation, ModelBehavior, PatternType, RagRule, Severity};
    use std::path::PathBuf;

    fn good_packet() -> RagPacket {
        RagPacket {
            packet_id: 1,
            category: "fallacies".to_string(),
            description: "test".to_string(),
            version: "1.0".to_string(),
            rules: vec![RagRule {
                rule_id: "FAL-01".to_string(),
                name: "Strawman".to_string(),
                pattern_type: PatternType::Linguistic,
                patterns: vec!["misrepresent".to_string()],
                severity: Severity::High,
                explanation: "A strawman argument".to_string(),
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
    fn valid_packet_passes() {
        assert!(validate_packet(&good_packet()).is_ok());
    }

    #[test]
    fn empty_rules_fails() {
        let mut p = good_packet();
        p.rules.clear();
        assert!(validate_packet(&p).is_err());
    }

    #[test]
    fn empty_patterns_fails() {
        let mut p = good_packet();
        p.rules[0].patterns.clear();
        assert!(validate_packet(&p).is_err());
    }

    #[test]
    fn duplicate_rule_id_fails() {
        let mut p = good_packet();
        let dup = p.rules[0].clone();
        p.rules.push(dup);
        assert!(validate_packet(&p).is_err());
    }
}
