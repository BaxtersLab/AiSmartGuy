# Phase 2 — Structured Findings Pipeline (the lossless fix)

_Approved by operator 2026-07-17 (option: "Structured findings + GBNF"). Phase 1
(context-budget correctness) landed as `cdb1906`. This doc is the executor brief —
design decided, no re-derivation needed._

## Why

Even with correct budgets, the pipeline folds **prose** analyses through repeated
LLM summarization — every pass is lossy, and by the final review early-book
findings are mush. The lossless move: per-chapter output becomes a **structured
findings list**; the fold becomes a **mechanical merge (no LLM, zero loss)**; one
final LLM pass writes the review from a compact aggregate that always fits context.
Context ceiling: gone for any book length. (This also gives `optimization/scoring.rs`
the real per-category counts it already has a seam for — see its comment
"Callers with richer output can supply pre-parsed counts".)

## Build steps

1. **Findings schema + GBNF grammar** (`assets/grammars/findings.gbnf`, embedded like
   the RAG defaults). Output = JSON array of:
   ```json
   {"rule_id": "F-03", "quote": "<verbatim excerpt ≤200 chars>",
    "location": "<page/paragraph hint>", "severity": "low|medium|high",
    "note": "<≤160-char rationale>"}
   ```
   `rule_id` must match the loaded RAG packets' rule ids (`rag_engine::packet::RagRule.rule_id`).
   llama.cpp enforces the shape natively: `--grammar-file findings.gbnf`.

2. **command_builder.rs**: optional `grammar_file: Option<PathBuf>` on
   `InferenceRequest` (or ModelInstance) → emit `--grammar-file <path>` when set.
   Keep `-n 2048`. NOTE: verify the user's llama.cpp build supports the flag at
   startup (probe `llama-cli --help` once, cache result); fall back to prose mode
   if absent.

3. **Per-chapter prompt** (orchestrator): analysis instruction becomes "emit ONLY
   the findings JSON per the schema; empty array if nothing found". RAG packets
   stay as the rule catalog in the system prompt.

4. **Mechanical merge** (new `crates/orchestrator/src/findings.rs`):
   - Parse each chapter/model output (serde_json; tolerate trailing junk via the
     grammar's guarantees + a lenient extractor as backstop).
   - Aggregate: group by `rule_id`; dedupe near-identical quotes (normalized-substring
     match is enough); keep counts, per-chapter locations, max severity, exemplar
     quotes (cap ~3 per rule).
   - Emit `findings_table.md` (and `.json`) into the run dir — this replaces the
     intermediate fold passes entirely.
   - **Parse-failure fallback**: any output that fails to parse keeps its prose and
     routes through the EXISTING `fusion.rs` fold (do not delete it — it is the
     degraded-mode path and some models fight grammars).

5. **Final synthesis** (reuse `fusion.rs` plumbing, single pass): prompt = the
   aggregated findings table + "write the coherent whole-book review". The table for
   a full book is ~3–8k tokens → one pass, no hierarchy. Budget-check it with the
   Phase-1 math; if it somehow exceeds budget, truncate exemplar quotes (metadata
   survives — still lossless on findings).

6. **Scoring**: feed real per-rule counts into `optimization::compute_scores`
   (replace the keyword-mention heuristic when structured data is present).

7. **Tests** (all no-model): grammar file parses (llama-cli optional); findings
   parser on golden outputs (valid, empty, garbage); merge dedupe/severity/count
   invariants; synthesis-prompt budget check; fallback routing on parse failure.

## Later / optional
- **LEANN or similar vector index** (operator's link: StarTrail-org/LEANN): only
  for cross-chapter contradiction checks + quote verification in the final review.
  Research mission candidate AFTER this ships.
- Exact token counts via `llama-tokenize` instead of chars/2 estimates.
- Linux `.deb`: same Tauri-2 recipe as GGUF Chatbox (`SOC_umbrella_port/
  Containerfile.tauri` + `run_chatbox_build.ps1` pattern); lib tests already 94/0
  in the ubuntu:25.04 container.

## Standing rules
Verify = build + tests on BOTH OSes (containers exist). Commit local; push needs
operator auth + leak-scan. robocopy prohibited.
