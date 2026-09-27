# handoffs — AiSmartGuy

_Append-only (Article VIII). Newest entry at the top._

## [2026-08-05] — The ✕ button: measured, not guessed. **The 2026-08-04 fix was wrong AND inert — do not port it.**

⚠️ **Windows fork: the previous entry told you to port everything. The `startup_scan`
size-nudge is the exception — it has been REMOVED. It never worked.**

### What the 2026-08-04 entry claimed, and why it was wrong

It claimed the ✕ was dead because showing a `"visible": false` window leaves the
compositor holding a **stale input region**, and that
`set_size(h+1); set_size(h)` forces a recompute. Both halves were measured on
2026-08-05 and both are false:

* A GTK 3.24.52 window created hidden and shown later reports **exactly** the
  same geometry as one created visible — `alloc=1012x889`, `frame=1012x889`,
  inset `(0,0)`. Nothing is stale. (PyGObject repro against the same GTK, with
  a real `WebKit2.WebView` child.)
* On Wayland GTK clamps to the size the compositor configured, so the two
  `set_size` calls **never changed the allocation at all**. The nudge was a
  no-op that could not have fixed anything.
* The input region is also correct. From a `WAYLAND_DEBUG=1` trace, main
  (`wl_surface#55`) sets geometry `(26, 23, 960, 847)` and input region
  `(16, 13, 980, 867)` — the window plus a 10px grab border. The titlebar
  (y 23–70) is comfortably inside it.

The removed code and the reasoning are recorded in `startup_scan`.

### The actual cause — a ~26px hit-test offset from the CSD shadow

Captured a `WAYLAND_DEBUG` trace while the operator clicked, and correlated every
`wl_pointer.button` with the preceding motion and the live window geometry:

```
UNMAXIMIZED   geometry (26, 23, 960, 847)      [26px client-drawn shadow inset]
  PRESS/RELEASE  surface (962.4, 47.9)  -> nothing
  PRESS/RELEASE  surface (968.4, 45.1)  -> nothing
  PRESS (double) surface (969.3, 44.2)  -> MAXIMIZED

MAXIMIZED     geometry (0, 0, 1853, 1048)      [no inset]
  PRESS/RELEASE  surface (1822.9, 22.1) -> CloseRequested
```

**A double-click on the ✕ maximized the window.** That is the proof: GTK sees
*titlebar background* at that point, not the close button.

The arithmetic pins it. Maximized, the working click was `1853 − 1823 = 30px`
from the right edge — the normal Adwaita button position. Unmaximized the button
is painted at that same 30px from the window's right edge (surface ≈956), yet
clicks at surface 962–969 hit nothing. Laying the titlebar out against the
**surface** width (1012) while clicks arrive in **window** coordinates puts the
hit area at surface ≈970–994 — about **26px right of where it is drawn**, exactly
the shadow inset. Maximizing zeroes the inset, painted and hit-tested coincide,
and the button works.

This is a GTK3-CSD-under-mutter bug. It cannot be fixed from inside the app.

### Two other findings, both settled

**There is no minimize button.** `org.gnome.desktop.wm.preferences button-layout`
is `'appmenu:close'`; GTK resolves `gtk-decoration-layout` to `'menu:close'`.
No app change can add one — it is a desktop-wide GNOME setting:
`gsettings set org.gnome.desktop.wm.preferences button-layout 'appmenu:minimize,maximize,close'`

**Server-side decorations are not available on this desktop.** mutter advertises
no `zxdg_decoration_manager_v1` (full global list checked), and GTK3 does not
implement it anyway. CSD is mandatory under Wayland here.

### `run.sh` was scrubbing 4 snap variables out of 21 — fixed

The missed ones sit in GTK's own load path: `GTK_PATH`, **`GTK_IM_MODULE_FILE`**
(the input-method path), `GTK_EXE_PREFIX`, `GIO_MODULE_DIR`, `GDK_PIXBUF_MODULEDIR`,
`GDK_PIXBUF_MODULE_FILE` and `LOCPATH`. `LOCPATH` is the loudest — it drags
`/snap/core20`'s glibc into the process and produces
`symbol lookup error: ... undefined symbol: __libc_pthread_init`; it killed
`gnome-screenshot` and `python3` twice during this session. `run.sh` now strips
every variable whose value points into `/snap/`, and **filters** `XDG_DATA_DIRS`
rather than unsetting it (it legitimately holds system paths).
Verified: all harmful vars gone, only inert `SNAP_*` metadata remains, and
`/snap/bin` sits at position 9 in `PATH` so it shadows nothing.
**Cross-platform note: this is Linux-only, do not port.**

### Window-event tracing added

There was **no `on_window_event` handler at all**, which is why this was
undiagnosable — nothing recorded whether a click ever reached the app. Added to
the builder in `main.rs`; it logs CloseRequested / Destroyed / Focused / Resized
/ Moved. Worth keeping. It immediately ruled out two suspects: the splash is
properly `CloseRequested → Destroyed` (not lingering always-on-top), and main
does hold focus.

### Fix — `export GDK_BACKEND=x11` in `run.sh`. **OPERATOR-CONFIRMED.**

Under XWayland the window gets **server-side decorations** — `_GTK_FRAME_EXTENTS`
is absent, mutter draws the titlebar — so there is no client shadow and no
offset. Operator verified: **"x to close works now"**, one click, no maximizing
first. Trace shows `CloseRequested → Destroyed` and a clean exit.

Verified again through `run.sh` after the change landed:

| check | Wayland (before) | X11 (now) |
|---|---|---|
| `_GTK_FRAME_EXTENTS` | present (CSD) | **absent (SSD)** |
| reported size | `1012x899` (surface + shadow) | **`960x800`** (true) |
| reported position | `Moved(0,0)` — unknowable | **`Moved(737,243)`** — real |
| ✕ without maximizing | dead | **works** |

Three things come with it, all improvements: the ✕ works; a minimize button
exists (mutter's titlebar — under Wayland CSD, `button-layout='appmenu:close'`
means there is none at all); and `center()` works, since a Wayland client may
not position itself — which is also why the old "opens off the bottom edge"
fix never took.

Safe for **this** app: no screen capture (unlike SOC Ultralight, where an X11
fallback silently captures black) and no HiDPI concern at 1920x1080 @1x. The
full reasoning and the measured numbers are in the `run.sh` header so the next
reader does not undo it.

**The splash's drop shadow disappears — this is correct, do not "restore" it.**
The operator called it exactly right: it was never intentional. GTK adds a
shadow to CSD windows unasked and then declares it part of the window geometry
(`set_window_geometry(0, 0, 852, 302)` for a requested 800x250), which is also
why the banner's alignment shifts. Its disappearance is the glitch going away.

Rejected alternative: `decorations: false` plus a custom HTML titlebar. Would be
Wayland-native but is materially more work, and X11 solves it today.

**This is a workaround, not a root-cause fix.** The bug is in GTK3 CSD under
mutter and cannot be fixed from inside the app.

### Verified

* `cargo build --manifest-path src-tauri/Cargo.toml` → exit 0, no new warnings.
* `cargo test --workspace` → **193 passed, 0 failed** (unchanged by this work).
* Operator-confirmed on hardware: ✕ closes on first click.

Not committed.

## [2026-08-05] — CONTEXT_TOO_SMALL fixed: the RAG library now batches across passes — **PORT ALL OF THIS TO WINDOWS**

**Cross-platform. No Linux-specific code in this entry.**

The operator hit this on a real book:

```
CONTEXT_TOO_SMALL: hardware-capped context is 8192 tokens but the RAG packets
(20833) + generation headroom (2048) + safety (819) leave only 0 for chapter
text (minimum 1024). Fix: disable some RAG packet categories, raise the
hardware throttle, or use a smaller/more-quantized model.
```

**All three suggested fixes were dead ends**, and the diagnosis in the message
was wrong. The throttle was already 100%. A smaller model does not raise a
*native* context limit. And disabling categories means silently analysing the
book against fewer rules — the opposite of the point.

### What was actually wrong

Lane models and their native contexts: DeepSeek-V4-Pro-Qwen3.5-9B **262144**,
mistral-7b **32768**, llama-3-8b **8192**, MiniCPM5-1B fusion **131072**.

1. **One small model capped every model.** `orchestrator.rs` took the
   `min()` native context across the whole lineup and applied it globally, so
   llama-3-8b's 8192 throttled a model with 32× the context. Nothing required
   this: models run strictly sequentially, each loading its own context.
2. **The RAG prompt was unbounded.** 69 rules across 5 packets render to
   **20833 tokens** — 2.5× llama-3-8b's *entire* window. Nothing budgeted the
   prompt against the context, so the subtraction went to zero and the run died
   before reading a single page.
3. **Per-model RAG packets could never load.** `rag_bridge` built the packet dir
   as `PathBuf::from(&mc.path).join("rag")`, but `ModelConfig::path` names the
   **`.gguf` file**, so it produced `…/model.gguf/rag` — a path that cannot
   exist. Every model silently got only the shared defaults, with no error.

Measured composition of the prompt, which is what made the fix obvious:
explanations are **11277 of the 20833 tokens (54%)**; ids, names, severities and
patterns are only ~9500.

### The fix — operator chose "compact rendering, then batch"

New **`crates/orchestrator/src/rag_plan.rs`**. Three tiers, best first:

1. **Full detail, one pass** — every field including explanations.
2. **Compact, one pass** — explanations dropped, every rule kept.
3. **Compact, N passes** — rules packed into as many prompts as it takes; each
   chapter is analysed once per batch and the outputs merge downstream like any
   other per-chapter output.

**No rule is ever dropped.** `rules_covered() == total_rules` is asserted across
a sweep of budgets down to zero. Tier 3 costs wall-clock time and nothing else.

Packing is at **rule** granularity, not packet granularity — packet granularity
would have left the operator's 8192 case unsatisfiable, because one packet alone
renders to ~2950 compact tokens. A rule too large to fit alone is still emitted,
in its own batch, and named in `oversized_rules` rather than discarded.

`plan_rag_for_context` picks the RAG share by **sweeping** it, because the
optimum is interior: work ∝ `passes / chapter_capacity`, so a bigger RAG share
buys fewer passes but shrinks chapters. **Detail is never traded for speed
silently** — full detail in one pass wins outright whenever it fits and leaves a
comfortable chapter budget; only when it cannot is the share swept.

Also in this change:

* **Per-model context.** Each model now gets `min(user setting, native limit,
  VRAM)` computed for itself, and the orchestrator records *which* of the three
  bound it so the error can say so.
* **`rag_bridge::model_rag_dir()`** takes the parent when the path names a file.
  Per-model packet overrides can load for the first time.
* **Honest error.** If a model genuinely cannot be made to fit, the message now
  names *which* model is limiting, what bound its context, and says the RAG
  library is already being split so the fix is the model, not the packets.
* **`crates/orchestrator/examples/rag_probe.rs`** — diagnostic companion to
  `ctx_probe`. Prints the plan each model would get against the real library.

`chunk_id` is now a running counter across (chapter × pass) rather than the
chapter index, so it stays unique. Artifact names are unchanged when there is
only one pass; multi-pass runs suffix `_r1`, `_r2`, ….

### Verified

* `cargo build --manifest-path src-tauri/Cargo.toml` → **exit 0** (3 pre-existing
  warnings, no new ones).
* `cargo test --workspace` → **193 passed, 0 failed** (was 170; +23 new).
* `rag_probe` against the **real** library and the operator's four actual models:

| model | ctx in use | bound by | plan | chapter capacity |
|---|---|---|---|---|
| DeepSeek-V4-Pro-Qwen3.5-9B | 16384 | context setting | compact, **2 passes**, max 4901 tok | 7797 |
| mistral-7b-uncensored | 16384 | context setting | compact, **2 passes**, max 4901 tok | 7797 |
| llama-3-8b-uncensored | 8192 | **model's trained context** | compact, **4 passes**, max 2661 tok | **2664** |
| MiniCPM5-1B (fusion) | 16384 | context setting | compact, **2 passes**, max 4901 tok | 7797 |

All four report **69/69 rules covered**. Chapter budget for the run is 2664
tokens, set by llama-3-8b, and the probe's verdict is **"run is viable"** where
the same lineup previously produced a hard CONTEXT_TOO_SMALL.

### NOT verified

* **No end-to-end run has been done on this code.** The probe proves the budget
  math against real GGUF metadata and the real packet library; it does not prove
  the book comes out the other side. The operator dropping a PDF is the real test.
* The three UI fixes from 2026-08-04 (library browse/paste, Terminate Run, ✕
  close) are **still unconfirmed** — the app was restarted onto this build, so
  they are now exercisable.
* Not committed. Not copied to the file cabinet (only proven work goes there).

### Open

* `fusion.rs` keeps its own `CHARS_PER_TOKEN` and `FOLD_INSTR_OVERHEAD`; the fold
  budget was left alone deliberately, but it is the same class of hardcoded
  estimate that caused this bug and should get the same treatment.
* Multi-pass runs produce `chapters × passes` leaves for the fold. That is
  correct but slower; if fold time becomes the bottleneck, merging the passes
  for one chapter before folding is the obvious next step.
* `16384` is still hardcoded in six places (carried over from 2026-08-03).

## [2026-08-04] — Model library is selectable; Linux launch fixed — **PORT ALL OF THIS TO WINDOWS**

**For the Windows-side agent: every change below is cross-platform and belongs
on main.** Nothing here is Linux-specific except the `run.sh` launcher and the
`schema-shim/`, and both are called out as such. The Rust and HTML changes
compile and behave identically on Windows.

### 1. The model library was unselectable — the core fix

`model_library_dir()` was hardcoded to `app_local_data_dir()/models`. Models are
5–15 GB and live on a separate disk, so the only way to use one was to copy it
into the app's private data folder. Lane 1 ("Model Library Folder") displayed
the path in a **`readonly` input with no picker**, which is what the operator hit
as "it won't let me browse or paste".

**New in `src-tauri/src/main.rs`:**

| item | what it does |
| --- | --- |
| `library_pref_path(app)` | `<app data>/model_library.txt` — where the choice is stored |
| `model_library_dir(app)` | now reads that override first; falls back to the default if unset **or if the saved folder no longer exists** (an unplugged drive must not brick the app) |
| `cmd_browse_model_library` | native folder picker via `rfd`; `Err("cancelled")` on dismiss |
| `cmd_set_model_library(path)` | typed/pasted path; **rejects a non-directory rather than storing it** — a stored bad path would silently fall back to the default and the operator would never learn their entry was refused |
| `cmd_reset_model_library` | forget the override |
| `cmd_model_library_info` | `{path, default_path, is_override}` — see §3 |

New dependency: **`rfd = "0.14"`** (already used by GGUF-Chatbox). Uses the XDG
portal on Linux and the native picker on Windows — no platform code needed.

**`frontend/index.html`:** lane 1 input is no longer `readonly`; added
`📁 Browse…` and `↺` reset; commits a typed path on **Enter and on blur** (so
paste-then-click-away is not silently discarded); on failure it restores the
field to the path actually in force, so the box never shows a path the app is
not using. Changing the library re-runs `populateLaneDropdowns()` immediately —
otherwise the lanes keep listing the previous folder's models.

### 2. Symlinked model folders were silently skipped — real bug

`cmd_list_library_subfolders` filtered with `item.file_type()?.is_dir()`.
**`DirEntry::file_type` does not follow symlinks**, so a symlinked model folder
reports as a symlink and was dropped with no error. Now uses
`item.path().is_dir()`, which follows. This matters on both platforms (Windows
directory junctions behave the same way) and linking a big library into place is
the obvious alternative to copying gigabytes.

### 3. Default vs override conflict — the operator raised this, it is real

The **fetcher downloads into the default folder** while the **lanes list the
effective one**. Point the library elsewhere and a fetched model succeeds and
then does not appear, with nothing on screen to explain why.

Not silently reconciled, because either choice is defensible and guessing would
be worse. Instead `cmd_model_library_info` returns all three facts and lane 1
states which is in force:

* default: *"Default library. Browse or paste a path to point at your own."*
* override: *"Custom library in use. Fetched models still download to the
  default (`<path>`), so move or re-point after fetching."*

**Open decision for the Windows fork:** should the fetcher download into the
*effective* library instead? That would remove the split entirely. It was not
changed here because it alters where existing installs put files.

### 4. "Open Output 📂" button

Next to *Lock for 10 Runs*. Calls the **already-existing**
`cmd_open_output_folder` with `path: ''` (which resolves to the output root).
The command was there; nothing invoked it before a run finished, so old reports
could not be cleared on the way in.

### 5. Terminate Run — the cancel path was built but never connected

The operator's only way to stop a run was to close the application.

Everything needed already existed and simply was not joined up:
`ModelInstance` carried `cancel: Arc<AtomicBool>`, `timeout::enforce_timeout`
polled it and returned *"inference cancelled by caller"*, and `cmd_cancel_run`
was registered. But **`ModelInstance::new` minted a fresh flag per instance**
and nothing upstream held a reference, so the flag the UI set and the flag the
inference loop read were never the same object. `cmd_cancel_run` stored `true`
into a value no one would ever read.

* **New `crates/model_loader/src/cancel.rs`** — one process-wide flag
  (`flag()` / `request()` / `clear()` / `is_requested()`), 2 unit tests, one of
  which asserts the exact property the old code lacked: a value set through one
  handle is visible through another. A run is a strictly sequential fold, one
  llama child at a time, so a global is honest about what the pipeline is.
* `ModelInstance::new` now takes `cancel::flag()` instead of a fresh `AtomicBool`.
* **`run_inference` bails before spawning** when cancellation is requested —
  otherwise cancel would stop only the pass in flight and the next would start
  immediately, looking like the run ignored the operator.
* `cmd_cancel_run` sets both the old `CancelFlag` (nothing that reads it
  changes) and the model_loader flag (the one actually polled).
* **`cancel::clear()` at all four `start_run` call sites** — without it a
  cancelled run leaves the flag set and the *next* run dies instantly with no
  explanation.
* Frontend: **■ Terminate Run** on the processing screen, next to Back. Logs a
  line to the run terminal and re-arms after 4 s rather than sitting on
  "Terminating…", which would read as a hang during a long step.

Partial output is left on disk deliberately — terminating stops work, it does
not roll it back.

### 6. Close button needed a resize before it worked — Linux symptom, cross-platform fix

Reported as: the ✕ does nothing until the window is resized, then needs several
clicks. No `prevent_close` handler exists, so nothing was blocking it in code.

Cause: `main` is declared `"visible": false` and shown from `startup_scan`.
Under Wayland with GTK client-side decorations that leaves the compositor
holding a **stale input region** — the titlebar buttons are painted in one place
and hit-tested in another, and a resize is precisely what forces recomputation.

Fix in `startup_scan`, after `w.show()`: set the size one pixel different and
straight back, then `center()` and `set_focus()`. Centring after showing also
fixes the window opening half off the bottom edge — a hidden window has no
reliable geometry to centre against.

Harmless on Windows, so it is left unconditional rather than `cfg`-gated.

### 7. Linux-only — do NOT port these two

* **`run.sh`** — Linux launcher.
* **`schema-shim/`** — GNOME 50 moved `antialiasing`/`hinting`/`rgba-order` into
  a `.deprecated` schema while WebKitGTK 2.52.3 still reads `antialiasing` from
  the original id. A missing GSettings key is fatal, so **every Tauri app on
  Ubuntu 26.04 aborts at startup**. The shim redefines that schema id as a
  superset; `run.sh` points `GSETTINGS_SCHEMA_DIR` at it. No root, no system
  files, reversible by deleting the folder. Windows is unaffected.

**One Linux finding worth knowing anyway:** VS Code's snap exports
`XDG_DATA_HOME=/home/<user>/snap/code/<rev>/.local/share`. Tauri derives
`app_local_data_dir()` from it, so anything launched from a VS Code terminal put
its library, settings and output **inside the snap sandbox** — silently, and at
a path that moves whenever VS Code updates. `run.sh` now clears any `XDG_*` that
points into a snap. If the Windows fork ever grows a sandboxed launcher, the
same class of bug applies.

### Status

Builds clean. Library override, browse, typed-path commit, symlink following and
the output button are all in. **Not yet exercised end-to-end by a run** — the
operator was mid-run on three models when this landed, so the app was
deliberately not restarted. First restart is the real test.


## [2026-08-03] — Pipeline confirmed running; installer fixed; context UI now tells the truth

**Operator-confirmed: the full pipeline runs and generates** (`[46%] model1
chapter 1/4`, `ctx=32768 gpu_layers=24`). This supersedes the "NOT VERIFIED: PDF
ingestion, model fetch, model load, RAG query" line in the entry below.

### llama.cpp installer — three defects, all fixed

The operator hit `llama.cpp install failed: read llama.cpp failed: timed out
reading response`. Unblocked immediately by copying GGUF-Chatbox's already-
verified binaries (`cp -a ~/.gguf-chatbox/llama-cpp/. ~/.aismartguy/llama-cpp/`
— `-a` matters, the SONAME symlinks must survive). Then fixed properly:

1. **Windows-only release assets** (`bin-win-cuda-12.4-x64`, `.zip`). Even on
   success it would have installed Windows binaries. Now selects
   `bin-ubuntu-vulkan-x64` / `bin-ubuntu-x64` / `.tar.gz` on Linux — there is no
   CUDA build published for Ubuntu, so Vulkan is the accelerated path.
2. **One 30-second agent for both the API call and the download.** Correct for a
   JSON request, hopeless for a few hundred MB — this was the literal cause of
   the operator's timeout on a healthy connection. Now a separate download agent
   at 600s, matching Chatbox.
3. **No `.tar.gz` path and no symlink handling.** Same bug already found and
   fixed in Chatbox: `entry_type().is_file()` is false for symlinks, so an
   extractor that filters on it drops every SONAME link. The install "succeeds"
   and each binary then dies with `error while loading shared libraries:
   libggml-base.so.0`.

`cudart_url` is now gated to Windows. Verified against the live release list:
`[PASS] linux nvidia -> llama-b10246-bin-ubuntu-vulkan-x64.tar.gz`,
`[PASS] linux fallback -> llama-b10246-bin-ubuntu-x64.tar.gz`.

### Real KV math ported from GGUF-Chatbox

`kv_bytes_per_token()` / `kv_reserve_mb()` now read the model's own GGUF
metadata (block_count, head_count_kv with GQA, key_length or embd/heads) instead
of a fixed constant, and `max_context_for_vram` was corrected. Added
`crates/model_loader/examples/ctx_probe.rs` as a standalone diagnostic; it
produced the numbers that settled the operator's CONTEXT_TOO_SMALL report —
MiniCPM5-1B is 24576 B/token, so ctx 32768 costs 768 MB with all 24 layers
resident, not the multiple GB the UI implied.

### Context-size UI: labels were ~3.5-5x too pessimistic

The six context options carried hardcoded VRAM estimates ("32K ~4 GB", "128K
~16 GB — high-end GPU"). Those assume roughly 128 KB/token, a large-model
figure. Measured against the models actually in the library:

| context | label claimed | qwen2.5-omni-3b actual (36 KB/token) |
| --- | --- | --- |
| 8K | ~1 GB | **288 MB** |
| 16K | ~2 GB | **576 MB** |
| 32K | ~4 GB | **1.1 GB** |

The labels were talking the operator out of contexts the GPU holds comfortably —
the opposite of helpful, and directly relevant given the CONTEXT_TOO_SMALL
failure was about not having enough context for the RAG packets.

New `cmd_ctx_vram_profile(model_dirs)` returns KV bytes/token and native context
for the **most expensive** of the selected lane models (the setting applies to
all lanes at once, so worst case is the honest number). The frontend recomputes
every label from it on lane change and on restore, and disables options above
the model's trained context with "exceeds model max (NNK)" rather than silently
offering a setting that buys nothing. If metadata cannot be read the static text
is left alone rather than replaced with something invented.

### Verified

* `cargo check --workspace` clean (2 pre-existing unrelated warnings).
* **168 tests passed, 0 failed.**
* `ctx_probe` against the real model library returns live metadata:
  qwen2.5-omni-3b → 36 layers, 2 KV heads, head_dim 128, 36/36 layers on GPU at
  ctx 32768 with KV 1152 MB.

### Still open

* Per the operator, the **debug terminal exists but is only visible during a
  run** — not a missing feature, a visibility one.
* `16384` is still hardcoded in six places; should be one named constant.
* Window minimize: operator reports it now works. There is no custom minimize in
  the app (`decorations: true`), so it was environmental — most likely the
  splash window, which is `alwaysOnTop` + `skipTaskbar`.
* Not committed.

## [2026-08-03] — Linux migration: builds, tests, and runs on Ubuntu 26.04

Copied from the transfer drive to `~/workspace/AiSmartGuy` (ext4 — the USB is FAT32 and cannot
carry Unix permissions), `target/` excluded. 21 MB of source. Tauri v2, 12 crates + `src-tauri`,
the same architecture as GGUF-Chatbox, so the WebKit dependencies installed for that app covered
this one with no further packages.

### Done

**One hard build blocker: the icons were not RGBA.**

```
error: proc macro panicked
    --> src-tauri/src/main.rs:1335:14
     = help: message: icon .../icons/32x32.png is not RGBA
```

All four PNGs (`32x32`, `128x128`, `128x128@2x`, `icon.png`) were plain RGB, and Tauri v2's
`generate_context!` requires an alpha channel. Converted with `Image.convert("RGBA")`, which adds a
fully-opaque alpha and leaves every pixel otherwise untouched. Originals preserved in
`src-tauri/icons/.rgb_originals/` so the change is reversible (Article VI).

Worth noting this was never Linux-specific — the app could not have built against this Tauri
version on Windows either.

**`explorer.exe` hardcoded in two commands** (`cmd_open_model_library`, `cmd_open_output_folder`).
That binary exists only on Windows, so both buttons would fail with "No such file or directory" and
simply look broken. Replaced with a `FILE_MANAGER` const — `explorer.exe` / `open` / `xdg-open` —
and a shared `open_in_file_manager()` helper carrying the path into the error message.

**Everything else was already portable.** A grep for `USERPROFILE`, `.exe` and `C:\` produced a
long list, but reading each in context showed `llama_detect.rs`, `orchestrator.rs`, `rag_bridge.rs`
and `seed_rag_defaults` are all correctly `#[cfg]`-gated with HOME branches — the grep was showing
the Windows arm of each pair without its guard. `model_fetcher::cache::dirs_next_home` consults
USERPROFILE then HOME, which is fine.

No `rfd` dialogs anywhere, so the blocking-file-dialog defect found in GGUF-Chatbox does not exist
here. No `.venv\Scripts` or bare `python` invocations either.

### Remaining

* **Nothing beyond startup has been exercised.** The window renders "AiSmartGuy is Ready — Drag &
  drop a PDF onto this window to begin"; no PDF has been processed, no model fetched, no RAG query
  run. Ingestion, `model_fetcher` downloads and `model_loader` inference are all unverified on Linux.
* `model_loader/src/llama_detect.rs` is a near-copy of GGUF-Chatbox's. **It therefore carries the
  same KV-cache bug that was fixed there today**: `kv_reserve = (ctx_tokens / 16).max(500)` assumes
  64 KB/token where a GQA 7B needs 128 KB and a non-GQA 7B needs 512 KB. Port
  `kv_bytes_per_token()` across before anyone loads a large model on a small card.
* Two dead-code warnings (`pdf` at main.rs:170, `bg_run_id` at :993) — pre-existing, untouched.
* Not copied to the file cabinet: that is for proven apps, and only startup is proven.

### Decisions

* **Converted the icons rather than regenerating them** with `tauri icon`. Conversion preserves the
  existing artwork exactly; regeneration would have resampled it from one source and quietly
  changed the other sizes.
* **`xdg-open` rather than hardcoding Nautilus.** It dispatches to whatever file manager the desktop
  registered, so the app keeps working on non-GNOME desktops.
* Left the `#[cfg]`-gated Windows paths alone. They are correct, and rewriting working
  platform-conditional code is churn that risks a regression on the Windows box.

### Open Stubs

None introduced.

### Verification

* `cargo build -p aismartguy-app` → **exit 0** after the icon fix (2 pre-existing dead-code
  warnings).
* `cargo test --workspace` → **168 passed, 0 failed**.
* Launched under `systemd-run --user`; the window rendered its ready state, screen-captured.
* NOT VERIFIED: PDF ingestion, model fetch, model load, RAG query, the two folder buttons
  (`xdg-open` path is compiled but has not been clicked).
