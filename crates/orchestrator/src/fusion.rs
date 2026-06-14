use std::path::{Path, PathBuf};

use model_loader::{
    gguf_context_length, max_context_for_vram, query_vram_mb, InferenceRequest, ModelInstance,
};

use crate::errors::{OrchestratorError, OrchestratorResult};
use crate::orchestrator::strip_llama_noise;
use crate::state_bridge;
use crate::types::FusionInput;

/// Instruction/preamble overhead reserved in every fold prompt (tokens).
const FOLD_INSTR_OVERHEAD: usize = 512;
/// Generation headroom — must stay in sync with `-n` in command_builder (tokens).
const FOLD_GEN_HEADROOM: usize = 2048;
/// Safety margin for token-estimation error (percent of context).
const FOLD_SAFETY_PCT: usize = 10;
/// Conservative chars-per-token estimate (matches pdf_io::chapter_detect).
const CHARS_PER_TOKEN: usize = 2;
/// Hard cap on fold passes to guarantee termination on pathological input.
const MAX_FOLD_PASSES: usize = 16;

/// Run the fusion model over the collected per-chapter analyses.
///
/// Instead of concatenating every chapter's analysis from every model into a
/// single prompt (which overflows the context window on a full book and makes
/// the model "lose its mind" when llama.cpp silently truncates the front), this
/// performs a **budgeted hierarchical fold**:
///
///   1. Flatten the outputs into labeled leaf blocks (deterministic order).
///   2. Size the fold budget from the fusion model's *real* limits — its native
///      GGUF context, capped again by what VRAM can hold.
///   3. Batch leaves into groups that fit the budget, fuse each group into one
///      intermediate summary, and repeat on the summaries until a single block
///      remains. Generation is capped at `-n`, so each pass strictly shrinks the
///      material — convergence is guaranteed.
///
/// The model is loaded once and reused across every pass.
pub fn run_fusion(
    fusion_config: &manifest::ModelConfig,
    input: &FusionInput,
    run_dir: &PathBuf,
    throttle_pct: u32,
) -> OrchestratorResult<PathBuf> {
    let prompt_dir = run_dir.join("prompts").join("fusion");
    let output_dir = run_dir.join("outputs").join("fusion");
    let log_dir = run_dir.join("logs");
    for dir in [&prompt_dir, &output_dir, &log_dir] {
        std::fs::create_dir_all(dir).map_err(|e| OrchestratorError::IoError(e.to_string()))?;
    }

    let final_output_path = output_dir.join("fusion_output.txt");

    // ── Step 1: Flatten model outputs into labeled leaf blocks ──────────────
    // Sort model names for determinism (HashMap iteration order is not stable).
    let mut model_names: Vec<&String> = input.model_outputs.keys().collect();
    model_names.sort();

    let mut leaves: Vec<String> = Vec::new();
    for name in model_names {
        if let Some(outputs) = input.model_outputs.get(name) {
            for (i, text) in outputs.iter().enumerate() {
                let cleaned = strip_llama_noise(text);
                let cleaned = cleaned.trim();
                if cleaned.is_empty() {
                    continue;
                }
                leaves.push(format!("=== {} · chapter {} ===\n{}", name, i + 1, cleaned));
            }
        }
    }

    if leaves.is_empty() {
        std::fs::write(&final_output_path, "No analysis output was produced to synthesize.")
            .map_err(|e| OrchestratorError::IoError(e.to_string()))?;
        return Ok(final_output_path);
    }

    // ── Step 2: Derive the fold budget from the fusion model's real limits ──
    let fusion_ctx = fusion_effective_ctx(fusion_config, throttle_pct);
    let safety = fusion_ctx * FOLD_SAFETY_PCT / 100;
    let budget_tokens = fusion_ctx
        .saturating_sub(FOLD_INSTR_OVERHEAD)
        .saturating_sub(FOLD_GEN_HEADROOM)
        .saturating_sub(safety)
        .max(512);
    let budget_chars = budget_tokens.saturating_mul(CHARS_PER_TOKEN);

    eprintln!(
        "[fusion] {} leaf block(s); fusion_ctx={} → fold budget {}tok (~{} chars)",
        leaves.len(),
        fusion_ctx,
        budget_tokens,
        budget_chars
    );

    // ── Step 3: Load fusion model once, fold, then always unload ────────────
    let mut sized_config = fusion_config.clone();
    sized_config.context_length = Some(fusion_ctx as u32);
    let mut instance = state_bridge::make_instance(&sized_config, throttle_pct);
    state_bridge::load(&mut instance)?;

    let fold_result = run_fold(
        &mut instance,
        leaves,
        budget_chars,
        &prompt_dir,
        &output_dir,
        &log_dir,
    );

    // Unload before propagating any fold error so we never leave the model
    // resident (Drop is a backstop, but be explicit).
    let _ = state_bridge::unload(&mut instance);

    let final_text = fold_result?;

    std::fs::write(&final_output_path, &final_text)
        .map_err(|e| OrchestratorError::IoError(e.to_string()))?;

    Ok(final_output_path)
}

/// Drive the hierarchical fold against an already-loaded instance.
fn run_fold(
    instance: &mut ModelInstance,
    mut leaves: Vec<String>,
    budget_chars: usize,
    prompt_dir: &Path,
    output_dir: &Path,
    log_dir: &Path,
) -> OrchestratorResult<String> {
    let mut infer_counter: usize = 0;

    for pass in 0..MAX_FOLD_PASSES {
        let groups = batch_to_budget(&leaves, budget_chars);
        let is_final = groups.len() == 1;

        eprintln!(
            "[fusion] pass {}: {} leaf(s) → {} group(s){}",
            pass,
            leaves.len(),
            groups.len(),
            if is_final { " (final synthesis)" } else { "" }
        );

        let mut next: Vec<String> = Vec::with_capacity(groups.len());
        for (gi, group) in groups.iter().enumerate() {
            let prompt = build_fold_prompt(group, is_final);
            let prompt_path = prompt_dir.join(format!("pass{:02}_group{:03}.txt", pass, gi));
            let output_path = output_dir.join(format!("pass{:02}_group{:03}_out.txt", pass, gi));
            let log_path = log_dir.join(format!("fusion_pass{:02}_group{:03}.log", pass, gi));

            std::fs::write(&prompt_path, &prompt)
                .map_err(|e| OrchestratorError::IoError(e.to_string()))?;

            let request = InferenceRequest {
                chunk_id: infer_counter,
                prompt_path,
                output_path: output_path.clone(),
                log_path,
            };
            infer_counter += 1;

            state_bridge::infer(instance, &request)?;

            let raw = std::fs::read_to_string(&output_path).unwrap_or_default();
            let cleaned = strip_llama_noise(&raw);
            let cleaned = cleaned.trim();
            // If a fold pass yields nothing usable, fall back to the group's raw
            // inputs so a section's findings are never silently dropped.
            if cleaned.is_empty() {
                next.push(group.join("\n\n"));
            } else {
                next.push(cleaned.to_string());
            }
        }

        leaves = next;

        if is_final {
            return Ok(leaves.into_iter().next().unwrap_or_default());
        }
    }

    // Pass cap reached (pathological input) — return what we have rather than
    // failing the whole run; the orchestrator's fallback will still surface it.
    Ok(leaves.join("\n\n"))
}

/// Determine the fusion model's usable context: native GGUF context, capped by
/// what the (throttled) VRAM budget can actually hold to avoid KV-cache OOM.
fn fusion_effective_ctx(fusion_config: &manifest::ModelConfig, throttle_pct: u32) -> usize {
    let path = Path::new(&fusion_config.path);
    let user_ctx = fusion_config.context_length.unwrap_or(16384) as usize;

    // Cap to the model's native context.
    let mut ctx = match gguf_context_length(path) {
        Some(native) => user_ctx.min(native as usize),
        None => user_ctx,
    };

    // Cap to what VRAM can actually hold.
    let throttle = throttle_pct.clamp(25, 100);
    let raw_vram = query_vram_mb();
    let usable_vram = (raw_vram as u64 * throttle as u64 / 100) as u32;
    if let Some(hw_max) = max_context_for_vram(path, usable_vram) {
        ctx = ctx.min(hw_max as usize);
    }

    ctx.max(2048)
}

/// Group consecutive leaves so each group's combined text fits `budget_chars`.
/// Every group holds at least one leaf, guaranteeing forward progress even if a
/// single leaf exceeds the budget (that lone leaf gets truncated by llama.cpp,
/// but its fold output is bounded and rejoins the fold on the next pass).
fn batch_to_budget(leaves: &[String], budget_chars: usize) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut current_len: usize = 0;

    for leaf in leaves {
        let leaf_len = leaf.len() + 2; // +2 for the "\n\n" joiner
        if !current.is_empty() && current_len + leaf_len > budget_chars {
            groups.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current_len += leaf_len;
        current.push(leaf.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Build a fold prompt. Intermediate passes consolidate without losing detail;
/// the final pass produces the coherent whole-book review.
fn build_fold_prompt(blocks: &[String], is_final: bool) -> String {
    let instruction = if is_final {
        "You are writing the final consolidated review of an entire book. Below are \
         analyses of its individual sections, each produced by examining that section \
         for manipulation tactics, logical fallacies, and rhetorical framing. Write one \
         coherent review of the whole book: a short summary of its central argument, \
         every distinct manipulation tactic or fallacy found (with where it occurs and \
         its severity), and an overall assessment. Do not omit any section's findings.\n\n\
         --- SECTION ANALYSES ---\n"
    } else {
        "You are consolidating analyses of several sections of a book into a single \
         intermediate summary. Merge the analyses below, preserving every distinct \
         finding, flagged manipulation tactic or fallacy, and its severity. Keep the \
         section/chapter labels so later synthesis can still tell them apart. Be concise \
         but drop nothing.\n\n--- SECTION ANALYSES ---\n"
    };
    format!("{}{}", instruction, blocks.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_leaves_yield_no_groups() {
        let groups = batch_to_budget(&[], 1000);
        assert!(groups.is_empty());
    }

    #[test]
    fn leaves_within_budget_form_one_group() {
        let leaves = vec!["a".repeat(100), "b".repeat(100), "c".repeat(100)];
        let groups = batch_to_budget(&leaves, 1000);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn oversized_set_splits_into_multiple_groups() {
        // Three 400-char leaves with a 900-char budget → 2 groups (2 + 1).
        let leaves = vec!["a".repeat(400), "b".repeat(400), "c".repeat(400)];
        let groups = batch_to_budget(&leaves, 900);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[1].len(), 1);
    }

    #[test]
    fn single_leaf_over_budget_still_makes_progress() {
        // One leaf larger than the budget must not be dropped or loop forever.
        let leaves = vec!["x".repeat(5000)];
        let groups = batch_to_budget(&leaves, 1000);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 1);
    }

    #[test]
    fn final_and_intermediate_prompts_differ() {
        let blocks = vec!["=== model1 · chapter 1 ===\nfinding".to_string()];
        let final_prompt = build_fold_prompt(&blocks, true);
        let inter_prompt = build_fold_prompt(&blocks, false);
        assert!(final_prompt.contains("final consolidated review"));
        assert!(inter_prompt.contains("intermediate summary"));
        assert!(final_prompt.contains("finding"));
    }
}
