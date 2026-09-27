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
            // Shared process-wide flag (crate::cancel) — NOT a fresh
            // AtomicBool. A run is one llama child at a time, so every
            // instance sharing one flag is what makes cmd_cancel_run's
            // write visible to whichever instance is actually inferencing.
            cancel: crate::cancel::flag(),
        }
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Regression test for the 2026-08-04 Terminate-Run bug: `ModelInstance::new`
    /// used to mint a fresh `Arc<AtomicBool>` per instance, so a cancel flag set
    /// through one instance (e.g. the UI's handle) was never visible to the
    /// instance actually running inference. Two instances must share the SAME
    /// flag object so a cancel request set anywhere is observed everywhere.
    #[test]
    fn model_instances_share_the_cancel_flag() {
        // This writes the process-wide flag: hold the crate's cancel lock
        // and leave the flag cleared, or it races crate::cancel's tests.
        let _guard = crate::cancel::test_lock();
        let cfg = ModelConfig {
            model_path: PathBuf::from("does-not-need-to-exist.gguf"),
            context_length: 4096,
            gpu_setting: "CPU".to_string(),
        };
        let a = ModelInstance::new(cfg.clone(), 0);
        let b = ModelInstance::new(cfg, 0);
        assert!(
            Arc::ptr_eq(&a.cancel, &b.cancel),
            "ModelInstance::new must hand out the same shared cancel flag \
             every time — a fresh AtomicBool per instance means cmd_cancel_run \
             can set a flag no running inference ever reads"
        );

        // The property that actually matters: a value set through one
        // instance's handle must be visible through the other's.
        a.cancel.store(true, Ordering::SeqCst);
        let seen = b.cancel.load(Ordering::SeqCst);
        crate::cancel::clear();
        assert!(seen, "cancelling via instance A's flag must be observed by instance B");
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
    /// When set, constrain generation to this GBNF grammar (Phase 2 structured
    /// findings) — emitted as `--grammar-file`. `None` = free-form prose (default).
    pub grammar_file: Option<PathBuf>,
}
