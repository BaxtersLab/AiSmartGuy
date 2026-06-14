use crate::errors::{LoaderResult, ModelError};
use crate::types::{ModelInstance, ModelState};

/// Attempt to transition `instance` to `next`. Returns `InvalidState` on
/// forbidden transitions. Error→Unloaded is always allowed.
pub fn transition(instance: &mut ModelInstance, next: ModelState) -> LoaderResult<()> {
    let allowed = match (&instance.state, &next) {
        (ModelState::Unloaded, ModelState::Loading) => true,
        (ModelState::Loading, ModelState::Loaded) => true,
        (ModelState::Loaded, ModelState::Inferencing) => true,
        (ModelState::Inferencing, ModelState::Loaded) => true,
        (ModelState::Loaded, ModelState::Unloading) => true,
        (ModelState::Inferencing, ModelState::Unloading) => true,
        (ModelState::Unloading, ModelState::Unloaded) => true,
        // Any state → Error
        (_, ModelState::Error(_)) => true,
        // Error → Unloaded for recovery
        (ModelState::Error(_), ModelState::Unloaded) => true,
        _ => false,
    };

    if !allowed {
        let msg = format!(
            "forbidden state transition: {:?} → {:?}",
            instance.state, next
        );
        return Err(ModelError::InvalidState(msg));
    }

    instance.state = next;
    Ok(())
}

/// Returns true if the instance is in the Error state.
pub fn is_error(instance: &ModelInstance) -> bool {
    matches!(instance.state, ModelState::Error(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModelConfig, ModelInstance, ModelState};
    use std::path::PathBuf;

    fn make_instance() -> ModelInstance {
        let config = ModelConfig {
            model_path: PathBuf::from("/tmp/test.gguf"),
            context_length: 4096,
            gpu_setting: "CPU".to_string(),
        };
        ModelInstance::new(config, 0)
    }

    #[test]
    fn test_valid_full_cycle() {
        let mut inst = make_instance();
        transition(&mut inst, ModelState::Loading).unwrap();
        transition(&mut inst, ModelState::Loaded).unwrap();
        transition(&mut inst, ModelState::Inferencing).unwrap();
        transition(&mut inst, ModelState::Unloading).unwrap();
        transition(&mut inst, ModelState::Unloaded).unwrap();
    }

    #[test]
    fn test_forbidden_unloaded_to_inferencing() {
        let mut inst = make_instance();
        let result = transition(&mut inst, ModelState::Inferencing);
        assert!(result.is_err());
    }

    #[test]
    fn test_error_to_unloaded_allowed() {
        let mut inst = make_instance();
        transition(&mut inst, ModelState::Error(ModelError::LoadFailure("test".into()))).unwrap();
        transition(&mut inst, ModelState::Unloaded).unwrap();
    }

    #[test]
    fn test_any_to_error_allowed() {
        let mut inst = make_instance();
        transition(&mut inst, ModelState::Loading).unwrap();
        transition(&mut inst, ModelState::Error(ModelError::Timeout("t".into()))).unwrap();
        assert!(is_error(&inst));
    }

    #[test]
    fn test_inferencing_to_loaded_allowed() {
        let mut inst = make_instance();
        transition(&mut inst, ModelState::Loading).unwrap();
        transition(&mut inst, ModelState::Loaded).unwrap();
        transition(&mut inst, ModelState::Inferencing).unwrap();
        // After inference completes, instance returns to Loaded for next chunk.
        transition(&mut inst, ModelState::Loaded).unwrap();
        // Can infer again without reloading.
        transition(&mut inst, ModelState::Inferencing).unwrap();
        transition(&mut inst, ModelState::Loaded).unwrap();
    }
}
