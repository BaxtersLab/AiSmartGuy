use crate::schema::{
    Manifest, ModelSet, OptimizationState, ResourceThrottle, RunMode, SourcePdf,
};
use utils::time::{generate_run_id, now_iso8601};

pub fn default_manifest() -> Manifest {
    Manifest {
        manifest_version: "1.0".to_string(),
        engine_version: "1.0".to_string(),
        run_id: generate_run_id(),
        timestamp: now_iso8601(),
        source_pdf: SourcePdf::default(),
        mode: RunMode::default(),
        models: ModelSet::default(),
        rag_packets_used: Default::default(),
        categories_active: Vec::new(),
        optimization_state: OptimizationState::default(),
        resource_throttle: ResourceThrottle::default(),
        partial_run: None,
        notes: None,
    }
}
