use std::time::Duration;
use crate::command_builder::build_command;
use crate::errors::{LoaderResult, ModelError};
use crate::process::{kill_process, spawn_process};
use crate::state_machine::transition;
use crate::timeout::enforce_timeout;
use crate::types::{InferenceRequest, ModelInstance, ModelState};

/// Default inference timeout: 10 minutes.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Runs a single inference request against an already-loaded model.
///
/// Transition: `Loaded → Inferencing → Loaded` on success (stays ready
/// for further inferences without reloading).
/// On error or timeout: `Any → Error → Unloaded`.
pub fn run_inference(
    instance: &mut ModelInstance,
    request: &InferenceRequest,
    timeout: Duration,
) -> LoaderResult<()> {
    // Must be in Loaded state.
    if !matches!(instance.state, ModelState::Loaded) {
        return Err(ModelError::InvalidState(format!(
            "run_inference called in invalid state: {:?}",
            instance.state
        )));
    }

    transition(instance, ModelState::Inferencing)?;

    let info = format!("chunk_id={} gpu_layers={} ctx={} prompt={} output={}",
        request.chunk_id, instance.n_gpu_layers, instance.context_length,
        request.prompt_path.display(), request.output_path.display());
    eprintln!("[model_loader][INFO] starting inference {}", info);
    crate::log_callback::emit_log_line(&format!("[inference] {}", info));

    // Ensure output/log parent directories exist.
    if let Some(parent) = request.output_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            let msg = format!("failed to create output dir: {}", e);
            transition(instance, ModelState::Error(ModelError::IoError(msg.clone())))?;
            let _ = crate::loader::unload_model(instance);
            return Err(ModelError::IoError(msg));
        }
    }
    if let Some(parent) = request.log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let cmd = build_command(instance, request);

    // Open output file and redirect llama-cli stdout into it
    // (llama-cli writes generated text to stdout, not to a file).
    let output_file = std::fs::File::create(&request.output_path)
        .map_err(|e| {
            let msg = format!("failed to create output file: {}", e);
            let _ = transition(instance, ModelState::Error(ModelError::IoError(msg.clone())));
            let _ = crate::loader::unload_model(instance);
            ModelError::IoError(msg)
        })?;
    let mut cmd = cmd;
    cmd.stdout(std::process::Stdio::from(output_file));

    let mut child = match spawn_process(instance, cmd) {
        Ok(c) => c,
        Err(e) => {
            let err = ModelError::InferenceFailure(e.to_string());
            let _ = transition(instance, ModelState::Error(err.clone()));
            let _ = crate::loader::unload_model(instance);
            return Err(err);
        }
    };

    // Drain stderr in a background thread line-by-line.
    // Each line is sent to the registered log callback (for live UI display)
    // and also written to the log file for post-mortem analysis.
    let stderr_handle = child.stderr.take();
    let log_path_clone = request.log_path.clone();
    let chunk_id = request.chunk_id;
    let stderr_thread = std::thread::spawn(move || {
        if let Some(stderr) = stderr_handle {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            let mut log_file = std::fs::File::create(&log_path_clone).ok();
            for line in reader.lines() {
                match line {
                    Ok(text) => {
                        // Write to log file
                        if let Some(ref mut f) = log_file {
                            use std::io::Write;
                            let _ = writeln!(f, "{}", text);
                        }
                        // Send to live UI callback
                        crate::log_callback::emit_log_line(&text);
                        // Also echo to eprintln for console debugging
                        eprintln!("[llama-cli][chunk {}] {}", chunk_id, text);
                    }
                    Err(_) => break,
                }
            }
        }
    });

    // Store handle on the instance so unload_model / Drop can always
    // find and kill the process if we panic or get interrupted.
    instance.child = Some(child);

    let result = enforce_timeout(instance.child.as_mut().unwrap(), timeout, &instance.cancel, &request.output_path, &instance.model_path);

    // Always kill/reap if still running after timeout or error.
    if result.is_err() {
        if let Some(ref mut c) = instance.child {
            kill_process(c);
        }
    }

    // Inference finished (success or failure) — clear the handle.
    instance.child = None;

    // Wait for stderr drain thread to finish.
    let _ = stderr_thread.join();

    match result {
        Ok(()) => {
            eprintln!("[model_loader][INFO] inference complete chunk_id={}", request.chunk_id);
            // Return to Loaded so the caller can run more chunks without
            // a full unload→reload cycle.  The subprocess has already exited;
            // Loaded just means "ready for next inference".
            transition(instance, ModelState::Loaded)?;
            Ok(())
        }
        Err(e) => {
            // Try to diagnose the failure from the log file.
            let diagnosis = diagnose_failure(&request.log_path, &request.output_path, instance);
            let enriched = if diagnosis.is_empty() {
                e.clone()
            } else {
                ModelError::InferenceFailure(format!("{} — {}", e, diagnosis))
            };
            eprintln!("[model_loader][ERROR] inference failed chunk_id={}: {}", request.chunk_id, enriched);
            crate::log_callback::emit_log_line(&format!("[error] {}", enriched));
            let _ = transition(instance, ModelState::Error(enriched.clone()));
            let _ = crate::loader::unload_model(instance);
            Err(enriched)
        }
    }
}

/// Read the stderr log and output file to produce a human-readable diagnosis.
fn diagnose_failure(
    log_path: &std::path::Path,
    output_path: &std::path::Path,
    instance: &ModelInstance,
) -> String {
    let output_empty = std::fs::metadata(output_path)
        .map(|m| m.len() == 0)
        .unwrap_or(true);

    let log_text = std::fs::read_to_string(log_path).unwrap_or_default();
    let log_lower = log_text.to_lowercase();

    // GPU out-of-memory (CUDA, ROCm, Vulkan)
    let is_oom = (log_lower.contains("out of memory") || log_lower.contains("oom"))
        && (log_lower.contains("cuda") || log_lower.contains("rocm")
            || log_lower.contains("vulkan") || log_lower.contains("ggml")
            || log_lower.contains("alloc"));
    if is_oom {
        return format!(
            "GPU out of memory. The model + {}K context requires more VRAM than available. \
             Try a smaller model or reduce context size.",
            instance.context_length / 1024
        );
    }

    // Process crashed before loading model (very short log, 0-byte output)
    if output_empty {
        let log_lines = log_text.lines().count();

        // Check for explicit error lines
        for line in log_text.lines() {
            let l = line.to_lowercase();
            if l.contains("error:") {
                return format!("llama-cli error: {}", line.trim());
            }
        }

        // Model file size vs VRAM heuristic — only applies when NOT
        // using partial GPU offload (if n_gpu_layers < total, the model
        // is intentionally split across GPU + system RAM).
        let model_size_mb = std::fs::metadata(&instance.model_path)
            .map(|m| m.len() / (1024 * 1024))
            .unwrap_or(0);
        let vram_mb = crate::gpu_mapper::query_vram_mb() as u64;

        if model_size_mb > 0 && vram_mb > 0 && model_size_mb > vram_mb && instance.n_gpu_layers == 0 {
            return format!(
                "Model file is {:.1}GB but GPU has only {:.1}GB VRAM. \
                 The model is too large for your GPU. Use a smaller quantization (Q4_K_M) \
                 or a smaller parameter model (7-8B).",
                model_size_mb as f64 / 1024.0,
                vram_mb as f64 / 1024.0
            );
        }

        if log_lines <= 6 && model_size_mb > 0 {
            return format!(
                "llama-cli crashed during startup (0-byte output, {} log lines). \
                 Model is {:.1}GB — likely insufficient VRAM for model + context. \
                 Try a smaller model or reduce GPU layers.",
                log_lines,
                model_size_mb as f64 / 1024.0
            );
        }
    }

    String::new()
}
