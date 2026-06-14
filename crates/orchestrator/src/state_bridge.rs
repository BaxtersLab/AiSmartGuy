use std::path::PathBuf;
use model_loader::{
    load_model, run_inference, unload_model, gguf_context_length, gguf_block_count,
    auto_gpu_layers, query_vram_mb, inference_timeout_for,
    ModelConfig as LoaderModelConfig, ModelInstance, InferenceRequest,
};
use manifest::ModelConfig as ManifestModelConfig;
use crate::errors::{OrchestratorError, OrchestratorResult};

/// Build a `model_loader::ModelInstance` from a manifest `ModelConfig`.
///
/// Reads the GGUF file to:
///   1. Detect native context length and cap `--ctx-size` accordingly.
///   2. Auto-calculate `--n-gpu-layers` based on model size vs available VRAM.
///      Layers that don't fit in VRAM are offloaded to system RAM.
///
/// `throttle_pct` (25–100) scales how much hardware to use: VRAM budget and
/// CPU thread count are both multiplied by this percentage.
pub fn make_instance(config: &ManifestModelConfig, throttle_pct: u32) -> ModelInstance {
    let gpu_setting = config.gpu_usage.clone().unwrap_or_else(|| "CPU".to_string());
    let user_ctx = config.context_length.unwrap_or(16384);
    let throttle = throttle_pct.clamp(25, 100);

    let model_path = std::path::Path::new(&config.path);

    // Determine effective context first (needed for KV-cache VRAM reservation).
    let ctx = match gguf_context_length(model_path) {
        Some(native) => {
            let capped = user_ctx.min(native);
            if capped < user_ctx {
                eprintln!(
                    "[state_bridge] capping ctx: user={} model_native={} → effective={}",
                    user_ctx, native, capped
                );
            }
            capped
        }
        None => user_ctx,
    };

    // Auto-calculate GPU layers from model size vs throttled VRAM budget.
    let n_gpu_layers = if gpu_setting.eq_ignore_ascii_case("cpu") {
        0
    } else {
        let raw_vram = query_vram_mb();
        let usable_vram = (raw_vram as u64 * throttle as u64 / 100) as u32;
        eprintln!(
            "[state_bridge] throttle={}% → VRAM budget {} of {} MB",
            throttle, usable_vram, raw_vram
        );
        let (fit, total) = auto_gpu_layers(model_path, usable_vram, ctx);
        eprintln!(
            "[state_bridge] auto gpu_layers: {}/{} (budget={} MB)",
            fit, total, usable_vram
        );
        fit
    };

    // CPU threads: scale logical cores by throttle %.
    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let threads = (logical_cores * throttle / 100).max(1);
    eprintln!("[state_bridge] throttle={}% → threads={}/{}", throttle, threads, logical_cores);

    let loader_config = LoaderModelConfig {
        model_path: PathBuf::from(&config.path),
        context_length: ctx,
        gpu_setting,
    };
    let mut inst = ModelInstance::new(loader_config, n_gpu_layers);
    inst.threads = threads;
    inst
}

/// Load model — wraps `model_loader::load_model`.
pub fn load(instance: &mut ModelInstance) -> OrchestratorResult<()> {
    load_model(instance).map_err(|e| OrchestratorError::ModelLoadFailed(e.to_string()))
}

/// Run inference on one chunk — wraps `model_loader::run_inference`.
///
/// Timeout is computed dynamically: more CPU-offloaded layers and larger
/// context windows get proportionally longer timeouts.
pub fn infer(
    instance: &mut ModelInstance,
    request: &InferenceRequest,
) -> OrchestratorResult<()> {
    let total_layers = gguf_block_count(&instance.model_path).unwrap_or(48);
    let timeout = inference_timeout_for(instance.n_gpu_layers, total_layers, instance.context_length);
    run_inference(instance, request, timeout)
        .map_err(|e| OrchestratorError::InferenceFailed(e.to_string()))
}

/// Unload model — wraps `model_loader::unload_model`.
pub fn unload(instance: &mut ModelInstance) -> OrchestratorResult<()> {
    unload_model(instance).map_err(|e| OrchestratorError::ModelLoadFailed(e.to_string()))
}
