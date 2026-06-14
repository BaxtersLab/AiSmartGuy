use crate::errors::{LoaderResult, ModelError};
use crate::state_machine::transition;
use crate::types::{ModelInstance, ModelState};

/// Validates the model path and drives the instance into `Loaded` state.
///
/// Transition: `Unloaded → Loading → Loaded`
pub fn load_model(instance: &mut ModelInstance) -> LoaderResult<()> {
    transition(instance, ModelState::Loading)?;

    eprintln!("[model_loader][INFO] loading model: {} (ctx={}, gpu_layers={})",
        instance.model_path.display(), instance.context_length, instance.n_gpu_layers);

    // Validate model path exists.
    if !instance.model_path.exists() {
        let msg = format!("model file not found: {}", instance.model_path.display());
        transition(instance, ModelState::Error(ModelError::LoadFailure(msg.clone())))?;
        return Err(ModelError::LoadFailure(msg));
    }

    // Transition to Loaded. The actual llama subprocess is spawned per-inference
    // request in inferencer.rs — no persistent process is held during Loaded state.
    transition(instance, ModelState::Loaded)?;

    eprintln!("[model_loader][INFO] model loaded successfully");
    Ok(())
}

/// Drives the instance into `Unloaded` state. Kills any running subprocess first.
///
/// Transition: `Loaded | Inferencing | Error → Unloading → Unloaded`
/// This function is designed to always succeed.
pub fn unload_model(instance: &mut ModelInstance) -> LoaderResult<()> {
    // Kill any running subprocess.
    if let Some(ref mut child) = instance.child {
        crate::process::kill_process(child);
    }
    instance.child = None;

    // Force into Unloading first (tolerate Error state).
    if !matches!(instance.state, ModelState::Unloading | ModelState::Unloaded) {
        // Best-effort: if transition fails (e.g. already Unloaded) that's fine.
        let _ = transition(instance, ModelState::Unloading);
    }

    transition(instance, ModelState::Unloaded)?;

    eprintln!("[model_loader][INFO] model unloaded");
    Ok(())
}
