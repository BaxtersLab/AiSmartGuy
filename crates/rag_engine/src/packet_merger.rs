use crate::packet::{MergedPacketSet, RagPacket};

/// Sort packets by packet_id ascending (deterministic, no ties allowed in practice).
/// Then apply conflict resolution and return a MergedPacketSet.
pub fn merge_packets(mut packets: Vec<RagPacket>) -> MergedPacketSet {
    // Sort ascending by packet_id — strictly numeric, no reordering
    packets.sort_by_key(|p| p.packet_id);
    resolve_conflicts(&mut packets);
    MergedPacketSet { packets }
}

/// Resolve rule_id conflicts across packets.
/// Higher packet_id wins for duplicate rule_ids.
/// Fusion model packets (packet_id >= 900) never override lower-id packets.
pub fn resolve_conflicts(packets: &mut Vec<RagPacket>) {
    use std::collections::HashMap;

    // Build a map: rule_id → winning packet_id
    let mut winners: HashMap<String, u16> = HashMap::new();

    for packet in packets.iter() {
        for rule in &packet.rules {
            let entry = winners.entry(rule.rule_id.clone()).or_insert(packet.packet_id);
            // Fusion packets (>= 900) never override reasoning model packets
            if packet.packet_id >= 900 && *entry < 900 {
                continue;
            }
            // Higher packet_id wins otherwise
            if packet.packet_id > *entry {
                *entry = packet.packet_id;
            }
        }
    }

    // Remove losing rules from packets
    for packet in packets.iter_mut() {
        packet.rules.retain(|rule| {
            winners.get(&rule.rule_id).copied() == Some(packet.packet_id)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{
        HitConditions, InjectLocation, ModelBehavior, PatternType, RagRule, Severity,
    };
    use std::path::PathBuf;

    fn make_packet(id: u16, rule_id: &str) -> RagPacket {
        RagPacket {
            packet_id: id,
            category: "test".to_string(),
            description: "".to_string(),
            version: "1.0".to_string(),
            rules: vec![RagRule {
                rule_id: rule_id.to_string(),
                name: rule_id.to_string(),
                pattern_type: PatternType::Linguistic,
                patterns: vec!["x".to_string()],
                severity: Severity::Low,
                explanation: "".to_string(),
            }],
            hit_conditions: HitConditions {
                min_pattern_matches: 1,
                confidence_weight: 0.5,
            },
            model_behavior: ModelBehavior {
                inject_as: InjectLocation::SystemPrompt,
                priority: 1,
            },
            source_path: PathBuf::new(),
        }
    }

    #[test]
    fn sorted_by_packet_id() {
        let p3 = make_packet(3, "R-03");
        let p1 = make_packet(1, "R-01");
        let p2 = make_packet(2, "R-02");
        let set = merge_packets(vec![p3, p1, p2]);
        assert_eq!(set.packets[0].packet_id, 1);
        assert_eq!(set.packets[1].packet_id, 2);
        assert_eq!(set.packets[2].packet_id, 3);
    }

    #[test]
    fn higher_id_wins_conflict() {
        let low = make_packet(1, "SAME");
        let high = make_packet(2, "SAME");
        let set = merge_packets(vec![low, high]);
        // rule retained only by packet 2
        let has_rule_in_2 = set.packets.iter().find(|p| p.packet_id == 2)
            .map(|p| !p.rules.is_empty()).unwrap_or(false);
        let has_rule_in_1 = set.packets.iter().find(|p| p.packet_id == 1)
            .map(|p| !p.rules.is_empty()).unwrap_or(false);
        assert!(has_rule_in_2);
        assert!(!has_rule_in_1);
    }
}
