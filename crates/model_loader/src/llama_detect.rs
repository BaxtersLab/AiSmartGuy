use std::path::PathBuf;
use std::io::{Read, Seek, SeekFrom};

/// Name of the llama.cpp completion binary on the current platform.
#[cfg(target_os = "windows")]
pub const LLAMA_BIN: &str = "llama-completion.exe";
#[cfg(not(target_os = "windows"))]
pub const LLAMA_BIN: &str = "llama-completion";

/// Directory where ASG stores the llama.cpp installation.
pub fn llama_install_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\default".to_string());
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".aismartguy").join("llama-cpp")
}

/// Full path to the local llama-cli binary (may not exist yet).
pub fn llama_local_path() -> PathBuf {
    llama_install_dir().join(LLAMA_BIN)
}

/// Detect llama-cli: check local install dir first, then PATH.
/// Returns `Some(path)` if found, `None` otherwise.
pub fn detect_llama() -> Option<PathBuf> {
    let local = llama_local_path();
    if local.is_file() {
        return Some(local);
    }

    // Check PATH
    if let Ok(output) = std::process::Command::new("where")
        .arg(LLAMA_BIN)
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().next() {
                let p = PathBuf::from(line.trim());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }

    // Also check bare "llama" / "llama.exe" in case of older installs
    #[cfg(target_os = "windows")]
    let alt = "llama.exe";
    #[cfg(not(target_os = "windows"))]
    let alt = "llama";

    if let Ok(output) = std::process::Command::new("where").arg(alt).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().next() {
                let p = PathBuf::from(line.trim());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }

    None
}

/// Resolve the llama-cli path to use. Panics with a clear message if not found.
/// Prefer `detect_llama()` for optional checking.
pub fn resolve_llama_path() -> PathBuf {
    detect_llama().unwrap_or_else(|| llama_local_path())
}

/// Read the native context length from a GGUF file's metadata.
///
/// Scans the GGUF key-value metadata for the key
/// `<arch>.context_length` (e.g. `qwen3.context_length`).
/// Returns `None` if the file can't be read or the key isn't found.
pub fn gguf_context_length(model_path: &std::path::Path) -> Option<u32> {
    gguf_read_u32_key(model_path, "context_length")
}

/// Read the number of transformer blocks (layers) from a GGUF file.
///
/// Looks for `<arch>.block_count` in the metadata.
pub fn gguf_block_count(model_path: &std::path::Path) -> Option<u32> {
    gguf_read_u32_key(model_path, "block_count")
}

/// Generic helper: scan GGUF metadata for a key ending in `.<suffix>` of type UINT32.
fn gguf_read_u32_key(model_path: &std::path::Path, suffix: &str) -> Option<u32> {
    let mut f = std::fs::File::open(model_path).ok()?;

    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }

    let version = read_u32_le(&mut f)?;
    if version < 2 || version > 3 {
        return None;
    }

    let _tensor_count = read_u64_le(&mut f)?;
    let kv_count = read_u64_le(&mut f)?;

    let target_suffix = format!(".{}", suffix);

    for _ in 0..kv_count {
        let key = read_gguf_string(&mut f)?;
        let value_type = read_u32_le(&mut f)?;

        if key.ends_with(&target_suffix) && value_type == 4 {
            let val = read_u32_le(&mut f)?;
            eprintln!("[gguf] {} = {}", key, val);
            return Some(val);
        }

        if !skip_gguf_value(&mut f, value_type) {
            return None;
        }
    }

    None
}

/// Compute the maximum context length the hardware can realistically handle
/// for the given model without OOM.
///
/// Works backwards from VRAM: subtracts a minimum GPU-layer reservation
/// (at least 8 layers or 25% of model, whichever is larger), overhead, and
/// then converts remaining VRAM into KV-cache capacity using the same
/// heuristic as `auto_gpu_layers`.
///
/// Returns `None` if VRAM/model info is unavailable (caller should fall back
/// to a safe default).
pub fn max_context_for_vram(model_path: &std::path::Path, vram_mb: u32) -> Option<u32> {
    let file_size_mb = std::fs::metadata(model_path)
        .map(|m| (m.len() / (1024 * 1024)) as u32)
        .ok()?;
    if file_size_mb == 0 || vram_mb == 0 {
        return None;
    }
    let total_layers = gguf_block_count(model_path).unwrap_or((file_size_mb / 512).max(16));
    let mb_per_layer = file_size_mb / (total_layers + 2).max(1);

    // Reserve VRAM for at least 8 layers or 25% of model, whichever is larger.
    let min_gpu_layers = (total_layers / 4).max(8).min(total_layers);
    let layer_reservation = min_gpu_layers * mb_per_layer;
    let overhead = 300_u32; // CUDA scratch + embedding tables

    let vram_for_kv = vram_mb.saturating_sub(layer_reservation).saturating_sub(overhead);
    // KV heuristic: ~1 MB per 16 tokens (same as auto_gpu_layers).
    let max_ctx = vram_for_kv * 16;
    // Floor at 2048, round down to 2048 boundary.
    let max_ctx = (max_ctx / 2048) * 2048;
    let max_ctx = max_ctx.max(2048);

    eprintln!(
        "[gpu] max_context_for_vram: model={}MB {}layers, {}MB/layer, vram={}MB, layer_reserve={}MB({}layers), overhead={}MB → kv_budget={}MB → max_ctx={}",
        file_size_mb, total_layers, mb_per_layer, vram_mb, layer_reservation, min_gpu_layers, overhead, vram_for_kv, max_ctx
    );

    Some(max_ctx)
}

/// Auto-calculate optimal `--n-gpu-layers` for a model given available VRAM.
///
/// Estimates memory per layer from file size / block count, reserves headroom
/// for KV cache + CUDA overhead, and fits as many layers in VRAM as possible.
/// Remaining layers run on system RAM via llama.cpp CPU offload.
///
/// Returns `(n_gpu_layers, total_layers)`.
pub fn auto_gpu_layers(model_path: &std::path::Path, vram_mb: u32, ctx_tokens: u32) -> (u32, u32) {
    let file_size_mb = std::fs::metadata(model_path)
        .map(|m| (m.len() / (1024 * 1024)) as u32)
        .unwrap_or(0);

    // Default fallback: estimate layers from file size if GGUF parse fails.
    // ~0.5 GB per layer is typical across model sizes.
    let fallback_layers = (file_size_mb / 512).max(16);
    let total_layers = gguf_block_count(model_path).unwrap_or(fallback_layers);

    if file_size_mb == 0 || vram_mb == 0 {
        eprintln!("[gpu] no VRAM or unknown model size — defaulting to CPU");
        return (0, total_layers);
    }

    // Reserve VRAM for KV cache + embeddings + CUDA overhead.
    // KV cache scales linearly with context length; use ~1 MB per 16 tokens
    // as a conservative heuristic (covers typical GQA architectures in fp16).
    let kv_reserve = (ctx_tokens / 16).max(500);
    let overhead = 300_u32; // CUDA scratch + embedding tables
    let reserved_mb = kv_reserve + overhead;
    let usable_vram = vram_mb.saturating_sub(reserved_mb);

    // Approximate MB per layer (model weights only).
    // +2 accounts for embedding + output head layers not in block_count.
    let mb_per_layer = file_size_mb / (total_layers + 2).max(1);

    let fit_layers = if mb_per_layer > 0 {
        (usable_vram / mb_per_layer).min(total_layers)
    } else {
        total_layers
    };

    eprintln!(
        "[gpu] model={} MB, {} layers, ~{} MB/layer, vram={} MB (reserved={}, usable={}), ctx={}→ gpu_layers={}/{}",
        file_size_mb, total_layers, mb_per_layer, vram_mb, reserved_mb, usable_vram, ctx_tokens, fit_layers, total_layers
    );

    (fit_layers, total_layers)
}

// ── GGUF binary helpers ────────────────────────────────────────────────────

fn read_u8(f: &mut std::fs::File) -> Option<u8> {
    let mut buf = [0u8; 1];
    f.read_exact(&mut buf).ok()?;
    Some(buf[0])
}

fn read_u16_le(f: &mut std::fs::File) -> Option<u16> {
    let mut buf = [0u8; 2];
    f.read_exact(&mut buf).ok()?;
    Some(u16::from_le_bytes(buf))
}

fn read_u32_le(f: &mut std::fs::File) -> Option<u32> {
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf))
}

fn read_i32_le(f: &mut std::fs::File) -> Option<i32> {
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).ok()?;
    Some(i32::from_le_bytes(buf))
}

fn read_u64_le(f: &mut std::fs::File) -> Option<u64> {
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf).ok()?;
    Some(u64::from_le_bytes(buf))
}

fn read_f32_le(f: &mut std::fs::File) -> Option<f32> {
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).ok()?;
    Some(f32::from_le_bytes(buf))
}

fn read_f64_le(f: &mut std::fs::File) -> Option<f64> {
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf).ok()?;
    Some(f64::from_le_bytes(buf))
}

/// Read a GGUF string: u64 length + UTF-8 bytes (no null terminator).
fn read_gguf_string(f: &mut std::fs::File) -> Option<String> {
    let len = read_u64_le(f)? as usize;
    if len > 1_000_000 { return None; } // sanity
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// Skip a GGUF metadata value of the given type.  Returns false if unknown type.
fn skip_gguf_value(f: &mut std::fs::File, vtype: u32) -> bool {
    match vtype {
        0 => read_u8(f).is_some(),                          // UINT8
        1 => { f.seek(SeekFrom::Current(1)).ok(); true }    // INT8
        2 => read_u16_le(f).map(|_| ()).is_some(),          // UINT16
        3 => { f.seek(SeekFrom::Current(2)).ok(); true }    // INT16
        4 => read_u32_le(f).map(|_| ()).is_some(),          // UINT32
        5 => read_i32_le(f).map(|_| ()).is_some(),          // INT32
        6 => read_f32_le(f).map(|_| ()).is_some(),          // FLOAT32
        7 => { read_u32_le(f).map(|_| ()).is_some() }       // BOOL (u32 in GGUF)
        8 => read_gguf_string(f).map(|_| ()).is_some(),     // STRING
        9 => {
            // ARRAY: type (u32) + count (u64) + elements
            let elem_type = match read_u32_le(f) { Some(t) => t, None => return false };
            let count = match read_u64_le(f) { Some(c) => c, None => return false };
            for _ in 0..count {
                if !skip_gguf_value(f, elem_type) { return false; }
            }
            true
        }
        10 => read_u64_le(f).map(|_| ()).is_some(),         // UINT64
        11 => { f.seek(SeekFrom::Current(8)).ok(); true }   // INT64
        12 => read_f64_le(f).map(|_| ()).is_some(),         // FLOAT64
        _ => false, // unknown type
    }
}
