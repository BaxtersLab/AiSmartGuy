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
    // Deadline for the "never produced any output" watchdog — see
    // zero_byte_deadline and the call site below for why `startup_timeout`
    // alone is too small.
    let zero_byte_deadline = zero_byte_deadline(timeout, startup_timeout);

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
            } else if zero_byte_stuck(size, last_output_change.elapsed(), zero_byte_deadline) {
                // Process has been running for a while but never produced any
                // output — probably stuck during model load.
                //
                // Bounded by `zero_byte_deadline` (>= the full ctx/CPU-scaled
                // inference timeout), NOT the much smaller model-size-only
                // `startup_timeout`. `output_path` only receives bytes once
                // the FIRST token is generated — with `llama-completion`,
                // boot/load messages go to stderr, not stdout — so on a large
                // RAG-heavy prompt under CPU-only inference, prefill alone can
                // legitimately take far longer than `startup_timeout` (a 4GB
                // model's 180s budget) without anything being "stuck". Using
                // the small deadline here killed healthy-but-slow prefill and
                // misreported it as a failed model load (the pre-fix
                // behaviour is this file at b18c755; the real run it was
                // found against is the 2026-08-17 test-box book run).
                eprintln!(
                    "[model_loader][WARN] 0-byte output after {}s — killing subprocess",
                    zero_byte_deadline.as_secs()
                );
                crate::log_callback::emit_log_line(
                    &format!("[timeout] no output after {}s — process likely stuck during model load",
                        zero_byte_deadline.as_secs())
                );
                let _ = child.kill();
                let _ = child.wait();
                return Err(ModelError::Timeout(format!(
                    "subprocess produced no output after {}s (model load may have failed)",
                    zero_byte_deadline.as_secs()
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

/// Deadline for the "never produced any output" watchdog: the full inference
/// `timeout` (already scaled for CPU offload and context size by
/// inference_timeout_for), never less than the model-size-only
/// `startup_timeout`. Using `startup_timeout` alone killed healthy CPU-only
/// prefill before its first token.
fn zero_byte_deadline(timeout: Duration, startup_timeout: Duration) -> Duration {
    timeout.max(startup_timeout)
}

/// True if the "process never produced any output" watchdog should fire.
///
/// Pulled out as a pure function so the deadline-selection bug (using the
/// small model-size-only `startup_timeout` instead of the full ctx/CPU-aware
/// `timeout`) is regression-testable without spawning a real subprocess.
fn zero_byte_stuck(output_size: u64, elapsed_since_change: Duration, deadline: Duration) -> bool {
    output_size == 0 && elapsed_since_change >= deadline
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this regresses: a large RAG-heavy prompt on CPU-only
    /// inference can legitimately sit at 0 output bytes for many minutes
    /// during prefill (llama-completion sends boot/load messages to stderr,
    /// not stdout) — well past a 180s model-size-only startup budget, but
    /// nowhere near the real, ctx-and-CPU-scaled inference timeout. Before
    /// the fix, this returned true and killed a healthy run.
    #[test]
    fn zero_bytes_within_full_inference_timeout_is_not_stuck() {
        let elapsed = Duration::from_secs(200); // past the old 180s startup budget
        let deadline = Duration::from_secs(12000); // real inference_timeout_for() result
        assert!(!zero_byte_stuck(0, elapsed, deadline));
    }

    #[test]
    fn zero_bytes_past_the_full_deadline_is_stuck() {
        let elapsed = Duration::from_secs(12001);
        let deadline = Duration::from_secs(12000);
        assert!(zero_byte_stuck(0, elapsed, deadline));
    }

    #[test]
    fn nonzero_output_is_never_flagged_stuck_by_this_check() {
        // Growing/complete output is handled by the stall-detection branch,
        // not this one — this check only ever looks at the 0-byte case.
        assert!(!zero_byte_stuck(1, Duration::from_secs(999_999), Duration::from_secs(1)));
    }

    #[test]
    fn zero_byte_deadline_is_never_smaller_than_the_real_inference_timeout() {
        // Regression for the exact bug found in a real run: a 4GB CPU-only
        // model with a 32K-context, RAG-heavy prompt computes a real
        // inference timeout of 12000s (inference_timeout_for), while the
        // model-size-only startup estimate is only 180s. The watchdog must
        // use the larger of the two — and 200s of silent prefill is healthy.
        let startup_timeout = Duration::from_secs(180);
        let real_inference_timeout = Duration::from_secs(12000);
        let deadline = zero_byte_deadline(real_inference_timeout, startup_timeout);
        assert_eq!(deadline, real_inference_timeout);
        assert!(!zero_byte_stuck(0, Duration::from_secs(200), deadline));
    }

    /// The other side of the max: a very short inference timeout must not
    /// shrink the watchdog below the time the model needs just to load.
    #[test]
    fn zero_byte_deadline_never_drops_below_the_startup_budget() {
        let deadline = zero_byte_deadline(Duration::from_secs(60), Duration::from_secs(180));
        assert_eq!(deadline, Duration::from_secs(180));
    }
}
