use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use crate::errors::ModelError;

/// Configuration for a model instance, sourced from the manifest.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Path to the GGUF model file (or first shard).
    pub model_path: PathBuf,
    /// Context window size in tokens.
    pub context_length: u32,
    /// GPU usage mode: "CPU" (all layers on CPU) or "GPU" (auto-calculated
    /// layer split based on model size, available VRAM, and context window).
    pub gpu_setting: String,
}

/// A fully initialized model instance tracked by the state machine.
#[derive(Debug)]
pub struct ModelInstance {
    pub config: ModelConfig,
    pub state: ModelState,
    pub model_path: PathBuf,
    pub context_length: u32,
    pub n_gpu_layers: u32,
    /// Number of CPU threads to use (scaled by resource throttle).
    pub threads: u32,
    /// Handle to a running llama.cpp subprocess, if any.
    pub child: Option<std::process::Child>,
    /// Set to `true` from any thread to cancel the current inference.
    pub cancel: Arc<AtomicBool>,
}

impl ModelInstance {
    pub fn new(config: ModelConfig, n_gpu_layers: u32) -> Self {
        let model_path = config.model_path.clone();
        let context_length = config.context_length;
        Self {
            config,
            state: ModelState::Unloaded,
            model_path,
            context_length,
            n_gpu_layers,
            threads: 4,
            child: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Drop for ModelInstance {
    fn drop(&mut self) {
        // Safety net: if a subprocess is still running when the instance
        // is dropped (panic, early return, etc.), kill it so we never
        // leave orphaned llama.cpp processes consuming GPU/RAM.
        if let Some(ref mut child) = self.child {
            eprintln!("[model_loader][WARN] ModelInstance dropped with live subprocess — killing");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Canonical state machine states for a loaded model.
#[derive(Debug, Clone)]
pub enum ModelState {
    Unloaded,
    Loading,
    Loaded,
    Inferencing,
    Unloading,
    Error(ModelError),
}

/// A single inference request targeting one chunk.
#[derive(Debug, Clone)]
pub struct InferenceRequest {
    pub chunk_id: usize,
    pub prompt_path: PathBuf,
    pub output_path: PathBuf,
    pub log_path: PathBuf,
}
