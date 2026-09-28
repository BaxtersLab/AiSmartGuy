use std::collections::HashMap;
use std::path::PathBuf;

use manifest::Manifest;
use model_loader::InferenceRequest;

use crate::bridge_model_fetcher::ensure_model_ready;
use crate::errors::{OrchestratorError, OrchestratorResult};
use crate::fusion::run_fusion;
use crate::manifest_bridge::{load_manifest, save_manifest, validate_manifest};
use crate::optimization_bridge::{run_optimization_pass, FindingsScoring};
use crate::pdf_bridge::{chapter_split, extract_pdf, write_final_pdf};
use crate::progress::emit_progress;
use crate::rag_bridge::{load_merged_packets, model_rag_dir};
use crate::rag_plan::{plan_rag_for_context, RagPlan, CHARS_PER_TOKEN};
use crate::sequence_plan::build_sequence;
use crate::state_bridge;
use crate::types::{FusionInput, ModelOutputs, OrchestratorProgressEvent, OrchestratorState};

/// Generation output headroom (tokens).
const GEN_HEADROOM: usize = 2048;
/// Percentage of context reserved as safety margin for token-estimation error.
/// Different tokenizers average 1.5–2.5 chars/token; 10% covers the variance.
const SAFETY_PCT: usize = 10;
/// Overlap between consecutive chapters (tokens).
const CHAPTER_OVERLAP: usize = 200;

/// Per-chapter analysis instruction for prose mode (the fallback: structured
/// findings are the default wherever the llama build supports grammars, see
/// `findings_mode_from`). Without this, the
/// prompt sent to `llama-completion` was just [RAG rule catalog] + [raw
/// chapter text] with nothing telling the model what to do with either one.
/// The directive alone was not enough: while `llama-completion` ran with
/// `-no-cnv` (raw completion, no chat template) the model still continued
/// the source essay on every pass of a real run (2026-09-27). The prompt now
/// goes in as one chat turn (`-cnv -st`, see model_loader's command
/// builder), which is where an instruct model's instruction-following
/// engages. Mirrors FINDINGS_INSTRUCTION's role for the structured-findings
/// path, but for free-form prose output.
const PROSE_ANALYSIS_INSTRUCTION: &str = "\
Using the rule catalog above, write a critical analysis of the passage below. \
Identify specific instances of logical fallacies, loaded or manipulative \
language, and rhetorical framing, quoting the exact text for each instance \
and explaining which pattern it matches and why. Do not continue, \
paraphrase, or summarize the passage itself — analyze it.\n\n\
PASSAGE TO ANALYZE:";
/// Absolute minimum chunk budget if context is very small.
const MIN_CHAPTER_TOKENS: usize = 1024;

/// What one model can actually do, worked out before any chapter is cut.
///
/// The RAG library no longer has to fit a single prompt — `rag` may spread it
/// over several passes — so `capacity` is a workable number even for a model
/// whose whole context is smaller than the library.
struct ModelBudget {
    /// Context this model will load: min(user setting, native limit, VRAM).
    ctx: usize,
    /// Which of those three bound it — for an honest error message.
    limited_by: &'static str,
    /// How this model's RAG packets get delivered.
    rag: RagPlan,
    /// Tokens left for chapter text after RAG, generation and safety.
    capacity: usize,
}

/// The orchestrator — master control loop for a single AiSmartGuy run.
pub struct Orchestrator {
    pub manifest: Manifest,
    /// Directory where run artifacts are stored.
    pub run_dir: PathBuf,
    /// Path to the manifest JSON file (for save-back).
    pub manifest_path: PathBuf,
    /// Current internal state.
    pub state: OrchestratorState,
    /// Accumulated score history across runs (for optimization consensus).
    pub score_history: optimization::ScoreHistory,
}

impl Orchestrator {
    /// Create a new orchestrator from a manifest file path and a run directory root.
    ///
    /// The run_dir will be created if it does not exist.
    pub fn new(manifest_path: PathBuf, run_dir: PathBuf) -> OrchestratorResult<Self> {
        let manifest = load_manifest(&manifest_path)?;
        validate_manifest(&manifest)?;

        std::fs::create_dir_all(&run_dir)
            .map_err(|e| OrchestratorError::IoError(e.to_string()))?;

        Ok(Self {
            manifest,
            run_dir,
            manifest_path,
            state: OrchestratorState::Idle,
            score_history: Vec::new(),
        })
    }

    /// Execute the full run lifecycle (blocking).
    ///
    /// Returns the path to the final output PDF on success.
    pub fn run(&mut self, pdf_path: PathBuf) -> OrchestratorResult<PathBuf> {
        self.emit("IDLE", "run started", 0.0);

        // ── Step 1: Extract PDF ─────────────────────────────────────────────
        self.state = OrchestratorState::LoadingPdf;
        self.emit("LOADING_PDF", "extracting PDF text", 0.05);

        let extracted = extract_pdf(&pdf_path)?;

        // ── Step 2: Chapter-split PDF ────────────────────────────────────
        //
        // Dynamic budget: context_length minus RAG overhead, generation
        // headroom, and safety → remainder is available for chapter text.
        self.state = OrchestratorState::Chunking;
        self.emit("CHUNKING", "splitting PDF into chapters", 0.10);

        // ── Step 2b: Build sequence plan (needed to know which models) ──────
        let plan = build_sequence(&self.manifest);

        if plan.model_order.is_empty() {
            return Err(OrchestratorError::InvalidState(
                "sequence plan is empty — no active models in manifest".to_string(),
            ));
        }

        // ── Step 2c: Per-model context and RAG delivery plan ────────────────
        //
        // Each model gets its OWN context — the smallest of what the user asked
        // for, what the GGUF was trained on, and what VRAM can hold. The old
        // code took the minimum across every model and applied it to all of
        // them, so one 8K model in the lineup dragged a 262K model down to 8K
        // for no reason.
        //
        // Each model also gets its own RAG plan. The library no longer has to
        // fit in a single prompt: `plan_rag_for_context` spreads it over as
        // many passes as the context needs, without dropping a rule. That is
        // what turns a negative chapter budget into a workable one.
        let user_ctx = self.manifest.models.model1
            .as_ref()
            .and_then(|m| m.context_length)
            .unwrap_or(16384) as usize;

        let throttle = self.manifest.resource_throttle.throttle_pct.clamp(25, 100);
        let raw_vram = model_loader::query_vram_mb();
        let usable_vram = (raw_vram as u64 * throttle as u64 / 100) as u32;

        let mut budgets: HashMap<String, ModelBudget> = HashMap::new();
        for model_name in &plan.model_order {
            let mc = match self.get_model_config(model_name) {
                Ok(mc) => mc,
                Err(e) => {
                    eprintln!("[orchestrator][WARN] no config for {}: {}", model_name, e);
                    continue;
                }
            };
            let path = std::path::Path::new(&mc.path);

            let mut ctx = user_ctx;
            let mut limited_by = "the context setting";
            if let Some(native) = model_loader::gguf_context_length(path) {
                let native = native as usize;
                eprintln!("[orchestrator] {} native context: {} tokens", model_name, native);
                if native < ctx {
                    ctx = native;
                    limited_by = "the model's own trained context";
                }
            }
            if let Some(hw_max) = model_loader::max_context_for_vram(path, usable_vram) {
                let hw_max = hw_max as usize;
                if hw_max < ctx {
                    ctx = hw_max;
                    limited_by = "available VRAM";
                }
            }
            let ctx = ctx.max(2048);

            let safety = ctx * SAFETY_PCT / 100;
            let usable = ctx.saturating_sub(GEN_HEADROOM).saturating_sub(safety);

            let packets = load_merged_packets(&model_rag_dir(&mc.path));
            let (rag, capacity) = plan_rag_for_context(&packets, usable, MIN_CHAPTER_TOKENS);

            eprintln!(
                "[orchestrator] {}: ctx={} (bound by {}), usable={} → RAG: {} → {}tok for chapter text",
                model_name, ctx, limited_by, usable, rag.describe(), capacity
            );

            budgets.insert(
                model_name.clone(),
                ModelBudget { ctx, limited_by, rag, capacity },
            );
        }

        // Chapters are cut once and read by every model, so the split has to
        // suit the tightest of them.
        let (tightest, chapter_budget) = budgets
            .iter()
            .map(|(name, b)| (name.clone(), b.capacity))
            .min_by_key(|(_, capacity)| *capacity)
            .unwrap_or_else(|| (String::new(), 0));

        if chapter_budget < MIN_CHAPTER_TOKENS {
            let (ctx, limited_by, rag_tokens, passes) = budgets
                .get(&tightest)
                .map(|b| (b.ctx, b.limited_by, b.rag.max_batch_tokens(), b.rag.passes()))
                .unwrap_or((0, "an unreadable model config", 0, 0));

            return Err(OrchestratorError::InvalidState(format!(
                "CONTEXT_TOO_SMALL: '{}' is the limiting model. Its context is {} tokens, \
                 bound by {}. After generation headroom ({}), a {}% safety margin and the \
                 largest of its {} RAG pass(es) ({} tokens), only {} remain for chapter text \
                 (minimum {}). The RAG library is already being split across passes, so the \
                 fix is the model, not the packets: drop '{}' from the lineup, or swap it for \
                 a build with a longer context.",
                tightest, ctx, limited_by, GEN_HEADROOM, SAFETY_PCT, passes, rag_tokens,
                chapter_budget, MIN_CHAPTER_TOKENS, tightest
            )));
        }

        eprintln!(
            "[orchestrator] chapter_budget={} tokens (set by '{}', the tightest model)",
            chapter_budget, tightest
        );

        let chapters = chapter_split(&extracted, chapter_budget, CHAPTER_OVERLAP);
        let total_chunks = chapters.len();

        if total_chunks == 0 {
            return Err(OrchestratorError::PdfError("PDF produced no chapters".to_string()));
        }

        eprintln!(
            "[orchestrator] split into {} chapter(s): {}",
            total_chunks,
            chapters.iter().map(|c| {
                let label = if c.title.is_empty() {
                    format!("ch{}", c.id)
                } else {
                    c.title.clone()
                };
                format!("{}(~{}t)", label, c.approx_tokens)
            }).collect::<Vec<_>>().join(", ")
        );

        // Write chapters to disk.
        let chunk_dir = self.run_dir.join("chapters");
        std::fs::create_dir_all(&chunk_dir)
            .map_err(|e| OrchestratorError::IoError(e.to_string()))?;

        for ch in &chapters {
            let label = if ch.title.is_empty() {
                format!("chapter_{:03}", ch.id + 1)
            } else {
                let safe: String = ch.title.chars()
                    .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                    .collect();
                format!("{:03}_{}", ch.id + 1, safe.trim())
            };
            let path = chunk_dir.join(format!("{}.txt", label));
            std::fs::write(&path, &ch.text)
                .map_err(|e| OrchestratorError::IoError(e.to_string()))?;
        }

        // ── Step 3: Cache root & pre-flight ─────────────────────────────────
        let model_count = plan.model_order.len();
        let cache_root: PathBuf = {
            #[cfg(target_os = "windows")]
            let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\default".to_string());
            #[cfg(not(target_os = "windows"))]
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".aismartguy").join("models")
        };

        let mut model_outputs: ModelOutputs = HashMap::new();
        // The same outputs, each with the chapter it analysed (see
        // ChapterOutputs). Built from the same vector as model_outputs.
        let mut chapter_outputs: ChapterOutputs = HashMap::new();
        let mut partial_failures: Vec<String> = Vec::new();

        // ── Pre-flight: verify llama-cli is available ───────────────────────
        if model_loader::detect_llama().is_none() {
            return Err(OrchestratorError::InvalidState(
                "LLAMA_NOT_INSTALLED: llama-cli not found. Install llama.cpp to run inference.".to_string(),
            ));
        }

        // ── Phase 2: structured-findings mode (the default; needs grammars) ────
        // Manager ruling F1 (2026-09-28): structured findings are the default
        // wherever the selected llama build supports --grammar-file, so every
        // quote is checked against its chapter. ASG_STRUCTURED_FINDINGS=0 opts
        // out. Without grammar support the run falls back to prose, and the
        // report says prominently that prose quotes are not checked.
        let findings_grammar: Option<PathBuf> = if findings_mode_enabled()
            && model_loader::llama_detect::llama_supports_grammar()
        {
            match crate::findings::write_grammar_file(&self.run_dir) {
                Ok(p) => {
                    eprintln!("[orchestrator] Phase 2 structured-findings mode ON → {}", p.display());
                    Some(p)
                }
                Err(e) => {
                    eprintln!("[orchestrator][WARN] could not write findings grammar ({e}); prose mode");
                    None
                }
            }
        } else {
            None
        };

        // ── Step 4: Per-model loop ──────────────────────────────────────────
        for (model_idx, model_name) in plan.model_order.iter().enumerate() {
            let model_progress_base = 0.15 + (model_idx as f64 / model_count as f64) * 0.70;

            // Get the manifest config for this model slot.
            let model_config = self.get_model_config(model_name)?;

            // ── 4a: Fetch / verify model ────────────────────────────────────
            self.state = OrchestratorState::FetchingModel;
            self.emit("FETCHING_MODEL", &format!("ensuring model ready: {}", model_name), model_progress_base);

            if let Err(e) = ensure_model_ready(model_name, &model_config, &cache_root) {
                eprintln!("[orchestrator][ERROR] fetch failed for {}: {}", model_name, e);
                partial_failures.push(model_name.to_string());
                continue;
            }

            // ── 4b: Load → infer chunks → unload ────────────────────────────
            self.state = OrchestratorState::RunningModel;
            self.emit("RUNNING_MODEL", &format!("loading model: {}", model_name), model_progress_base + 0.02);

            // Build directories.
            let prompt_dir = self.run_dir.join("prompts").join(model_name);
            let output_dir = self.run_dir.join("outputs").join(model_name);
            let log_dir = self.run_dir.join("logs");

            for dir in [&prompt_dir, &output_dir, &log_dir] {
                std::fs::create_dir_all(dir)
                    .map_err(|e| OrchestratorError::IoError(e.to_string()))?;
            }

            // The RAG delivery plan for this model, worked out in step 2c.
            // A model with no budget entry could not be configured; it was
            // already logged there, so fall back to a single RAG-free pass
            // rather than inventing a plan here.
            let budget = budgets.get(model_name);
            let rag = match budget {
                Some(b) => b.rag.clone(),
                None => crate::rag_plan::plan_rag(&[], 0),
            };
            let model_ctx = budget.map(|b| b.ctx).unwrap_or(2048);
            let passes = rag.passes();

            if passes > 1 {
                eprintln!(
                    "[orchestrator] {}: RAG needs {} passes per chapter — {} chapter(s) × {} = {} inference(s)",
                    model_name, passes, total_chunks, passes, total_chunks * passes
                );
            }

            // ── Auto-size context to actual content ─────────────────────────
            // Instead of always allocating the full user-requested context
            // (which wastes VRAM on KV cache the chapter doesn't need),
            // measure the largest chapter + the largest RAG pass, add
            // generation headroom, and use that. Freed VRAM → more GPU layers
            // → faster. The largest *pass* is the right figure, not the whole
            // library: no single prompt ever carries more than one batch.
            let max_chapter_tokens = chapters.iter()
                .map(|c| c.text.len() / CHARS_PER_TOKEN)
                .max()
                .unwrap_or(0);
            let system_prompt_tokens = rag.max_batch_tokens();
            let needed_ctx = max_chapter_tokens + system_prompt_tokens + GEN_HEADROOM
                + (max_chapter_tokens + system_prompt_tokens + GEN_HEADROOM) * SAFETY_PCT / 100;
            // Round up to nearest 2048 boundary for KV cache alignment.
            let needed_ctx = ((needed_ctx + 2047) / 2048) * 2048;
            // Cap at model config AND at this model's own effective context —
            // the split was budgeted against it, and loading larger than the
            // hardware cap OOMs/spills (the split/load numbers MUST agree,
            // that mismatch was half the context-ceiling failure).
            let right_sized_ctx = needed_ctx
                .min(model_config.context_length.unwrap_or(16384) as usize)
                .min(model_ctx.max(2048))
                .max(2048); // floor at 2K

            eprintln!(
                "[orchestrator] auto-ctx: max_chapter={}tok rag_pass={}tok gen={} → needed={} → using {}",
                max_chapter_tokens, system_prompt_tokens, GEN_HEADROOM, needed_ctx, right_sized_ctx
            );

            // Override context in the config for this model instance.
            let mut sized_config = model_config.clone();
            sized_config.context_length = Some(right_sized_ctx as u32);

            let mut instance = state_bridge::make_instance(&sized_config, self.manifest.resource_throttle.throttle_pct);

            if let Err(e) = state_bridge::load(&mut instance) {
                eprintln!("[orchestrator][ERROR] load failed for {}: {}", model_name, e);
                partial_failures.push(model_name.to_string());
                continue;
            }

            let mut chunk_outputs: Vec<(usize, PathBuf)> = Vec::new();
            let mut model_had_failure = false;

            // One inference per (chapter × RAG pass). With a single pass this
            // is exactly the old loop and the file names are unchanged.
            let mut inference_seq = 0usize;
            let total_inferences = total_chunks * passes;

            'chapters: for (ch_idx, chapter) in chapters.iter().enumerate() {
                let ch_label = if chapter.title.is_empty() {
                    format!("chapter_{:03}", ch_idx + 1)
                } else {
                    let safe: String = chapter.title.chars()
                        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                        .collect();
                    format!("{:03}_{}", ch_idx + 1, safe.trim())
                };

                for pass_idx in 0..passes {
                    // Only suffix when there is more than one pass, so existing
                    // single-pass runs keep their familiar artifact names.
                    let pass_label = if passes > 1 {
                        format!("{}_r{}", ch_label, pass_idx + 1)
                    } else {
                        ch_label.clone()
                    };

                    let system_prompt = rag.prompt_for(pass_idx);

                    // Build prompt file: system prompt + chapter text (+ findings
                    // instruction in structured-findings mode).
                    let prompt_path = prompt_dir.join(format!("{}.txt", pass_label));
                    // Ported from the test box 2026-08-17: prose mode previously sent
                    // [RAG catalog] + [chapter text] with NO instruction, so the model
                    // continued the essay instead of analysing it. build_chapter_prompt
                    // takes system_prompt as a parameter, so it composes with rag_plan's
                    // per-pass prompt unchanged.
                    let prompt_content = build_chapter_prompt(
                        &system_prompt,
                        &chapter.text,
                        findings_grammar.is_some(),
                    );

                    if let Err(e) = std::fs::write(&prompt_path, &prompt_content) {
                        eprintln!("[orchestrator][ERROR] failed to write prompt {}: {}", pass_label, e);
                        model_had_failure = true;
                        break 'chapters;
                    }

                    // Pre-flight overflow guard: NEVER send a prompt that cannot
                    // fit the loaded context with generation headroom — llama.cpp
                    // would truncate the front (the RAG instructions) and produce
                    // garbage. Skipping one pass honestly beats poisoning the
                    // whole fold with a mindless analysis.
                    let prompt_tokens_est = prompt_content.len() / CHARS_PER_TOKEN;
                    if prompt_tokens_est + GEN_HEADROOM > right_sized_ctx {
                        eprintln!(
                            "[orchestrator][ERROR] {} {}: prompt ~{}tok + gen {} exceeds loaded ctx {} — \
                             pass SKIPPED (budget bug upstream; report this)",
                            model_name, pass_label, prompt_tokens_est, GEN_HEADROOM, right_sized_ctx
                        );
                        model_had_failure = true;
                        inference_seq += 1;
                        continue;
                    }

                    let output_path = output_dir.join(format!("{}_output.txt", pass_label));
                    let log_path = log_dir.join(format!("{}_{}.log", model_name, pass_label));

                    let request = InferenceRequest {
                        chunk_id: inference_seq,
                        prompt_path,
                        output_path: output_path.clone(),
                        log_path,
                        grammar_file: findings_grammar.clone(),
                    };

                    inference_seq += 1;
                    let frac = inference_seq as f64 / total_inferences as f64;
                    let ch_progress = model_progress_base + 0.05 + frac * 0.60 / model_count as f64;
                    self.emit(
                        "RUNNING_MODEL",
                        &format!("{} chapter {}/{}{}{}", model_name, ch_idx + 1, total_chunks,
                            if chapter.title.is_empty() { String::new() }
                            else { format!(" ({})", chapter.title) },
                            if passes > 1 { format!(" — rules {}/{}", pass_idx + 1, passes) }
                            else { String::new() }),
                        ch_progress,
                    );

                    if let Err(e) = state_bridge::infer(&mut instance, &request) {
                        // Terminate Run: stop here. Not a failure to retry, and
                        // nothing after it may start (it used to "retry in 3s",
                        // and the run then reported "All models failed").
                        if model_loader::cancel::is_requested() {
                            let _ = state_bridge::unload(&mut instance);
                            return Err(OrchestratorError::Cancelled);
                        }
                        // If the GPU could not take the model, retry on the CPU and keep
                        // it there for the rest of this model's passes: a run that
                        // cannot use the GPU should be slow, not impossible. The cause
                        // is only in the pass's own llama log -- the error that reaches
                        // us is llama's last line ("unable to create context").
                        let llama_log = std::fs::read_to_string(&request.log_path).unwrap_or_default();
                        if should_retry_on_cpu(sized_config.gpu_usage.as_deref(), &llama_log) {
                            eprintln!(
                                "[orchestrator][WARN] {}/{}: the GPU could not load the model \
                                 (device memory allocation failed); running {} on the CPU \
                                 instead -- slower, but the run continues",
                                model_name, pass_label, model_name
                            );
                            self.emit(
                                "RUNNING_MODEL",
                                &format!("{}: the GPU could not load the model — continuing on the CPU (slower)", model_name),
                                ch_progress,
                            );
                            sized_config.gpu_usage = Some("CPU".to_string());
                        } else {
                            // Retry once after a brief pause — CUDA may need time to
                            // release VRAM from a prior process (stall-kill, previous run, etc.).
                            eprintln!(
                                "[orchestrator][WARN] inference failed {}/{}: {} — retrying in 3s",
                                model_name, pass_label, e
                            );
                            std::thread::sleep(std::time::Duration::from_secs(3));
                        }

                        // Re-create the instance (fresh state machine). Use the
                        // SIZED config — retrying at the unsized (user/native)
                        // context was a guaranteed-worse OOM on capped hardware.
                        let _ = state_bridge::unload(&mut instance);
                        instance = state_bridge::make_instance(&sized_config, self.manifest.resource_throttle.throttle_pct);
                        if let Err(e2) = state_bridge::load(&mut instance) {
                            eprintln!("[orchestrator][ERROR] reload failed for {}: {}", model_name, e2);
                            model_had_failure = true;
                            break 'chapters;
                        }

                        // Rebuild prompt & output paths (same paths, fresh attempt).
                        let retry_request = InferenceRequest {
                            chunk_id: request.chunk_id,
                            prompt_path: request.prompt_path.clone(),
                            output_path: request.output_path.clone(),
                            log_path: request.log_path.clone(),
                            grammar_file: request.grammar_file.clone(),
                        };

                        if let Err(e2) = state_bridge::infer(&mut instance, &retry_request) {
                            eprintln!(
                                "[orchestrator][ERROR] retry also failed {}/{}: {}",
                                model_name, pass_label, e2
                            );
                            model_had_failure = true;
                            break 'chapters;
                        }
                    }

                    chunk_outputs.push((ch_idx, output_path));

                    // After inference the instance returns to Loaded state,
                    // ready for the next pass — no reload needed.
                }
            }

            // Best-effort unload at end of model (may already be Unloaded).
            let _ = state_bridge::unload(&mut instance);

            if model_had_failure {
                partial_failures.push(model_name.to_string());
            } else {
                model_outputs.insert(
                    model_name.to_string(),
                    chunk_outputs.iter().map(|(_, p)| p.clone()).collect(),
                );
                chapter_outputs.insert(model_name.to_string(), chunk_outputs);
            }
        }

        // ── Fail-fast: abort if every model failed ──────────────────────────
        if model_outputs.is_empty() {
            let msg = if partial_failures.is_empty() {
                "All models failed — no inference output produced.".to_string()
            } else {
                format!(
                    "All models failed ({}). No inference output produced. \
                     Check that your models fit in available VRAM and that context size is valid.",
                    partial_failures.join(", ")
                )
            };
            self.emit("ERROR", &msg, 0.0);
            return Err(OrchestratorError::InferenceFailed(msg));
        }

        // ── Findings: merged once, every quote checked (ASG-Q3) ─────────────
        // In structured-findings mode every quote is checked against the
        // chapter it cites. The one checked merge feeds the synthesis, the
        // scores and the report, and is written to the run dir whether or not
        // a synthesis model runs.
        let sources = quote_sources(&chapters);
        let checked: Option<crate::findings::MergeResult> = if findings_grammar.is_some() {
            let merged = crate::findings::merge_findings(
                &findings_merge_inputs(&chapter_outputs),
                &sources,
            );
            let _ = std::fs::write(
                self.run_dir.join("findings_table.md"),
                crate::findings::render_findings_table(&merged.aggregate),
            );
            let _ = std::fs::write(
                self.run_dir.join("findings.json"),
                crate::findings::render_findings_json(&merged.aggregate),
            );
            eprintln!(
                "[orchestrator] findings merge: {} finding(s), {} rule(s), {} chapter(s) parsed, {} fell back to prose; {}",
                merged.aggregate.total_findings,
                merged.aggregate.rules.len(),
                merged.aggregate.chapters_parsed,
                merged.parse_failures.len(),
                merged.aggregate.quotes_header()
            );
            Some(merged)
        } else {
            None
        };

        // ── Step 5: Fusion ──────────────────────────────────────────────────
        let fusion_output_path = if let Some(fusion_config) = &self.manifest.models.fusion.clone() {
            if fusion_config.active && model_outputs.len() > 0 {
                self.state = OrchestratorState::RunningFusion;
                self.emit("RUNNING_FUSION", "running fusion model", 0.88);

                // Build the fusion input. In structured-findings mode this is the
                // LOSSLESSLY-merged findings table (one compact synthesis pass —
                // the Phase 2 context-ceiling fix); otherwise it's the raw
                // per-chapter prose (the existing hierarchical fold). The table
                // given to the synthesis leaves out unsupported claimed quotes.
                let fusion_input = if let Some(merged) = &checked {
                    FusionInput { blocks: synthesis_blocks(merged) }
                } else {
                    FusionInput { blocks: fold_blocks(&chapter_outputs) }
                };

                match run_fusion(fusion_config, &fusion_input, &self.run_dir, self.manifest.resource_throttle.throttle_pct) {
                    Ok(path) => Some(path),
                    // Terminated during the fold: not a finished report
                    // without its synthesis.
                    Err(_) if model_loader::cancel::is_requested() => {
                        return Err(OrchestratorError::Cancelled);
                    }
                    Err(e) => {
                        eprintln!("[orchestrator][WARN] fusion failed: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // ── Step 6: Optimization pass ────────────────────────────────────
        // Score model outputs against hitlist categories and update the
        // optimization state (aggregation, consensus, best-model map).
        // In structured-findings mode, build the rule_id→category map (union
        // across all models' packets) so scoring uses REAL per-category counts
        // instead of the keyword heuristic.
        let rule_category_map: Option<HashMap<String, String>> = if findings_grammar.is_some() {
            let model_paths: Vec<String> = plan.model_order
                .iter()
                .filter_map(|name| self.get_model_config(name).ok())
                .map(|cfg| cfg.path)
                .collect();
            Some(findings_rule_category_map(&model_paths))
        } else {
            None
        };

        let scoring = rule_category_map.as_ref().map(|rule_category| FindingsScoring {
            rule_category,
            outputs: &chapter_outputs,
            sources: &sources,
        });
        if let Err(e) = run_optimization_pass(
            &mut self.manifest,
            &model_outputs,
            &mut self.score_history,
            scoring.as_ref(),
        ) {
            eprintln!("[orchestrator][WARN] optimization pass failed: {}", e);
            // Non-fatal: the run still produces output.
        }

        // ── Step 7: Update manifest ─────────────────────────────────────────
        self.state = OrchestratorState::UpdatingManifest;
        self.emit("UPDATING_MANIFEST", "updating manifest", 0.95);

        // Record partial failures.
        if !partial_failures.is_empty() {
            self.manifest.partial_run = Some(manifest::PartialRunInfo {
                model_failures: partial_failures,
                failed_chunks: vec![],
                fusion_partial: fusion_output_path.is_none(),
            });
        }

        save_manifest(&self.manifest, &self.manifest_path)?;

        // ── Step 7: Write final PDF ─────────────────────────────────────────
        self.state = OrchestratorState::WritingFinalPdf;
        self.emit("WRITING_FINAL_PDF", "writing final PDF", 0.96);

        let synthesis = fusion_output_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .filter(|t| !t.trim().is_empty());
        let results_text = compose_report(synthesis, checked.as_ref(), || {
            // No fusion output — aggregate individual model chunk outputs.
            let titles: Vec<String> = chapters
                .iter()
                .enumerate()
                .map(|(i, c)| if c.title.is_empty() {
                    format!("Chapter {}", i + 1)
                } else {
                    c.title.clone()
                })
                .collect();
            combine_outputs_without_fusion(&chapter_outputs, &titles)
        });

        let manifest_json = manifest::serialize(&self.manifest)
            .map_err(|e| OrchestratorError::ManifestError(format!("{:?}", e)))?;

        let report_name = {
            let stem = self.manifest.source_pdf.filename
                .strip_suffix(".pdf")
                .or_else(|| self.manifest.source_pdf.filename.strip_suffix(".PDF"))
                .unwrap_or(&self.manifest.source_pdf.filename);
            let safe: String = stem.chars()
                .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' { c } else { '_' })
                .collect();
            format!("{}_AiSmartGuy_Report.pdf", safe)
        };
        let final_pdf_path = self.run_dir.join(&report_name);
        write_final_pdf(&pdf_path, &results_text, &manifest_json, &final_pdf_path)?;

        // ── Step 8: Complete ────────────────────────────────────────────────
        self.state = OrchestratorState::Completed;
        self.emit("COMPLETED", "run complete", 1.0);

        Ok(final_pdf_path)
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    fn emit(&self, stage: &str, message: &str, percent: f64) {
        emit_progress(&OrchestratorProgressEvent {
            stage: stage.to_string(),
            message: message.to_string(),
            percent,
        });
    }

    /// Look up a model config slot by name ("model1", "model2", "model3", "fusion").
    fn get_model_config(&self, name: &str) -> OrchestratorResult<manifest::ModelConfig> {
        let config = match name {
            "model1" => self.manifest.models.model1.clone(),
            "model2" => self.manifest.models.model2.clone(),
            "model3" => self.manifest.models.model3.clone(),
            "fusion" => self.manifest.models.fusion.clone(),
            other => {
                return Err(OrchestratorError::InvalidState(format!(
                    "unknown model slot: {}",
                    other
                )))
            }
        };
        config.ok_or_else(|| {
            OrchestratorError::InvalidState(format!("model slot '{}' is None in manifest", name))
        })
    }
}

/// Every model's inference outputs in order, each with the index of the
/// chapter it analysed. With a multi-pass RAG plan one chapter has several
/// consecutive outputs, so an output's position is NOT its chapter.
type ChapterOutputs = HashMap<String, Vec<(usize, PathBuf)>>;

/// Every output read back for the findings merge, labelled with its model and
/// the chapter it analysed. Passes over the same chapter share a label.
fn findings_merge_inputs(outputs: &ChapterOutputs) -> Vec<(usize, String)> {
    let mut names: Vec<&String> = outputs.keys().collect();
    names.sort();
    let mut chapters = Vec::new();
    for name in names {
        for (ch, path) in &outputs[name] {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            // The chapter index only, never the lane: the merge labels it
            // "chapter N", and that label reaches the model.
            chapters.push((*ch, text));
        }
    }
    chapters
}

/// The text each chapter's quotes are checked against: the chapter's own
/// text, never the preamble carried from the previous section. A quote the
/// model took from the preamble belongs to the chapter before.
fn quote_sources(chapters: &[pdf_io::Chapter]) -> Vec<&str> {
    chapters.iter().map(|c| c.body()).collect()
}

/// The synthesis input in findings mode: the compact table as one block, plus
/// any analysis that did not parse as its own block, headed as unchecked
/// prose (F1), so nothing is dropped.
fn synthesis_blocks(merged: &crate::findings::MergeResult) -> Vec<(String, String)> {
    let mut blocks = vec![(
        "Findings table".to_string(),
        crate::findings::render_synthesis_table(&merged.aggregate),
    )];
    for (label, prose) in &merged.parse_failures {
        blocks.push((crate::findings::unparsed_heading(label), prose.clone()));
    }
    blocks
}

/// The report text for every combination of synthesis and findings mode.
/// Findings mode heads it with the quote counts (ASG-Q3) and, when chapters
/// fell back to prose, the note that their quotes are not checked (F1).
/// Prose mode heads it with the disclosure. `prose_outputs` is called only
/// when there is neither a synthesis nor a findings merge.
fn compose_report(
    synthesis: Option<String>,
    checked: Option<&crate::findings::MergeResult>,
    prose_outputs: impl FnOnce() -> String,
) -> String {
    use crate::findings::PROSE_DISCLOSURE;
    match (synthesis, checked) {
        (Some(text), Some(merged)) => format!("{}\n\n{}", merged.report_header(), text),
        (Some(text), None) => format!("{PROSE_DISCLOSURE}\n\n{text}"),
        // No synthesis in findings mode: the checked table is the report,
        // never the raw findings JSON, whose quotes carry no label.
        (None, Some(merged)) => report_from_checked_findings(merged),
        (None, None) => {
            let combined = prose_outputs();
            if combined.trim().is_empty() {
                "Run complete. No inference output was produced.".to_string()
            } else {
                format!("{PROSE_DISCLOSURE}\n\n{combined}")
            }
        }
    }
}

/// The report body when findings were merged but no synthesis ran: the
/// checked table, then any analyses that did not parse as findings.
fn report_from_checked_findings(merged: &crate::findings::MergeResult) -> String {
    let mut text = crate::findings::render_findings_table(&merged.aggregate);
    if let Some(note) = merged.prose_fallback_note() {
        text.push_str(&format!("\n{note}\n"));
    }
    for (label, prose) in &merged.parse_failures {
        let cleaned = strip_llama_noise(prose);
        if !cleaned.trim().is_empty() {
            text.push_str(&format!("\n## {}\n\n{}\n", crate::findings::unparsed_heading(label), cleaned.trim()));
        }
    }
    text
}

/// The prose fold's blocks: every analysis, ordered by chapter (then lane,
/// then pass), headed "Chapter N, analysis K", K counting within the chapter.
/// The chapter is the one the output analysed, never its position in the
/// list: with two RAG passes over chapter 1, the fold used to call the second
/// one "chapter 2". And no lane name: the model took "fusion" for the book's
/// title.
fn fold_blocks(outputs: &ChapterOutputs) -> Vec<(String, String)> {
    let mut all: Vec<(usize, &String, usize, &PathBuf)> = Vec::new();
    for (name, list) in outputs {
        for (pass, (ch, path)) in list.iter().enumerate() {
            all.push((*ch, name, pass, path));
        }
    }
    all.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    let mut blocks = Vec::new();
    let mut current: Option<usize> = None;
    let mut k = 0;
    for (ch, _, _, path) in all {
        if current != Some(ch) {
            current = Some(ch);
            k = 0;
        }
        k += 1;
        blocks.push((format!("Chapter {}, analysis {}", ch + 1, k),
                     std::fs::read_to_string(path).unwrap_or_default()));
    }
    blocks
}

/// The report body when there is no fusion output: each model's outputs in
/// order under the heading of the chapter they analysed. `titles[i]` is the
/// heading for chapter `i`; consecutive passes over one chapter share it.
fn combine_outputs_without_fusion(outputs: &ChapterOutputs, titles: &[String]) -> String {
    let mut combined = String::new();
    let mut names: Vec<&String> = outputs.keys().collect();
    names.sort();
    for name in names {
        combined.push_str(&format!("═══ {} ═══\n\n", name));
        let mut heading: Option<usize> = None;
        for (ch, path) in &outputs[name] {
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            let cleaned = strip_llama_noise(&text);
            if cleaned.trim().is_empty() {
                continue;
            }
            if heading != Some(*ch) {
                let title = titles
                    .get(*ch)
                    .cloned()
                    .unwrap_or_else(|| format!("Chapter {}", ch + 1));
                combined.push_str(&format!("── {} ──\n", title));
                heading = Some(*ch);
            }
            combined.push_str(cleaned.trim());
            combined.push_str("\n\n");
        }
    }
    combined
}

/// Whether a failed pass should be retried on the CPU: it asked for the GPU,
/// and llama's own log says the GPU could not take the model. A pass already
/// on the CPU is never "retried on the CPU" again.
pub(crate) fn should_retry_on_cpu(gpu_usage: Option<&str>, llama_log: &str) -> bool {
    state_bridge::uses_gpu(gpu_usage) && gpu_load_failed(llama_log)
}

/// True when llama.cpp's log shows it could not allocate the model on the GPU.
/// Seen for real on this estate's GTX 1660 SUPER with the in-app Vulkan build
/// (2026-09-27): "Memory allocation of size 191439360 failed" /
/// "ErrorOutOfDeviceMemory" with 5.4 GB free, on every attempt for about
/// twenty minutes, then not again with the same memory in use. The cause is
/// unknown, so it can happen to anyone; the CPU path worked throughout.
fn gpu_load_failed(llama_log: &str) -> bool {
    const MARKERS: [&str; 5] = [
        "ErrorOutOfDeviceMemory",
        "Memory allocation of size",
        "out of memory",
        "failed to allocate",
        "unable to allocate",
    ];
    MARKERS.iter().any(|m| llama_log.contains(m))
}

/// Build the per-chapter prompt: RAG system prompt, a mode-appropriate
/// instruction, then the chapter text. Extracted so the instruction-gating
/// bug (prose mode shipped with NO instruction at all — see
/// PROSE_ANALYSIS_INSTRUCTION's doc comment) is unit-testable without
/// spawning a real model.
fn build_chapter_prompt(system_prompt: &str, chapter_text: &str, structured_findings: bool) -> String {
    if structured_findings {
        let base = if system_prompt.is_empty() {
            chapter_text.to_string()
        } else {
            format!("{}\n\n{}", system_prompt, chapter_text)
        };
        format!("{}\n\n{}", base, crate::findings::FINDINGS_INSTRUCTION)
    } else if system_prompt.is_empty() {
        format!("{}\n{}", PROSE_ANALYSIS_INSTRUCTION, chapter_text)
    } else {
        format!("{}\n\n{}\n{}", system_prompt, PROSE_ANALYSIS_INSTRUCTION, chapter_text)
    }
}

/// rule_id → category across every model in the run, built from the same
/// packets each model's prompt was built from: the shared defaults plus the
/// `rag/` folder beside its .gguf. `ModelConfig::path` names the .gguf file,
/// so it must go through `model_rag_dir` — joining `rag` onto it directly
/// gives `…/model.gguf/rag`, and every per-model rule then scores as nothing.
fn findings_rule_category_map(model_paths: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for path in model_paths {
        for (rule_id, cat) in crate::rag_bridge::build_rule_category_map(&model_rag_dir(path)) {
            map.insert(rule_id, cat);
        }
    }
    map
}

/// Whether structured findings are wanted. They are the default (manager
/// ruling F1, 2026-09-28): only `ASG_STRUCTURED_FINDINGS=0` or `false` opts out.
/// They still run only where the llama build supports grammars.
fn findings_mode_enabled() -> bool {
    findings_mode_from(std::env::var("ASG_STRUCTURED_FINDINGS").ok().as_deref())
}

fn findings_mode_from(value: Option<&str>) -> bool {
    !matches!(value.map(str::trim), Some(v) if v == "0" || v.eq_ignore_ascii_case("false"))
}

/// Strip llama-cli noise from inference output (banner art, "Loading model...",
/// timing stats, "Exiting..." etc.), keeping only the actual analysis text.
pub(crate) fn strip_llama_noise(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut in_banner = true;

    for line in text.lines() {
        // llama.cpp ends a finished answer with " [end of text]" on stdout.
        // It is never part of the answer, and it reached every report once
        // the model was asked in a chat turn and could finish (2026-09-27).
        let line = match line.trim_end().strip_suffix("[end of text]") {
            Some(rest) => rest.trim_end(),
            None => line,
        };
        let trimmed = line.trim();

        // Skip leading blank lines, ASCII banner, and interactive-mode header
        if in_banner {
            if trimmed.is_empty()
                || trimmed.starts_with("Loading model")
                || trimmed.contains('▄')
                || trimmed.contains('█')
                || trimmed.contains('▀')
                || trimmed.starts_with("llama_")
                || trimmed.starts_with("common_")
                || trimmed.starts_with("build")
                || trimmed.starts_with("model")
                || trimmed.starts_with("modalities")
                || trimmed.starts_with("available commands:")
                || trimmed.starts_with("/exit")
                || trimmed.starts_with("/regen")
                || trimmed.starts_with("/clear")
                || trimmed.starts_with("/read")
                || trimmed.starts_with("/glob")
            {
                continue;
            }
            in_banner = false;
        }

        // Skip noise lines anywhere in output
        if trimmed == "Exiting..."
            || trimmed.starts_with("[ Prompt:")
            || trimmed.starts_with("llama_perf_")
            || trimmed.starts_with("Error:")
            || trimmed == ">"
        {
            continue;
        }

        // Strip leading "> " from interactive-mode echo
        let cleaned = trimmed.strip_prefix("> ").unwrap_or(trimmed);
        if cleaned.is_empty() {
            continue;
        }

        lines.push(line);
    }

    lines.join("\n")
}

#[cfg(test)]
mod end_of_text_tests {
    use super::*;

    #[test]
    fn the_end_of_text_marker_never_reaches_the_report() {
        assert_eq!(strip_llama_noise("### Review\nThe last point. [end of text]\n"),
                   "### Review\nThe last point.");
        assert_eq!(strip_llama_noise("The answer.\n [end of text]\n"), "The answer.");
    }

    /// Control: the words themselves, inside the answer, are kept.
    #[test]
    fn the_phrase_inside_an_answer_is_kept() {
        let text = "llama prints [end of text] when it stops, then exits.";
        assert_eq!(strip_llama_noise(text), text);
    }
}

// Ported from the test box 2026-08-17 alongside build_chapter_prompt().
// These need no model: the defect was that prose mode sent no instruction at
// all, which is a pure string-assembly property and therefore unit-testable.
#[cfg(test)]
mod chapter_prompt_tests {
    use super::*;

    /// The bug this regresses: prose mode (the default — structured findings
    /// is opt-in) built its prompt as JUST [RAG system prompt] + [chapter
    /// text], with no instruction anywhere. Combined with llama-completion's
    /// raw (non-chat-templated) completion mode, the model had no signal to
    /// do anything but continue the chapter text as if it were the next part
    /// of the same document — confirmed against a real run, where the model
    /// output was a stylistic continuation of the source essay, not analysis
    /// of it.
    #[test]
    fn prose_mode_prompt_contains_an_analysis_instruction() {
        let prompt = build_chapter_prompt("RAG CATALOG HERE", "CHAPTER TEXT HERE", false);
        assert!(
            prompt.contains(PROSE_ANALYSIS_INSTRUCTION),
            "prose-mode prompt must contain an explicit instruction to analyze, \
             not just [RAG catalog] + [chapter text] with nothing telling the \
             model what to do with either"
        );
    }

    #[test]
    fn prose_mode_prompt_still_contains_the_chapter_text() {
        let prompt = build_chapter_prompt("RAG CATALOG HERE", "CHAPTER TEXT HERE", false);
        assert!(prompt.contains("CHAPTER TEXT HERE"));
        assert!(prompt.contains("RAG CATALOG HERE"));
    }

    #[test]
    fn prose_mode_orders_catalog_then_instruction_then_chapter_text() {
        let prompt = build_chapter_prompt("RAG CATALOG HERE", "CHAPTER TEXT HERE", false);
        let catalog_pos = prompt.find("RAG CATALOG HERE").unwrap();
        let instruction_pos = prompt.find(PROSE_ANALYSIS_INSTRUCTION).unwrap();
        let chapter_pos = prompt.find("CHAPTER TEXT HERE").unwrap();
        assert!(catalog_pos < instruction_pos, "catalog must come before the instruction");
        assert!(instruction_pos < chapter_pos, "instruction must come before the chapter text");
    }

    /// Even with no RAG packets loaded at all (empty system prompt), prose
    /// mode must still instruct the model to analyze — dropping the
    /// instruction silently in this edge case would reproduce the same bug.
    #[test]
    fn prose_mode_with_empty_system_prompt_still_gets_the_instruction() {
        let prompt = build_chapter_prompt("", "CHAPTER TEXT HERE", false);
        assert!(prompt.contains(PROSE_ANALYSIS_INSTRUCTION));
        assert!(prompt.contains("CHAPTER TEXT HERE"));
    }

    /// Structured-findings mode's existing behavior (JSON-only instruction
    /// appended at the end, after the raw chapter text) must be unchanged by
    /// this fix.
    #[test]
    fn structured_findings_mode_unchanged_instruction_after_chapter_text() {
        let prompt = build_chapter_prompt("RAG CATALOG HERE", "CHAPTER TEXT HERE", true);
        assert!(prompt.ends_with(crate::findings::FINDINGS_INSTRUCTION));
        let chapter_pos = prompt.find("CHAPTER TEXT HERE").unwrap();
        let instruction_pos = prompt.find(crate::findings::FINDINGS_INSTRUCTION).unwrap();
        assert!(chapter_pos < instruction_pos);
        assert!(!prompt.contains(PROSE_ANALYSIS_INSTRUCTION), "structured mode must not mix in the prose instruction");
    }
}

#[cfg(test)]
mod findings_category_map_tests {
    use super::*;

    /// A packet that exists only in one model's own `rag/` folder.
    const PER_MODEL_PACKET: &str = r#"{
        "packet_id": 900, "category": "per_model_only", "description": "t", "version": "1.0",
        "rules": [{"rule_id": "PMO-01", "name": "t", "pattern_type": "linguistic",
                   "patterns": ["x"], "severity": "low", "explanation": "t"}],
        "hit_conditions": {"min_pattern_matches": 1, "confidence_weight": 1.0},
        "model_behavior": {"inject_as": "system_prompt", "priority": 9}
    }"#;

    /// `ModelConfig::path` names the .gguf file. Findings mode built the map
    /// from `<path>/rag` (`…/model.gguf/rag`), so a model's own packet rules
    /// were never mapped and scored as nothing, while its prompt did use them.
    #[test]
    fn per_model_packet_rules_reach_the_category_map() {
        let root = std::env::temp_dir().join(format!("asg_rulecat_{}", std::process::id()));
        let rag = root.join("rag");
        std::fs::create_dir_all(&rag).unwrap();
        let gguf = root.join("model.gguf");
        std::fs::write(&gguf, b"").unwrap();
        std::fs::write(rag.join("900_per_model.json"), PER_MODEL_PACKET).unwrap();

        // Control: the fixture itself loads, so a miss below is the path.
        let direct = crate::rag_bridge::build_rule_category_map(&rag);
        let map = findings_rule_category_map(&[gguf.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(direct.get("PMO-01").map(String::as_str), Some("per_model_only"));
        assert_eq!(
            map.get("PMO-01").map(String::as_str),
            Some("per_model_only"),
            "rules from the rag/ folder beside the .gguf must be mapped"
        );
    }
}

#[cfg(test)]
mod multi_pass_label_tests {
    use super::*;

    /// One model's outputs as written by a run: `(chapter, text)` in order.
    fn write_outputs(tag: &str, outputs: &[(usize, &str)]) -> (PathBuf, ChapterOutputs) {
        let dir = std::env::temp_dir().join(format!("asg_labels_{}_{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let list = outputs
            .iter()
            .enumerate()
            .map(|(i, (ch, text))| {
                let p = dir.join(format!("out_{i}.txt"));
                std::fs::write(&p, text).unwrap();
                (*ch, p)
            })
            .collect();
        let mut map = ChapterOutputs::new();
        map.insert("model1".to_string(), list);
        (dir, map)
    }

    /// Two chapters, two RAG passes each: four outputs.
    const TWO_BY_TWO: [(usize, &str); 4] =
        [(0, "ch1 pass1"), (0, "ch1 pass2"), (1, "ch2 pass1"), (1, "ch2 pass2")];

    /// With several RAG passes, outputs were labelled by their position, so
    /// the second pass over chapter 1 was reported as "chapter 2".
    #[test]
    fn findings_labels_name_the_chapter_not_the_output_position() {
        let (dir, outputs) = write_outputs("findings", &TWO_BY_TWO);
        let inputs = findings_merge_inputs(&outputs);
        let _ = std::fs::remove_dir_all(&dir);
        let indices: Vec<usize> = inputs.iter().map(|(ch, _)| *ch).collect();
        assert_eq!(indices, [0, 0, 1, 1]);
        // The merge turns the index into the label the model sees. These
        // outputs are prose, so the labels come back on the parse failures.
        let labels: Vec<String> = crate::findings::merge_findings(&inputs, &[])
            .parse_failures
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        assert_eq!(labels, ["chapter 1", "chapter 1", "chapter 2", "chapter 2"]);
    }

    /// F1: structured findings are the default; only an explicit 0/false opts out.
    #[test]
    fn structured_findings_are_the_default() {
        assert!(findings_mode_from(None), "unset: structured");
        for on in ["1", "true", "yes", ""] {
            assert!(findings_mode_from(Some(on)), "{on:?}");
        }
        for off in ["0", "false", "FALSE", " 0 "] {
            assert!(!findings_mode_from(Some(off)), "{off:?}");
        }
    }

    fn merged_with_one_prose_chapter() -> crate::findings::MergeResult {
        let finding = r#"[{"rule_id":"F-01","quote":"remember the keyword BANANA","location":"p","severity":"low","note":"n"}]"#;
        crate::findings::merge_findings(
            &[(0, finding.to_string()), (1, "The chapter argues from authority.".to_string())],
            &["remember the keyword BANANA here", "The second chapter."],
        )
    }

    /// Every arm of the report: counts and the fallback note with a synthesis
    /// in findings mode; the disclosure first in prose mode, with or without one.
    #[test]
    fn every_report_says_what_its_quotes_are() {
        use crate::findings::PROSE_DISCLOSURE;
        let merged = merged_with_one_prose_chapter();
        let r = compose_report(Some("SYNTH".into()), Some(&merged), || unreachable!());
        assert!(r.starts_with("Quotes verified: 1 of 1\nNot verified: 0 not found"), "{r}");
        assert!(r.contains(&merged.prose_fallback_note().unwrap()), "{r}");
        assert!(r.ends_with("\n\nSYNTH"), "{r}");
        let r = compose_report(Some("SYNTH".into()), None, || unreachable!());
        assert_eq!(r, format!("{PROSE_DISCLOSURE}\n\nSYNTH"));
        let r = compose_report(None, None, || "PROSE".into());
        assert_eq!(r, format!("{PROSE_DISCLOSURE}\n\nPROSE"));
        let r = compose_report(None, None, || "  ".into());
        assert_eq!(r, "Run complete. No inference output was produced.");
        let r = compose_report(None, Some(&merged), || unreachable!());
        assert!(r.starts_with("# Findings\n\nQuotes verified: 1 of 1"), "{r}");
    }

    /// F1: the synthesis model is told which analyses are unchecked prose.
    #[test]
    fn the_synthesis_input_marks_prose_fallback() {
        let blocks = synthesis_blocks(&merged_with_one_prose_chapter());
        let headings: Vec<&str> = blocks.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(headings, ["Findings table", "Unparsed analysis of chapter 2 (prose; quotes not checked)"]);
    }

    /// F1: without a synthesis, a chapter that fell back to prose is shown
    /// under a heading that says its quotes are not checked, after the note.
    #[test]
    fn a_report_without_synthesis_discloses_prose_fallback() {
        let finding = r#"[{"rule_id":"F-01","quote":"remember the keyword BANANA","location":"p","severity":"low","note":"n"}]"#;
        let merged = crate::findings::merge_findings(
            &[(0, finding.to_string()), (1, "The chapter argues from authority.".to_string())],
            &["remember the keyword BANANA here", "The second chapter."],
        );
        let report = report_from_checked_findings(&merged);
        let note = merged.prose_fallback_note().unwrap();
        let heading = "## Unparsed analysis of chapter 2 (prose; quotes not checked)";
        assert!(report.contains(&note), "{report}");
        assert!(report.find(&note) < report.find(heading), "note before the prose: {report}");
        assert!(report.contains("The chapter argues from authority."));
    }

    /// A quote from the preamble carried into chapter 2 is chapter 1's text:
    /// cited as chapter 2 it is not found, cited as chapter 1 it verifies.
    #[test]
    fn quotes_are_checked_against_the_chapter_without_its_preamble() {
        let pdf = pdf_io::ExtractedPdf {
            pages: vec![
                "Chapter 1 First\n\nThe first chapter ends: remember the keyword BANANA.".into(),
                "Chapter 2 Second\n\nThe second chapter continues elsewhere.".into(),
            ],
            page_count: 2,
        };
        let chapters = pdf_io::split_into_chapters(&pdf, 500000, 100);
        assert_eq!(chapters.len(), 2);
        assert!(chapters[1].text.contains("remember the keyword BANANA"), "premise: the preamble carries it");
        let sources = quote_sources(&chapters);
        let finding = r#"[{"rule_id":"F-01","quote":"remember the keyword BANANA","location":"p","severity":"low","note":"n"}]"#;
        let merged = crate::findings::merge_findings(
            &[(1, finding.to_string()), (0, finding.to_string())],
            &sources,
        );
        let checks: Vec<bool> = merged.aggregate.findings.iter().map(|f| f.check.is_verified()).collect();
        assert_eq!(checks, [false, true], "chapter 2 (its preamble) no, chapter 1 yes");
    }

    /// The fold's headings: the chapter each output analysed (not its
    /// position), and no lane or stage name for the model to take as content.
    #[test]
    fn fold_blocks_are_headed_by_chapter_never_by_lane_or_position() {
        let (dir, mut outputs) = write_outputs("fold", &TWO_BY_TWO);
        let (dir2, second) = write_outputs("fold2", &TWO_BY_TWO);
        outputs.insert("fusion".to_string(), second.into_values().next().unwrap());
        let blocks = fold_blocks(&outputs);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
        let headings: Vec<&str> = blocks.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(headings, [
            "Chapter 1, analysis 1", "Chapter 1, analysis 2", "Chapter 1, analysis 3", "Chapter 1, analysis 4",
            "Chapter 2, analysis 1", "Chapter 2, analysis 2", "Chapter 2, analysis 3", "Chapter 2, analysis 4",
        ]);
        assert_eq!(blocks[0].1, "ch1 pass1", "chapter 1's text under chapter 1");
        for (h, _) in &blocks {
            assert!(!h.contains("model") && !h.contains("fusion") && !h.contains('·'), "{h}");
        }
    }

    /// The no-fusion report took each heading from the output's position:
    /// chapter 1's second pass appeared under chapter 2's title, and passes
    /// past the last chapter got invented "Chapter 3", "Chapter 4" headings.
    #[test]
    fn report_without_fusion_heads_each_chapter_once_in_order() {
        let (dir, outputs) = write_outputs("report", &TWO_BY_TWO);
        let titles = ["Intro".to_string(), "Chapter 2".to_string()];
        let body = combine_outputs_without_fusion(&outputs, &titles);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(body.matches("── Intro ──").count(), 1, "{body}");
        assert_eq!(body.matches("── Chapter 2 ──").count(), 1, "{body}");
        assert!(!body.contains("Chapter 3") && !body.contains("Chapter 4"), "{body}");
        let at = |s: &str| body.find(s).unwrap_or_else(|| panic!("missing {s:?} in {body}"));
        assert!(at("── Intro ──") < at("ch1 pass1"));
        assert!(at("ch1 pass2") < at("── Chapter 2 ──"));
        assert!(at("── Chapter 2 ──") < at("ch2 pass1"));
        assert!(at("ch2 pass1") < at("ch2 pass2"));
    }

    /// Control: a single-pass run keeps one heading per chapter, as before.
    #[test]
    fn single_pass_report_keeps_one_heading_per_chapter() {
        let (dir, outputs) = write_outputs("single", &[(0, "first"), (1, "second")]);
        let titles = ["Intro".to_string(), "Chapter 2".to_string()];
        let body = combine_outputs_without_fusion(&outputs, &titles);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(body.find("── Intro ──").unwrap() < body.find("first").unwrap());
        assert!(body.find("first").unwrap() < body.find("── Chapter 2 ──").unwrap());
        assert!(body.find("── Chapter 2 ──").unwrap() < body.find("second").unwrap());
    }
}

#[cfg(test)]
mod gpu_fallback_tests {
    use super::*;

    /// llama.cpp's own log from a real failed pass (in-app Vulkan build,
    /// GTX 1660 SUPER, 2026-09-27); only the model path is neutralised.
    const REAL_VULKAN_OOM_LOG: &[&str] = &[
    "0.00.121.343 I llama_completion: llama backend init\n",
    "0.00.121.351 I llama_completion: load the model and apply lora adapter, if any\n",
    "0.00.778.599 W load: control-looking token: 128247 '</s>' was not control-type; this is probably a bug in the model. its type will be overridden\n",
    "ggml_vulkan: Memory allocation of size 191439360 failed.\n",
    "ggml_vulkan: vk::Device::allocateMemory: ErrorOutOfDeviceMemory\n",
    "0.01.086.726 E llama_model_load: error loading model: vk::Device::allocateMemory: ErrorOutOfDeviceMemory\n",
    "0.01.086.734 E llama_model_load_from_file_impl: failed to load model\n",
    "0.01.086.738 E cmn  common_init_: failed to load model '/models/qwen2.5-1.5b-instruct-q4_k_m.gguf'\n",
    "0.01.086.742 E llama_completion: error: unable to create context\n",
    ];

    #[test]
    fn a_real_gpu_allocation_failure_is_recognised() {
        assert!(gpu_load_failed(&REAL_VULKAN_OOM_LOG.concat()));
    }

    #[test]
    fn cuda_out_of_memory_is_recognised_too() {
        assert!(gpu_load_failed("ggml_cuda_host_malloc: failed to allocate 512.00 MiB of pinned memory: out of memory"));
    }

    #[test]
    fn a_gpu_pass_that_the_gpu_refused_is_retried_on_the_cpu() {
        assert!(should_retry_on_cpu(Some("GPU"), &REAL_VULKAN_OOM_LOG.concat()));
    }

    /// A pass already on the CPU (explicitly, or by default) is not "moved to
    /// the CPU" again: it takes the ordinary retry.
    #[test]
    fn a_cpu_pass_is_never_retried_on_the_cpu() {
        assert!(!should_retry_on_cpu(Some("CPU"), &REAL_VULKAN_OOM_LOG.concat()));
        assert!(!should_retry_on_cpu(Some("cpu"), &REAL_VULKAN_OOM_LOG.concat()));
        assert!(!should_retry_on_cpu(None, &REAL_VULKAN_OOM_LOG.concat()));
    }

    #[test]
    fn a_gpu_pass_that_failed_for_another_reason_stays_on_the_gpu() {
        assert!(!should_retry_on_cpu(Some("GPU"), "llama_completion: error: unable to read prompt"));
    }

    /// Control: other failures keep the old retry (same device, after a pause).
    #[test]
    fn an_unrelated_failure_is_not_a_gpu_failure() {
        assert!(!gpu_load_failed("error: failed to open file '/prompts/chapter_001.txt'\nllama_completion: error: unable to read prompt"));
        assert!(!gpu_load_failed(""));
    }
}
