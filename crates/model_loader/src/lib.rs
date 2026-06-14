// AiSmartGuy — model_loader crate (Phase 5)

pub mod errors;
pub mod types;
pub mod state_machine;
pub mod gpu_mapper;
pub mod command_builder;
pub mod timeout;
pub mod process;
pub mod loader;
pub mod inferencer;
pub mod llama_detect;
pub mod log_callback;

pub use errors::{ModelError, LoaderResult};
pub use types::{ModelConfig, ModelInstance, ModelState, InferenceRequest};
pub use gpu_mapper::query_vram_mb;
pub use command_builder::build_command;
pub use loader::{load_model, unload_model};
pub use inferencer::{run_inference, DEFAULT_TIMEOUT};
pub use timeout::inference_timeout_for;
pub use llama_detect::{auto_gpu_layers, detect_llama, gguf_block_count, gguf_context_length, llama_install_dir, llama_local_path, max_context_for_vram, resolve_llama_path, LLAMA_BIN};
pub use log_callback::{set_log_callback, clear_log_callback};
