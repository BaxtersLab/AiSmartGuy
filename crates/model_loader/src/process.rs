use std::process::Stdio;
use crate::errors::{LoaderResult, ModelError};
use crate::types::ModelInstance;

/// Spawns `cmd` as a subprocess. Returns the live `Child` handle.
///
/// The caller is responsible for waiting on the child (via `timeout::enforce_timeout`
/// or `child.wait()`).
pub fn spawn_process(instance: &ModelInstance, cmd: std::process::Command) -> LoaderResult<std::process::Child> {
    let mut cmd = cmd;

    // Explicit stdin: llama-cli reads from -f file, never stdin.
    cmd.stdin(Stdio::null());
    // Capture stderr so we can log it (don't suppress — we need diagnostics).
    cmd.stderr(Stdio::piped());

    // On Windows, hide the console window that llama-cli would otherwise open.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // Log the full command line to the live terminal for debugging.
    let args: Vec<String> = std::iter::once(cmd.get_program().to_string_lossy().into_owned())
        .chain(cmd.get_args().map(|a| a.to_string_lossy().into_owned()))
        .collect();
    let cmdline = args.join(" ");
    eprintln!("[model_loader][INFO] exec: {}", cmdline);
    crate::log_callback::emit_log_line(&format!("[exec] {}", cmdline));

    cmd.spawn().map_err(|e| {
        ModelError::LoadFailure(format!(
            "failed to spawn llama process: {}",
            e
        ))
    })
}

/// Kills a running subprocess unconditionally. Logs any kill failure but does
/// not propagate it as an error — unloading must always succeed.
pub fn kill_process(child: &mut std::process::Child) {
    if let Err(e) = child.kill() {
        eprintln!("[model_loader][WARN] kill failed (process may have already exited): {}", e);
    }
    // Reap the zombie so we don't leave orphaned processes.
    let _ = child.wait();
}
