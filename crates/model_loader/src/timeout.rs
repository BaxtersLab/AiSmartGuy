use std::path::Path;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use crate::errors::{LoaderResult, ModelError};

/// How long the output file must be unchanged (after gaining content)
/// before we declare inference complete and force-kill the stalled process.
/// llama-cli sometimes hangs during CUDA cleanup after finishing inference.
const STALL_THRESHOLD: Duration = Duration::from_secs(15);

/// Minimum startup timeout (small models / fast disks).
const MIN_STARTUP_SECS: u64 = 120;
/// Extra seconds per GB of model file — large models need much longer to
/// load from disk, allocate GPU/RAM buffers, and build the KV cache.
const SECS_PER_GB: u64 = 15;

/// Compute a generous startup timeout from the model file size.
/// Returns at least [`MIN_STARTUP_SECS`].
fn startup_timeout_for(model_path: &Path) -> Duration {
    let size_gb = std::fs::metadata(model_path)
        .map(|m| m.len() / (1024 * 1024 * 1024))
        .unwrap_or(0);
    let secs = MIN_STARTUP_SECS + size_gb * SECS_PER_GB;
    eprintln!("[timeout] model {:.1} GB → startup timeout {}s", size_gb, secs);
    Duration::from_secs(secs)
}

/// Minimum inference timeout (fully GPU-offloaded, small context).
const MIN_INFERENCE_SECS: u64 = 600;

/// Compute a dynamic inference timeout based on how much work lands on CPU.
///
/// When most layers are on GPU, inference is fast and 10 min is plenty.
/// When most layers are on CPU (partial offload), each token takes much
/// longer; a 24B model with 32/48 layers on CPU can need 30+ minutes per
/// chunk at 32K context.
///
/// Formula:  `base × cpu_ratio × ctx_scale`
///   - `cpu_ratio` = proportion of layers on CPU (1.0 → 5.0 multiplier)
///   - `ctx_scale` = context multiplier (32K → 2×, 8K → 1×)
pub fn inference_timeout_for(n_gpu_layers: u32, total_layers: u32, ctx: u32) -> Duration {
    let base = MIN_INFERENCE_SECS as f64;

    // CPU offload multiplier: 1× when fully on GPU, up to 5× when fully on CPU.
    let cpu_frac = if total_layers == 0 {
        1.0
    } else {
        let on_cpu = total_layers.saturating_sub(n_gpu_layers) as f64;
        on_cpu / total_layers as f64
    };
    let cpu_mult = 1.0 + cpu_frac * 4.0; // 1.0 – 5.0

    // Context scale: 1× at 8K, 2× at 32K, linear.
    let ctx_mult = (ctx as f64 / 8192.0).max(1.0);

    let secs = (base * cpu_mult * ctx_mult) as u64;
    eprintln!(
        "[timeout] inference: gpu_layers={}/{} cpu_frac={:.0}% ctx={}K → timeout {}s ({}m)",
        n_gpu_layers, total_layers, cpu_frac * 100.0, ctx / 1024, secs, secs / 60
    );
    Duration::from_secs(secs)
}

/// Blocks until `child` exits, `timeout` elapses, or output-file stall is
/// detected.
///
/// **Stall detection**: once `output_path` has non-zero size and stops growing
/// for [`STALL_THRESHOLD`] seconds, the child is killed and `Ok(())` is
/// returned — the output is already complete.
pub fn enforce_timeout(
    child: &mut Child,
    timeout: Duration,
    cancel: &AtomicBool,
    output_path: &Path,
    model_path: &Path,
) -> LoaderResult<()> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(500);
    let startup_timeout = startup_timeout_for(model_path);

    let mut last_output_size: u64 = 0;
    let mut last_output_change = Instant::now();

    loop {
        // ── 1. Check if the process exited on its own ───────────────────
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                return Err(ModelError::InferenceFailure(format!(
                    "subprocess exited with status: {}",
                    status
                )));
            }
            Ok(None) => { /* still running — fall through to checks */ }
            Err(e) => {
                return Err(ModelError::IoError(format!(
                    "failed to poll subprocess: {}",
                    e
                )));
            }
        }

        // ── 2. Cancellation ─────────────────────────────────────────────
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ModelError::Cancelled(
                "inference cancelled by caller".to_string(),
            ));
        }

        // ── 3. Output-file stall detection ──────────────────────────────
        // IMPORTANT: only activate stall detection after the startup phase.
        // During model loading, llama-cli writes boot messages ("Loading
        // model...") to stdout (which we redirect to the output file), then
        // goes silent for potentially several minutes while loading weights.
        // Without this guard the stall detector would kill the process
        // during a perfectly normal model load.
        let past_startup = start.elapsed() >= startup_timeout;

        if let Ok(meta) = std::fs::metadata(output_path) {
            let size = meta.len();
            if size != last_output_size {
                last_output_size = size;
                last_output_change = Instant::now();
            } else if past_startup && size > 0 && last_output_change.elapsed() >= STALL_THRESHOLD {
                // Output has content and hasn't grown — inference is done
                // but the process is stuck (e.g. CUDA cleanup hang).
                eprintln!(
                    "[model_loader][INFO] output stalled at {} bytes for {}s — killing subprocess",
                    size,
                    STALL_THRESHOLD.as_secs()
                );
                crate::log_callback::emit_log_line(
                    &format!("[stall] output unchanged at {} bytes for {}s — killing process",
                        size, STALL_THRESHOLD.as_secs())
                );
                let _ = child.kill();
                let _ = child.wait();
                return Ok(()); // success — the output file is complete
            } else if size == 0 && last_output_change.elapsed() >= startup_timeout {
                // Process has been running for a while but never produced any
                // output — probably stuck during model load.
                eprintln!(
                    "[model_loader][WARN] 0-byte output after {}s — killing subprocess",
                    startup_timeout.as_secs()
                );
                crate::log_callback::emit_log_line(
                    &format!("[timeout] no output after {}s — process likely stuck during model load",
                        startup_timeout.as_secs())
                );
                let _ = child.kill();
                let _ = child.wait();
                return Err(ModelError::Timeout(format!(
                    "subprocess produced no output after {}s (model load may have failed)",
                    startup_timeout.as_secs()
                )));
            }
        }

        // ── 4. Hard timeout ─────────────────────────────────────────────
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ModelError::Timeout(format!(
                "subprocess exceeded timeout of {}s",
                timeout.as_secs()
            )));
        }

        std::thread::sleep(poll_interval);
    }
}
