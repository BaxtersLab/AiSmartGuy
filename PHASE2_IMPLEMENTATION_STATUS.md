# Phase 2 — implementation status (2026-07-30, Windows box)

Phase 2 (structured findings + GBNF) from `PHASE2_STRUCTURED_FINDINGS_DESIGN.md` is **implemented and
verified on Windows by no-model tests**, LOCAL + UNCOMMITTED. This note is for whoever picks it up on
the Linux box.

## Done (all 7 build steps)
1. **Grammar** — `assets/grammars/findings.gbnf` + embedded via `include_str!` as
   `orchestrator::findings::FINDINGS_GBNF`.
2. **Grammar seam** — `grammar_file: Option<PathBuf>` on `model_loader::InferenceRequest`;
   `command_builder` emits `--grammar-file`; `model_loader::llama_detect::llama_supports_grammar()`
   probes `--help` once (cached) and falls back to prose if unsupported.
3. **Per-chapter prompt** — `orchestrator::findings::FINDINGS_INSTRUCTION` + grammar wired into the
   analysis loop, **gated by `ASG_STRUCTURED_FINDINGS=1` AND grammar support; DEFAULT OFF** (the prose
   pipeline is unchanged unless opted in).
4. **Mechanical merge (the heart)** — `crates/orchestrator/src/findings.rs`: `parse_findings` (serde +
   lenient `[...]` extractor), `merge_findings` (aggregate by rule_id: count, max severity, dedup'd
   locations, ≤3 near-dup-collapsed exemplars, parse-failure→prose fallback list),
   `render_findings_table` / `render_findings_json`, `per_rule_counts`. **15 unit tests.**
5. **Synthesis** — in findings mode, the merged table is fed to the existing `run_fusion` as a single
   leaf → one synthesis pass; `findings_table.md` + `findings.json` written to the run dir; unparsed
   chapters ride along as prose leaves (nothing dropped).
6. **Scoring** — `optimization::compute_scores_from_counts(model→category→hits)` + test.

## Verified here (Article IX)
`cargo test -p orchestrator -p model_loader -p optimization` → **orchestrator 23/0, model_loader 7/0,
optimization 17/0**. `cargo build --workspace` → **green** (incl. src-tauri/Tauri). No new warnings.

## Step 6 — NOW ACTIVATED (2026-07-30, was deferred; done on the Windows box)
The structured counts now feed the *live* scoring: `FindingsAggregate::per_category_counts(rule→cat map)`
+ `rag_bridge::build_rule_category_map` (rule_id→category from the same merged packets as the prompt);
`run_optimization_pass` takes an `Option<&HashMap<rule_id,category>>` — in findings mode the orchestrator
builds the union map across models and passes it → `compute_scores_from_counts` replaces the keyword
heuristic (keyword path unchanged when None). Tested (per_category_counts mapping + scoring). **All 6
build steps are now complete + unit-tested.**

## Remaining for the Linux box (in order)
1. **Both-OS gate** — run the same `cargo test` in the ubuntu:25.04 container (the standing rule; Phase 1
   was 94/0 both OSes). Confirm green.
2. **Live LLM path (the ONLY UNVERIFIED part)** — needs a real model + a `--grammar-file`-capable
   llama.cpp build. Run with `ASG_STRUCTURED_FINDINGS=1` on a real book; confirm: per-chapter output is
   findings JSON, `findings_table.md`/`findings.json` are produced, the synthesis reads well, scoring
   reflects the structured counts, and parse-failures fall back to prose. Tune `FINDINGS_INSTRUCTION`
   if a model fights the grammar.
3. **Commit local** (scoped to the Phase-2 files), then **push needs operator auth + a leak-scan.**
4. Optional/later: exact `llama-tokenize` counts; LEANN (StarTrail-org/LEANN) for cross-chapter
   contradiction checks; the Linux `.deb` (same Tauri-2 recipe as GGUF Chatbox `Containerfile.tauri`).

## Files changed
NEW: `assets/grammars/findings.gbnf`, `crates/orchestrator/src/findings.rs`, this file.
MODIFIED: `crates/orchestrator/{Cargo.toml,src/lib.rs,src/fusion.rs,src/orchestrator.rs,src/rag_bridge.rs,src/optimization_bridge.rs}`,
`crates/model_loader/src/{types.rs,command_builder.rs,llama_detect.rs}`,
`crates/optimization/src/{lib.rs,scoring.rs}`, `Cargo.lock`.

## Verified here (Article IX) — updated
`cargo test` → orchestrator **24/0** (16 findings incl. per_category_counts), optimization **17/0**,
rag_engine **13/0**, model_loader **7/0**. `cargo build --workspace` **green** (incl. Tauri src-tauri).
