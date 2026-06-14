# AiSmartGuy

Offline, deterministic, local-LLM analysis of long documents (books, reports).
A PDF is split into chapters; each chapter is analysed by one or more local GGUF
models against a set of "RAG packets" (rules for logical fallacies, weaponized
language, manipulative framing, etc.); the per-chapter analyses are then folded
into a single coherent review.

Built as a Rust workspace with a Tauri desktop front-end. No network calls are
required at run time — everything runs against local `llama.cpp` and local model
files.

## Architecture

```
crates/
  utils, logging, error_system   foundation
  manifest                       run config (models, mode, RAG map) as JSON
  rag_engine                     loads/merges RAG packets → system prompt
  pdf_io                         PDF extract, chapter detection, chunking
  model_fetcher                  optional GGUF downloader (Hugging Face)
  model_loader                   llama.cpp process control + GGUF metadata
  optimization                   per-category scoring / consensus
  orchestrator                   the run loop: extract → split → infer → fold
  ui                             Tauri command/bridge layer
src-tauri/                       Tauri app entry (binary: aismartguy-app)
assets/rag_defaults/             bundled RAG packets (embedded at compile time)
frontend/                        static HTML/CSS/JS UI
```

The book-review step uses a **budgeted hierarchical fold**: per-chapter analyses
are batched into groups that fit the fusion model's context window, each group is
summarised, and the summaries are folded again until one review remains — so a
full book never overflows the context window.

## Prerequisites

1. **Rust** (stable) — https://rustup.rs
2. **llama.cpp runtime** — the app looks for `llama-completion.exe` (falling back
   to `llama-cli` / `llama`) in this order:
   - `~/.aismartguy/llama-cpp/`  ← recommended location
   - anything on your `PATH`

   Copy a llama.cpp build (the executables **and** its `ggml*.dll` / CUDA DLLs)
   into `~/.aismartguy/llama-cpp/`. Use a **CUDA** build only if the machine has a
   matching NVIDIA GPU + driver; otherwise use a **CPU** build. This runtime is
   **not** part of the repo and is not auto-downloaded.
3. **At least one GGUF model** on disk, selectable from the UI.

On first launch the app auto-seeds the bundled RAG packets to
`~/.aismartguy/rag_defaults/` — no manual step needed.

## Build

```sh
cargo build --release -p aismartguy-app
```

Output: `target/release/aismartguy-app.exe`. Launch that binary directly (a
previously installed copy will not contain newer changes until rebuilt).

## Test

```sh
cargo test --workspace --lib      # unit tests (no model required)
```

## Hardware notes

Effective context is auto-capped to what the GPU's VRAM can hold, so **pick a
model that fits your hardware**:

| VRAM / RAM budget | Sensible model size |
|-------------------|---------------------|
| ~6 GB             | 7–8B at Q4_K_M      |
| ~12 GB            | up to ~13–14B at Q4, or 8B at Q5/Q6 |
| 24 GB+            | 24B+ at Q5/Q6       |

A model larger than available memory will offload to CPU/RAM (slow) or fail to
load entirely. Run one model in VRAM at a time.
