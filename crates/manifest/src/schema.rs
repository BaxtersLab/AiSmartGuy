use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// RunMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Single,
    Dual,
    Full,
    Optimized,
}

impl Default for RunMode {
    fn default() -> Self {
        RunMode::Full
    }
}

// ---------------------------------------------------------------------------
// SourcePdf
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePdf {
    pub filename: String,
    pub hash_sha256: Option<String>,
    pub page_count: Option<u32>,
}

impl Default for SourcePdf {
    fn default() -> Self {
        SourcePdf {
            filename: String::new(),
            hash_sha256: None,
            page_count: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ModelConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub path: String,
    pub quantization: String,
    pub context_length: Option<u32>,
    pub gpu_usage: Option<String>,
    pub n_gpu_layers: Option<u32>,
    pub active: bool,
    // model_fetcher fields (Phase 4)
    pub revision: Option<String>,
    pub sha256: Option<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            name: String::new(),
            path: String::new(),
            quantization: String::new(),
            context_length: None,
            gpu_usage: Some("CPU".to_string()),
            n_gpu_layers: None,
            active: false,
            revision: None,
            sha256: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ModelSet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelSet {
    pub model1: Option<ModelConfig>,
    pub model2: Option<ModelConfig>,
    pub model3: Option<ModelConfig>,
    pub fusion: Option<ModelConfig>,
}

// ---------------------------------------------------------------------------
// RagPacketMap  —  "model1": ["001", "002", ...]
// ---------------------------------------------------------------------------

pub type RagPacketMap = HashMap<String, Vec<String>>;

// ---------------------------------------------------------------------------
// OptimizationState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationState {
    pub books_completed: u32,
    pub optimized_available: bool,
    pub optimized_enabled: bool,
    pub best_model_per_category: Option<HashMap<String, String>>,
}

impl Default for OptimizationState {
    fn default() -> Self {
        OptimizationState {
            books_completed: 0,
            optimized_available: false,
            optimized_enabled: false,
            best_model_per_category: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ResourceThrottle
// ---------------------------------------------------------------------------

/// Percentage of hardware resources (GPU VRAM + CPU threads) the app is
/// allowed to use.  75 = use ~75% of available resources.  Lower values
/// protect the hardware at the cost of slower inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceThrottle {
    pub throttle_pct: u32,
}

impl Default for ResourceThrottle {
    fn default() -> Self {
        ResourceThrottle { throttle_pct: 75 }
    }
}

// ---------------------------------------------------------------------------
// PartialRunInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartialRunInfo {
    pub model_failures: Vec<String>,
    pub failed_chunks: Vec<String>,
    pub fusion_partial: bool,
}

// ---------------------------------------------------------------------------
// Manifest (top-level)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_version: String,
    pub engine_version: String,
    pub run_id: String,
    pub timestamp: String,

    pub source_pdf: SourcePdf,
    pub mode: RunMode,

    pub models: ModelSet,
    pub rag_packets_used: RagPacketMap,
    pub categories_active: Vec<String>,

    pub optimization_state: OptimizationState,
    pub resource_throttle: ResourceThrottle,

    pub partial_run: Option<PartialRunInfo>,
    pub notes: Option<String>,
}
