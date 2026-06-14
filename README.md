<p align="center">
  <img src="frontend/assets/banner.png" alt="AiSmartGuy" width="100%">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows-0078D6" alt="Platform">
  <img src="https://img.shields.io/badge/built%20with-Rust%20%2B%20Tauri-CE412B" alt="Built with Rust + Tauri">
  <img src="https://img.shields.io/badge/inference-100%25%20offline-2EA043" alt="100% offline">
  <img src="https://img.shields.io/badge/engine-llama.cpp%20(GGUF)-555" alt="llama.cpp">
</p>

# AiSmartGuy

**AiSmartGuy is an offline desktop application that reads a book and tells you how it is trying to persuade you.**

Point it at a PDF and it performs a chapter-by-chapter critical analysis using local large language models, flagging logical fallacies, loaded language, manipulative framing, and other rhetorical techniques — then folds every chapter's findings into a single coherent review and exports a PDF report. Everything runs locally against [llama.cpp](https://github.com/ggml-org/llama.cpp); no text, no API keys, and no network connection are involved at analysis time.

---

## What it detects

Detection is driven by versioned **RAG packets** — rule sets that tell the model what to look for and how to score it. The packets shipped by default cover:

| Category | What it flags |
|---|---|
| **Logical fallacies** | Strawman, ad hominem, false dilemma, slippery slope, and other reasoning errors |
| **Weaponized language** | Loaded terms, euphemism, dog-whistles, emotionally coercive phrasing |
| **Ambiguous framing** | Vague attribution, conceptual mismatch, motte-and-bailey constructions |
| **NLP techniques** | Neuro-linguistic-programming patterns (linguistic, behavioral, cognitive, conversational) |
| **Racism & intolerance** | Stereotyping, conditional acceptance, in-group/out-group manipulation |

The catalog is number-coded and extensible — new categories can be added as packets without changing the scoring engine.

---

## How it works

```
PDF ──▶ extract text ──▶ detect chapters ──▶ ┌─ analyse each chapter ─┐ ──▶ fold ──▶ PDF report
                                             │  (1–3 local models +   │   (synthesise
                                             │   RAG rule packets)    │    one review)
                                             └────────────────────────┘
```

1. **Extract & chapter-split.** The PDF is parsed and divided into chapters (PDF outline → heading detection → size-based fallback), each sized to fit the model's context window.
2. **Per-chapter analysis.** Each chapter is sent to one or more local GGUF models with the active RAG packets injected as a system prompt. The app auto-sizes the context window and GPU-layer split to your hardware.
3. **Scoring & optimization.** Outputs are scored per category; across runs the optimizer can learn which model is strongest per category.
4. **Hierarchical fusion.** A budgeted, hierarchical *fold* consolidates the per-chapter analyses into one review — batching them into context-sized groups and summarising repeatedly — so a full-length book never overflows the model's context window.
5. **Report.** The final review is written to a PDF (with the run manifest embedded for reproducibility).

---

## Requirements

1. **[Rust](https://rustup.rs)** (stable toolchain).
2. **A llama.cpp runtime.** The app looks for `llama-completion` (falling back to `llama-cli` / `llama`) in this order:
   - `~/.aismartguy/llama-cpp/`  ← recommended
   - anything on your `PATH`

   Place a llama.cpp build — the executables **and** their `ggml*` / CUDA DLLs — in `~/.aismartguy/llama-cpp/`. Use a **CUDA** build only on machines with a matching NVIDIA GPU + driver; otherwise use a **CPU** build. This runtime is not bundled and is not downloaded automatically.
3. **At least one GGUF model** on disk (a neutral, instruction-tuned model works best for unbiased analysis).

> On first launch the app auto-seeds its RAG packets to `~/.aismartguy/rag_defaults/` — no manual setup required.

---

## Build

```sh
git clone https://github.com/BaxtersLab/AiSmartGuy
cd AiSmartGuy
cargo build --release -p aismartguy-app
```

The application binary is produced at `target/release/aismartguy-app.exe`. Launch that file directly — a previously installed copy will not contain newer changes until rebuilt.

---

## Usage

1. **Launch** `aismartguy-app.exe`.
2. **Select a model** (and, optionally, a second/third model and a fusion model) from your GGUF library.
3. **Choose a run mode:**
   | Mode | Models used |
   |---|---|
   | **Single** | One analysis model |
   | **Dual** | Two analysis models |
   | **Full** | Three analysis models |
   | **Optimized** | Three models, with per-category model selection learned from prior runs |
4. **Adjust settings** if needed — context window (or leave on **Auto**), hardware throttle, and an optional default system prompt.
5. **Load your PDF** and click **Run**. Progress is shown per stage and per chapter.
6. **Collect the report.** The output PDF is written to:
   ```
   %LOCALAPPDATA%\com.aismartguy.app\output\<run_id>\<book>_AiSmartGuy_Report.pdf
   ```
   The same folder also contains the intermediate `chapters/`, per-model `outputs/`, and the `fusion/` fold artifacts for inspection.

---

## Hardware guidance

Effective context is automatically capped to what your GPU's VRAM can hold, so **choose a model sized to your hardware**:

| VRAM budget | Sensible model size |
|---|---|
| ~6 GB | 7–8B at Q4_K_M |
| ~12 GB | 13–14B at Q4, or 8B at Q5/Q6 |
| 24 GB+ | 24B+ at Q5/Q6 |

A model larger than available memory will offload to system RAM (slower) or fail to load. Run one model in VRAM at a time.

---

## Project structure

```
crates/
  utils, logging, error_system   foundation
  manifest                       run configuration (models, mode, RAG map) as JSON
  rag_engine                     loads/merges RAG packets → system prompt
  pdf_io                         PDF extraction, chapter detection, chunking
  model_fetcher                  optional GGUF downloader (Hugging Face)
  model_loader                   llama.cpp process control + GGUF metadata
  optimization                   per-category scoring / consensus
  orchestrator                   run loop: extract → split → analyse → fold
  ui                             Tauri command/bridge layer
src-tauri/                       Tauri application entry (binary: aismartguy-app)
assets/rag_defaults/             bundled RAG packets (embedded at compile time)
frontend/                        static HTML/CSS/JS interface
```

Each crate is self-contained — the workspace deliberately avoids cross-crate code sharing so modules stay independently testable.

---

## Testing

```sh
cargo test --workspace --lib      # unit tests (no model required)
```

---

## Status

Pre-release, entering live testing. Built and tested on Windows against CUDA and CPU llama.cpp builds.
