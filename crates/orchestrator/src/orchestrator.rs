use std::collections::HashMap;
use std::path::PathBuf;

use manifest::Manifest;
use model_loader::InferenceRequest;

use crate::bridge_model_fetcher::ensure_model_ready;
use crate::errors::{OrchestratorError, OrchestratorResult};
use crate::fusion::run_fusion;
use crate::manifest_bridge::{load_manifest, save_manifest, validate_manifest};
use crate::optimization_bridge::run_optimization_pass;
use crate::pdf_bridge::{chapter_split, extract_pdf, write_final_pdf};
use crate::progress::emit_progress;
use crate::rag_bridge::build_system_prompt;
use crate::sequence_plan::build_sequence;
use crate::state_bridge;
use crate::types::{FusionInput, ModelOutputs, OrchestratorProgressEvent, OrchestratorState};

/// RAG system prompt overhead (tokens).  All active packets rendered.
const RAG_OVERHEAD: usize = 6800;
/// Generation output headroom (tokens).
const GEN_HEADROOM: usize = 2048;
/// Percentage of context reserved as safety margin for token-estimation error.
/// Different tokenizers average 1.5–2.5 chars/token; 10% covers the variance.
const SAFETY_PCT: usize = 10;
/// Overlap between consecutive chapters (tokens).
const CHAPTER_OVERLAP: usize = 200;
/// Absolute minimum chunk budget if context is very small.
const MIN_CHAPTER_TOKENS: usize = 1024;
/// Rough chars-per-token estimate (conservative: most tokenizers average 1.5–2.5).
const CHARS_PER_TOKEN: usize = 2;

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

        // ── Step 2c: Determine effective context ────────────────────────────
        // user_ctx = what the user selected in the UI.
        // model_ctx = smallest native context among all active models' GGUF files.
        // effective = min(user_ctx, model_ctx) so we never exceed any model's limit.
        let user_ctx = self.manifest.models.model1
            .as_ref()
            .and_then(|m| m.context_length)
            .unwrap_or(16384) as usize;

        let mut model_native_ctx: Option<usize> = None;
        for model_name in &plan.model_order {
            if let Ok(mc) = self.get_model_config(model_name) {
                let path = std::path::Path::new(&mc.path);
                if let Some(native) = model_loader::gguf_context_length(path) {
                    let native = native as usize;
                    eprintln!("[orchestrator] {} native context: {} tokens", model_name, native);
                    model_native_ctx = Some(match model_native_ctx {
                        Some(prev) => prev.min(native),
                        None => native,
                    });
                }
            }
        }

        let effective_ctx = match model_native_ctx {
            Some(native) => {
                let eff = user_ctx.min(native);
                if eff < user_ctx {
                    eprintln!(
                        "[orchestrator] capping context: user={} model_native={} → effective={}",
                        user_ctx, native, eff
                    );
                }
                eff
            }
            None => user_ctx, // couldn't read GGUF — trust user setting
        };

        // ── VRAM-aware context cap ──────────────────────────────────────────
        // Clamp context to what the hardware can actually support so chapters
        // are split small enough to avoid OOM at inference time.
        let throttle = self.manifest.resource_throttle.throttle_pct.clamp(25, 100);
        let effective_ctx = {
            let mut cap = effective_ctx;
            for model_name in &plan.model_order {
                if let Ok(mc) = self.get_model_config(model_name) {
                    let path = std::path::Path::new(&mc.path);
                    let raw_vram = model_loader::query_vram_mb();
                    let usable_vram = (raw_vram as u64 * throttle as u64 / 100) as u32;
                    if let Some(hw_max) = model_loader::max_context_for_vram(path, usable_vram) {
                        let hw_max = hw_max as usize;
                        if hw_max < cap {
                            eprintln!(
                                "[orchestrator] VRAM cap: {} can support max {}tok (was {})",
                                model_name, hw_max, cap
                            );
                            cap = hw_max;
                        }
                    }
                }
            }
            cap
        };

        let safety_margin = effective_ctx * SAFETY_PCT / 100;
        let chapter_budget = effective_ctx
            .saturating_sub(RAG_OVERHEAD)
            .saturating_sub(GEN_HEADROOM)
            .saturating_sub(safety_margin)
            .max(MIN_CHAPTER_TOKENS);

        eprintln!(
            "[orchestrator] effective_ctx={} RAG={} gen={} safety={}({}%) → chapter_budget={} tokens",
            effective_ctx, RAG_OVERHEAD, GEN_HEADROOM, safety_margin, SAFETY_PCT, chapter_budget
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
        let mut partial_failures: Vec<String> = Vec::new();

        // ── Pre-flight: verify llama-cli is available ───────────────────────
        if model_loader::detect_llama().is_none() {
            return Err(OrchestratorError::InvalidState(
                "LLAMA_NOT_INSTALLED: llama-cli not found. Install llama.cpp to run inference.".to_string(),
            ));
        }

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

            // Build system prompt via RAG (best-effort; empty if no packets).
            let rag_dir = PathBuf::from(&model_config.path).join("rag");
            let system_prompt = build_system_prompt(&rag_dir).unwrap_or_default();
            let system_prompt_tokens = system_prompt.len() / CHARS_PER_TOKEN;

            // ── Auto-size context to actual content ─────────────────────────
            // Instead of always allocating the full user-requested context
            // (which wastes VRAM on KV cache the chapter doesn't need),
            // measure the largest chapter + system prompt, add generation
            // headroom, and use that.  Freed VRAM → more GPU layers → faster.
            let max_chapter_tokens = chapters.iter()
                .map(|c| c.text.len() / CHARS_PER_TOKEN)
                .max()
                .unwrap_or(0);
            let needed_ctx = max_chapter_tokens + system_prompt_tokens + GEN_HEADROOM
                + (max_chapter_tokens + system_prompt_tokens + GEN_HEADROOM) * SAFETY_PCT / 100;
            // Round up to nearest 2048 boundary for KV cache alignment.
            let needed_ctx = ((needed_ctx + 2047) / 2048) * 2048;
            // Cap at model config (which is already capped at model native).
            let right_sized_ctx = needed_ctx.min(
                model_config.context_length.unwrap_or(16384) as usize
            ).max(2048); // floor at 2K

            eprintln!(
                "[orchestrator] auto-ctx: max_chapter={}tok sys_prompt={}tok gen={} → needed={} → using {}",
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

            let mut chunk_outputs: Vec<PathBuf> = Vec::new();
            let mut model_had_failure = false;

            for (ch_idx, chapter) in chapters.iter().enumerate() {
                let ch_label = if chapter.title.is_empty() {
                    format!("chapter_{:03}", ch_idx + 1)
                } else {
                    let safe: String = chapter.title.chars()
                        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                        .collect();
                    format!("{:03}_{}", ch_idx + 1, safe.trim())
                };

                // Build prompt file: system prompt + chapter text.
                let prompt_path = prompt_dir.join(format!("{}.txt", ch_label));
                let prompt_content = if system_prompt.is_empty() {
                    chapter.text.clone()
                } else {
                    format!("{}\n\n{}", system_prompt, chapter.text)
                };

                if let Err(e) = std::fs::write(&prompt_path, &prompt_content) {
                    eprintln!("[orchestrator][ERROR] failed to write prompt {}: {}", ch_label, e);
                    model_had_failure = true;
                    break;
                }

                let output_path = output_dir.join(format!("{}_output.txt", ch_label));
                let log_path = log_dir.join(format!("{}_{}.log", model_name, ch_label));

                let request = InferenceRequest {
                    chunk_id: ch_idx,
                    prompt_path,
                    output_path: output_path.clone(),
                    log_path,
                };

                let ch_frac = (ch_idx as f64 + 1.0) / total_chunks as f64;
                let ch_progress = model_progress_base + 0.05 + ch_frac * 0.60 / model_count as f64;
                self.emit(
                    "RUNNING_MODEL",
                    &format!("{} chapter {}/{}{}", model_name, ch_idx + 1, total_chunks,
                        if chapter.title.is_empty() { String::new() }
                        else { format!(" ({})", chapter.title) }),
                    ch_progress,
                );

                if let Err(e) = state_bridge::infer(&mut instance, &request) {
                    // Retry once after a brief pause — CUDA may need time to
                    // release VRAM from a prior process (stall-kill, previous run, etc.).
                    eprintln!(
                        "[orchestrator][WARN] inference failed {}/{}: {} — retrying in 3s",
                        model_name, ch_label, e
                    );
                    std::thread::sleep(std::time::Duration::from_secs(3));

                    // Re-create the instance (fresh state machine).
                    let _ = state_bridge::unload(&mut instance);
                    instance = state_bridge::make_instance(&model_config, self.manifest.resource_throttle.throttle_pct);
                    if let Err(e2) = state_bridge::load(&mut instance) {
                        eprintln!("[orchestrator][ERROR] reload failed for {}: {}", model_name, e2);
                        model_had_failure = true;
                        break;
                    }

                    // Rebuild prompt & output paths (same paths, fresh attempt).
                    let retry_request = InferenceRequest {
                        chunk_id: ch_idx,
                        prompt_path: request.prompt_path.clone(),
                        output_path: request.output_path.clone(),
                        log_path: request.log_path.clone(),
                    };

                    if let Err(e2) = state_bridge::infer(&mut instance, &retry_request) {
                        eprintln!(
                            "[orchestrator][ERROR] retry also failed {}/{}: {}",
                            model_name, ch_label, e2
                        );
                        model_had_failure = true;
                        break;
                    }
                }

                chunk_outputs.push(output_path);

                // After inference the instance returns to Loaded state,
                // ready for the next chunk — no reload needed.
            }

            // Best-effort unload at end of model (may already be Unloaded).
            let _ = state_bridge::unload(&mut instance);

            if model_had_failure {
                partial_failures.push(model_name.to_string());
            } else {
                model_outputs.insert(model_name.to_string(), chunk_outputs);
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

        // ── Step 5: Fusion ──────────────────────────────────────────────────
        let fusion_output_path = if let Some(fusion_config) = &self.manifest.models.fusion.clone() {
            if fusion_config.active && model_outputs.len() > 0 {
                self.state = OrchestratorState::RunningFusion;
                self.emit("RUNNING_FUSION", "running fusion model", 0.88);

                // Read output texts for fusion input.
                let mut fusion_texts: HashMap<String, Vec<String>> = HashMap::new();
                for (name, paths) in &model_outputs {
                    let texts: Vec<String> = paths
                        .iter()
                        .map(|p| std::fs::read_to_string(p).unwrap_or_default())
                        .collect();
                    fusion_texts.insert(name.clone(), texts);
                }

                let fusion_input = FusionInput { model_outputs: fusion_texts };

                match run_fusion(fusion_config, &fusion_input, &self.run_dir, self.manifest.resource_throttle.throttle_pct) {
                    Ok(path) => Some(path),
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
        if let Err(e) = run_optimization_pass(
            &mut self.manifest,
            &model_outputs,
            &mut self.score_history,
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

        let results_text = fusion_output_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                // No fusion output — aggregate individual model chunk outputs.
                let mut combined = String::new();
                let mut sorted_models: Vec<_> = model_outputs.keys().collect();
                sorted_models.sort();
                for model_name in sorted_models {
                    if let Some(paths) = model_outputs.get(model_name) {
                        combined.push_str(&format!("═══ {} ═══\n\n", model_name));
                        for (i, path) in paths.iter().enumerate() {
                            if let Ok(text) = std::fs::read_to_string(path) {
                                let cleaned = strip_llama_noise(&text);
                                if !cleaned.trim().is_empty() {
                                    let ch_title = chapters.get(i)
                                        .map(|c| if c.title.is_empty() {
                                            format!("Chapter {}", i + 1)
                                        } else {
                                            c.title.clone()
                                        })
                                        .unwrap_or_else(|| format!("Chapter {}", i + 1));
                                    combined.push_str(&format!("── {} ──\n", ch_title));
                                    combined.push_str(cleaned.trim());
                                    combined.push_str("\n\n");
                                }
                            }
                        }
                    }
                }
                if combined.trim().is_empty() {
                    "Run complete. No inference output was produced.".to_string()
                } else {
                    combined
                }
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

/// Strip llama-cli noise from inference output (banner art, "Loading model...",
/// timing stats, "Exiting..." etc.), keeping only the actual analysis text.
pub(crate) fn strip_llama_noise(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut in_banner = true;

    for line in text.lines() {
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
