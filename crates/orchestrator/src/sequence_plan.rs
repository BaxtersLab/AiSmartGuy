use manifest::{Manifest, RunMode};
use crate::types::SequencePlan;

/// Build the execution sequence from the manifest.
///
/// Rules:
/// - Fusion is always last (if configured and active).
/// - Inactive models are skipped.
/// - RunMode::Single  → only model1 + fusion.
/// - RunMode::Dual    → model1 + model2 + fusion.
/// - RunMode::Full    → model1 + model2 + model3 + fusion.
/// - RunMode::Optimized → same as Full (optimization scoring lives in
///   the optimization crate; at this phase the sequence is the same).
pub fn build_sequence(manifest: &Manifest) -> SequencePlan {
    let mut order: Vec<String> = Vec::new();

    let models = &manifest.models;

    let candidates: Vec<(&str, bool)> = match manifest.mode {
        RunMode::Single => {
            vec![
                ("model1", models.model1.as_ref().map_or(false, |m| m.active)),
            ]
        }
        RunMode::Dual => {
            vec![
                ("model1", models.model1.as_ref().map_or(false, |m| m.active)),
                ("model2", models.model2.as_ref().map_or(false, |m| m.active)),
            ]
        }
        RunMode::Full | RunMode::Optimized => {
            vec![
                ("model1", models.model1.as_ref().map_or(false, |m| m.active)),
                ("model2", models.model2.as_ref().map_or(false, |m| m.active)),
                ("model3", models.model3.as_ref().map_or(false, |m| m.active)),
            ]
        }
    };

    for (name, active) in candidates {
        if active {
            order.push(name.to_string());
        }
    }

    // Fusion always last, if configured and active.
    if models.fusion.as_ref().map_or(false, |m| m.active) {
        order.push("fusion".to_string());
    }

    SequencePlan { model_order: order }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manifest::{ModelConfig, ModelSet, RunMode};
    use manifest::defaults::default_manifest;

    fn active_model(name: &str) -> ModelConfig {
        ModelConfig { name: name.to_string(), active: true, ..Default::default() }
    }

    fn make_manifest(mode: RunMode, models: ModelSet) -> Manifest {
        let mut m = default_manifest();
        m.mode = mode;
        m.models = models;
        m
    }

    #[test]
    fn test_single_mode() {
        let models = ModelSet {
            model1: Some(active_model("m1")),
            fusion: Some(active_model("fusion")),
            ..Default::default()
        };
        let plan = build_sequence(&make_manifest(RunMode::Single, models));
        assert_eq!(plan.model_order, vec!["model1", "fusion"]);
    }

    #[test]
    fn test_full_mode_skips_inactive() {
        let models = ModelSet {
            model1: Some(active_model("m1")),
            model2: Some(ModelConfig { active: false, ..Default::default() }),
            model3: Some(active_model("m3")),
            fusion: Some(active_model("fusion")),
        };
        let plan = build_sequence(&make_manifest(RunMode::Full, models));
        assert_eq!(plan.model_order, vec!["model1", "model3", "fusion"]);
    }

    #[test]
    fn test_no_fusion() {
        let models = ModelSet {
            model1: Some(active_model("m1")),
            ..Default::default()
        };
        let plan = build_sequence(&make_manifest(RunMode::Single, models));
        assert_eq!(plan.model_order, vec!["model1"]);
    }
}
