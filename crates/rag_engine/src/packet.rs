use std::path::PathBuf;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// PatternType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    Linguistic,
    Semantic,
    Contextual,
}

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

// ---------------------------------------------------------------------------
// InjectLocation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectLocation {
    SystemPrompt,
    PrePrompt,
    PostPrompt,
}

// ---------------------------------------------------------------------------
// HitConditions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitConditions {
    pub min_pattern_matches: u8,
    pub confidence_weight: f32,
}

// ---------------------------------------------------------------------------
// ModelBehavior
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBehavior {
    pub inject_as: InjectLocation,
    pub priority: u8,
}

// ---------------------------------------------------------------------------
// RagRule
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagRule {
    pub rule_id: String,
    pub name: String,
    pub pattern_type: PatternType,
    pub patterns: Vec<String>,
    pub severity: Severity,
    pub explanation: String,
}

// ---------------------------------------------------------------------------
// RagPacket
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagPacket {
    pub packet_id: u16,
    pub category: String,
    pub description: String,
    pub version: String,
    pub rules: Vec<RagRule>,
    pub hit_conditions: HitConditions,
    pub model_behavior: ModelBehavior,
    #[serde(skip)]
    pub source_path: PathBuf,
}

// ---------------------------------------------------------------------------
// MergedPacketSet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MergedPacketSet {
    pub packets: Vec<RagPacket>,
}
