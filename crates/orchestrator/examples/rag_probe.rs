//! Diagnostic: what RAG plan does each model actually get?
//!
//! Companion to `model_loader/examples/ctx_probe.rs`. That one answered "how
//! much context will this GGUF give me"; this one answers the question that
//! actually killed a run — "does the RAG library fit in it, and if not, what
//! does the orchestrator do about it".
//!
//! Reads the real packet library from `~/.aismartguy/rag_defaults/` plus any
//! per-model `rag/` folder, and prints the same numbers the orchestrator
//! computes in step 2c.
//!
//! ```text
//! cargo run -p orchestrator --example rag_probe -- <model.gguf> [more.gguf ...]
//! ```

use orchestrator::rag_bridge::{load_merged_packets, model_rag_dir};
use orchestrator::rag_plan::{est_tokens, plan_rag_for_context};
use rag_engine::PromptDetail;

const GEN_HEADROOM: usize = 2048;
const SAFETY_PCT: usize = 10;
const MIN_CHAPTER_TOKENS: usize = 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: rag_probe <model.gguf> [more.gguf ...]");
        eprintln!("prints the RAG delivery plan the orchestrator would choose for each");
        std::process::exit(2);
    }

    let raw_vram = model_loader::query_vram_mb();
    println!("detected VRAM: {} MB (throttle 100%)\n", raw_vram);

    let mut tightest: Option<(String, usize)> = None;

    for arg in &args {
        let path = std::path::Path::new(arg);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| arg.clone());

        if !path.exists() {
            println!("{name}\n  MISSING: {arg}\n");
            continue;
        }

        let packets = load_merged_packets(&model_rag_dir(arg));
        let rules: usize = packets.iter().map(|p| p.rules.len()).sum();
        let full = est_tokens(&rag_engine::build_system_prompt_with_detail(
            &packets,
            PromptDetail::Full,
        ));
        let compact = est_tokens(&rag_engine::build_system_prompt_with_detail(
            &packets,
            PromptDetail::Compact,
        ));

        let native = model_loader::gguf_context_length(path).map(|n| n as usize);
        let vram_cap = model_loader::max_context_for_vram(path, raw_vram).map(|n| n as usize);

        // Same three-way min the orchestrator applies, at the default 16384.
        let user_ctx = 16384usize;
        let mut ctx = user_ctx;
        let mut bound = "the context setting";
        if let Some(n) = native {
            if n < ctx {
                ctx = n;
                bound = "the model's own trained context";
            }
        }
        if let Some(v) = vram_cap {
            if v < ctx {
                ctx = v;
                bound = "available VRAM";
            }
        }
        let ctx = ctx.max(2048);

        let safety = ctx * SAFETY_PCT / 100;
        let usable = ctx.saturating_sub(GEN_HEADROOM).saturating_sub(safety);
        let (plan, capacity) = plan_rag_for_context(&packets, usable, MIN_CHAPTER_TOKENS);

        println!("{name}");
        println!(
            "  native ctx      : {}",
            native.map(|n| n.to_string()).unwrap_or_else(|| "unreadable".into())
        );
        println!(
            "  VRAM allows     : {}",
            vram_cap.map(|n| n.to_string()).unwrap_or_else(|| "unreadable".into())
        );
        println!("  ctx in use      : {ctx}  (bound by {bound})");
        println!("  usable          : {usable}  (ctx − gen {GEN_HEADROOM} − safety {safety})");
        println!("  library         : {rules} rules, {full} tok full / {compact} tok compact");
        println!("  plan            : {}", plan.describe());
        println!("  chapter capacity: {capacity} tok");

        if capacity < MIN_CHAPTER_TOKENS {
            println!("  >> BELOW the {MIN_CHAPTER_TOKENS}-token floor — this model would block the run");
        }
        println!();

        match &tightest {
            Some((_, c)) if *c <= capacity => {}
            _ => tightest = Some((name, capacity)),
        }
    }

    if let Some((name, capacity)) = tightest {
        println!("---");
        println!("chapter budget for the run: {capacity} tok, set by '{name}' (the tightest model)");
        if capacity < MIN_CHAPTER_TOKENS {
            println!("run would FAIL with CONTEXT_TOO_SMALL");
        } else {
            println!("run is viable");
        }
    }
}
