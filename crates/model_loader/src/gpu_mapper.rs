use std::sync::OnceLock;

/// Cached VRAM value so we only shell out once per process.
static VRAM_CACHE: OnceLock<u32> = OnceLock::new();

/// Query total dedicated GPU memory (MB) via nvidia-smi.
/// Returns 0 if the query fails (no NVIDIA GPU, driver missing, etc.).
pub fn query_vram_mb() -> u32 {
    *VRAM_CACHE.get_or_init(|| {
        let result = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let vram = stdout
                    .lines()
                    .next()
                    .and_then(|line| line.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                eprintln!("[model_loader][INFO] detected GPU VRAM: {} MB", vram);
                vram
            }
            _ => {
                eprintln!("[model_loader][WARN] nvidia-smi query failed — assuming no GPU");
                0
            }
        }
    })
}
