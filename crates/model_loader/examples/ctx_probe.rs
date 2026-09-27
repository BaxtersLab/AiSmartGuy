//! Report what context length a model can actually run in the available VRAM.
//!
//! Exists because "CONTEXT_TOO_SMALL" is reported in tokens while the limit is
//! really VRAM, and the two are only connected by the model's per-token KV cost.
//!
//!   cargo run -p model_loader --example ctx_probe -- <model.gguf> [vram_mb]

fn main() {
    let mut a = std::env::args().skip(1);
    let p = std::path::PathBuf::from(a.next().expect("usage: ctx_probe <model.gguf> [vram_mb]"));
    let vram: u32 = a.next().and_then(|v| v.parse().ok())
        .unwrap_or_else(model_loader::query_vram_mb);

    let size_mb = std::fs::metadata(&p).map(|m| m.len() / 1048576).unwrap_or(0);
    let native = model_loader::gguf_context_length(&p).unwrap_or(0);
    let layers = model_loader::gguf_block_count(&p).unwrap_or(0);
    let per_tok = model_loader::kv_bytes_per_token(&p);

    println!("model      {}", p.file_name().unwrap_or_default().to_string_lossy());
    println!("size       {} MB", size_mb);
    println!("layers     {}", layers);
    println!("native ctx {}", native);
    println!("VRAM       {} MB", vram);
    match per_tok {
        Some(b) => println!("KV/token   {} bytes", b),
        None    => println!("KV/token   unknown (metadata unreadable)"),
    }
    if let Some(max_ctx) = model_loader::max_context_for_vram(&p, vram) {
        println!("max ctx    {}", max_ctx);
    }
    println!();
    for ctx in [131072u32, 65536, 32768, 16384, 8192] {
        if ctx > native && native > 0 { continue; }
        let (ngl, total) = model_loader::auto_gpu_layers(&p, vram, ctx);
        let kv_mb = per_tok.map(|b| (b * ctx as u64) / 1048576).unwrap_or(0);
        println!("  ctx {:>6}  ->  {:>2}/{} layers on GPU,  KV {} MB", ctx, ngl, total, kv_mb);
    }
}
