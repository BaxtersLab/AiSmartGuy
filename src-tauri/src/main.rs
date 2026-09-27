// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::io::Read;

use tauri::{AppHandle, Emitter, Manager, State};
use serde::Serialize;

use manifest::{
    Manifest, ModelConfig, ModelSet, OptimizationState,
    RagPacketMap, ResourceThrottle, RunMode, SourcePdf,
};
use model_loader::gpu_mapper;
use rag_engine::hitlist;
use ui::state::{new_shared_state, SharedUiState};
use ui::types::UiConflict;

// ── Cancel flag managed state ────────────────────────────────────────────────
struct CancelFlag(Arc<AtomicBool>);

// ── Startup progress event payload ──────────────────────────────────────────
#[derive(Clone, Serialize)]
struct StartupProgress {
    percent: f32,
    message: String,
}

// ── Load-PDF result returned to frontend ────────────────────────────────────
#[derive(Clone, Serialize)]
struct LoadPdfResult {
    config_found: bool,
    estimated_tokens: u64,
    page_count: u32,
}

// ── startup_scan ─────────────────────────────────────────────────────────────
/// Called by the splash screen JS. Scans internal folders, emits progress
/// events to the splash window, then shows main and closes splash.
#[tauri::command]
async fn startup_scan(app: AppHandle) -> Result<(), String> {
    let steps: &[(f32, &str)] = &[
        (10.0, "Scanning internal folders…"),
        (30.0, "Checking configuration files…"),
        (55.0, "Loading manifest…"),
        (75.0, "Initialising engine…"),
        (90.0, "Seeding RAG defaults…"),
        (100.0, "Ready."),
    ];

    for (percent, message) in steps {
        app.emit("startup-progress", StartupProgress {
            percent: *percent,
            message: message.to_string(),
        }).map_err(|e: tauri::Error| e.to_string())?;

        // At the "Seeding RAG defaults" step, copy bundled packets to disk
        if *percent as u32 == 90 {
            seed_rag_defaults();
        }

        tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    }

    // Show main loader window.
    //
    // The size nudge that used to live here has been REMOVED. It was added on
    // 2026-08-04 on the theory that showing a `"visible": false` window leaves
    // the compositor holding a stale input region, and that setting the size
    // one pixel different and straight back would force a recompute.
    //
    // Both halves of that were measured on 2026-08-05 and both are false:
    //
    //  * A GTK 3.24.52 window created hidden and shown later reports EXACTLY
    //    the same geometry as one created visible — 1012x889, zero inset. No
    //    staleness to correct.
    //  * On Wayland GTK clamps to the size the compositor configured, so
    //    `set_size(h+1); set_size(h)` never changed the allocation at all. The
    //    nudge was inert; it cannot have fixed anything.
    //
    // Leaving dead code in place that claims to fix a live bug is worse than
    // having no fix, so it is gone until the real cause is known.
    if let Some(w) = app.get_webview_window("main") {
        w.show().map_err(|e| e.to_string())?;
        // Centre AFTER showing: a hidden window has no reliable geometry to
        // centre against, which is how it ended up hanging off the bottom edge.
        let _ = w.center();
        let _ = w.set_focus();
    }

    // Tell the main window it can transition to Ready.
    app.emit("pipeline-progress", PipelineProgress {
        percent: 100.0,
        message: "Ready.".into(),
    }).map_err(|e: tauri::Error| e.to_string())?;

    // Close splash.
    if let Some(w) = app.get_webview_window("splash") {
        w.close().map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ── Bundled RAG default packets ─────────────────────────────────────────────
/// Embedded at compile time from assets/rag_defaults/.
const RAG_DEFAULTS: &[(&str, &str)] = &[
    ("001_fallacies.json",           include_str!("../../assets/rag_defaults/001_fallacies.json")),
    ("002_weaponized_language.json", include_str!("../../assets/rag_defaults/002_weaponized_language.json")),
    ("006_ambiguous_framing.json",   include_str!("../../assets/rag_defaults/006_ambiguous_framing.json")),
    ("007_racism_intolerance.json",  include_str!("../../assets/rag_defaults/007_racism_intolerance.json")),
    ("009_nlp_techniques.json",      include_str!("../../assets/rag_defaults/009_nlp_techniques.json")),
];

/// Seed `~/.aismartguy/rag_defaults/` with the bundled RAG packets.
/// Existing files are overwritten so updates ship with new builds.
fn seed_rag_defaults() {
    let home = {
        #[cfg(target_os = "windows")]
        { std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\default".to_string()) }
        #[cfg(not(target_os = "windows"))]
        { std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()) }
    };
    let dir = PathBuf::from(home).join(".aismartguy").join("rag_defaults");
    if std::fs::create_dir_all(&dir).is_err() { return; }
    for (name, content) in RAG_DEFAULTS {
        let _ = std::fs::write(dir.join(name), content);
    }
}

// ── Pipeline progress event payload ─────────────────────────────────────────
#[derive(Clone, Serialize)]
struct PipelineProgress {
    percent: f32,
    message: String,
}

/// Separate event for run-time progress so the processing screen can listen
/// independently of the startup pipeline-progress listener.
#[derive(Clone, Serialize)]
struct RunProgress {
    percent: f32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
}

// ── Command wrappers ─────────────────────────────────────────────────────────

#[tauri::command]
fn cmd_load_pdf(
    app: AppHandle,
    state: State<SharedUiState>,
    pdf_path: String,
) -> Result<LoadPdfResult, String> {
    app.emit("pipeline-progress", PipelineProgress { percent: 5.0,  message: "Loading PDF…".into() }).ok();
    ui::commands::load_pdf(state.inner().clone(), &pdf_path)
        .map_err(|e| format!("{:?}", e))?;

    let config_found = state.lock().unwrap().config_detected;

    // Extract text to estimate token count and page count
    let (estimated_tokens, page_count) = match pdf_io::extract_text(std::path::Path::new(&pdf_path)) {
        Ok(extracted) => {
            let total_chars: usize = extracted.pages.iter().map(|p| p.len()).sum();
            // Conservative estimate: ~2 chars per token (matches chapter_detect::CHARS_PER_TOKEN)
            let tokens = (total_chars as u64) / 2;
            (tokens, extracted.page_count as u32)
        }
        Err(_) => (0, 0),
    };

    app.emit("pipeline-progress", PipelineProgress { percent: 25.0, message: "PDF loaded." .into() }).ok();
    Ok(LoadPdfResult { config_found, estimated_tokens, page_count })
}

/// After a PDF is loaded, auto-chain into the full pipeline if the PDF
/// contained an embedded manifest.  Serialises the in-memory manifest to a
/// temp file and creates a run directory next to the PDF.
#[tauri::command]
fn cmd_auto_run(
    app: AppHandle,
    state: State<SharedUiState>,
) -> Result<(), String> {
    let (has_manifest, pdf_path) = {
        let s = state.lock().unwrap();
        (s.config_detected, s.pdf_path.clone())
    };

    let pdf = PathBuf::from(
        pdf_path.ok_or_else(|| "no PDF loaded".to_string())?,
    );

    if !has_manifest {
        // No embedded manifest — stay on the Ready screen for now.
        app.emit("pipeline-progress", PipelineProgress {
            percent: 100.0,
            message: "PDF loaded (no embedded manifest). Awaiting configuration.".into(),
        }).ok();
        return Ok(());
    }

    // Serialize the in-memory manifest to a temp file next to the PDF.
    let manifest_json = {
        let s = state.lock().unwrap();
        let m = s.manifest.as_ref().ok_or("manifest disappeared")?;
        serde_json::to_string_pretty(m).map_err(|e| e.to_string())?
    };

    let run_dir = output_dir(&app)?.join("config_run");
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;

    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(&manifest_path, &manifest_json).map_err(|e| e.to_string())?;

    app.emit("pipeline-progress", PipelineProgress {
        percent: 30.0,
        message: "Starting analysis…".into(),
    }).ok();

    // A cancelled run leaves the flag set; without clearing it here the
    // next run dies instantly with no explanation.
    model_loader::cancel::clear();
    ui::commands::start_run(
        state.inner().clone(),
        manifest_path,
        run_dir,
    ).map_err(|e| format!("{:?}", e))?;

    app.emit("pipeline-progress", PipelineProgress {
        percent: 100.0,
        message: "Run complete.".into(),
    }).ok();

    Ok(())
}

/// Apply a previously-found configuration to a brand-new PDF.
/// The manifest is already in shared state from an earlier `cmd_load_pdf`
/// on a previous AiSmartGuy report.  This command:
///   1. Updates state.pdf_path to the new PDF.
///   2. Serialises the stored manifest to a run dir next to the new PDF.
///   3. Kicks off start_run.
#[tauri::command]
fn cmd_run_with_stored_config(
    app: AppHandle,
    state: State<SharedUiState>,
    new_pdf_path: String,
) -> Result<(), String> {
    // Grab the stored manifest.
    let manifest_json = {
        let s = state.lock().unwrap();
        let m = s.manifest.as_ref()
            .ok_or_else(|| "no stored configuration — load a report PDF first".to_string())?;
        serde_json::to_string_pretty(m).map_err(|e| e.to_string())?
    };

    // Point state at the new PDF.
    let _new_pdf = PathBuf::from(&new_pdf_path);
    {
        let mut s = state.lock().unwrap();
        s.pdf_path = Some(new_pdf_path);
        s.pdf_loaded = true;
    }

    app.emit("pipeline-progress", PipelineProgress {
        percent: 10.0,
        message: "Applying stored configuration to new PDF…".into(),
    }).ok();

    let run_dir = output_dir(&app)?.join("stored_config_run");
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;

    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(&manifest_path, &manifest_json).map_err(|e| e.to_string())?;

    app.emit("pipeline-progress", PipelineProgress {
        percent: 30.0,
        message: "Starting analysis…".into(),
    }).ok();

    // A cancelled run leaves the flag set; without clearing it here the
    // next run dies instantly with no explanation.
    model_loader::cancel::clear();
    ui::commands::start_run(
        state.inner().clone(),
        manifest_path,
        run_dir,
    ).map_err(|e| format!("{:?}", e))?;

    app.emit("pipeline-progress", PipelineProgress {
        percent: 100.0,
        message: "Run complete.".into(),
    }).ok();

    Ok(())
}

#[tauri::command]
fn cmd_apply_configuration(
    app: AppHandle,
    state: State<SharedUiState>,
    manifest_path: String,
) -> Result<(), String> {
    app.emit("pipeline-progress", PipelineProgress { percent: 30.0, message: "Applying configuration…".into() }).ok();
    ui::commands::apply_configuration(state.inner().clone(), manifest_path)
        .map_err(|e| format!("{:?}", e))?;
    app.emit("pipeline-progress", PipelineProgress { percent: 50.0, message: "Configuration applied.".into() }).ok();
    Ok(())
}

#[tauri::command]
fn cmd_resolve_conflict(
    state: State<SharedUiState>,
    conflict: UiConflict,
) -> Result<(), String> {
    ui::commands::resolve_conflict(state.inner().clone(), conflict)
        .map_err(|e| format!("{:?}", e))
}

#[tauri::command]
fn cmd_start_run(
    app: AppHandle,
    state: State<SharedUiState>,
    manifest_path: String,
    run_dir: String,
) -> Result<(), String> {
    app.emit("pipeline-progress", PipelineProgress { percent: 55.0, message: "Running pipeline…".into() }).ok();
    // A cancelled run leaves the flag set; without clearing it here the
    // next run dies instantly with no explanation.
    model_loader::cancel::clear();
    ui::commands::start_run(
        state.inner().clone(),
        PathBuf::from(manifest_path),
        PathBuf::from(run_dir),
    ).map_err(|e| format!("{:?}", e))?;
    app.emit("pipeline-progress", PipelineProgress { percent: 100.0, message: "Run complete.".into() }).ok();
    Ok(())
}

/// Stop the current run.
///
/// Sets BOTH flags. The `CancelFlag` managed state is the original one and is
/// kept so nothing that already reads it changes behaviour; the model_loader
/// flag is the one the inference loop actually polls. Before this, only the
/// former was set — and nothing read it — so cancelling did nothing at all and
/// the only way to stop a run was to close the application.
#[tauri::command]
fn cmd_cancel_run(cancel: State<CancelFlag>) {
    cancel.0.store(true, Ordering::SeqCst);
    model_loader::cancel::request();
}

#[tauri::command]
fn cmd_download_model(
    app: AppHandle,
    state: State<SharedUiState>,
    model_name: String,
    repo_id: String,
    cache_dir: String,
) -> Result<(), String> {
    app.emit("pipeline-progress", PipelineProgress {
        percent: 10.0,
        message: format!("Fetching {}…", model_name),
    }).ok();
    ui::commands::download_model(state.inner().clone(), model_name, repo_id, PathBuf::from(cache_dir))
        .map_err(|e| format!("{:?}", e))
}

#[tauri::command]
fn cmd_retry_model_download(
    app: AppHandle,
    state: State<SharedUiState>,
    model_name: String,
    repo_id: String,
    cache_dir: String,
) -> Result<(), String> {
    app.emit("pipeline-progress", PipelineProgress {
        percent: 5.0,
        message: format!("Retrying {}…", model_name),
    }).ok();
    ui::commands::retry_model_download(state.inner().clone(), model_name, repo_id, PathBuf::from(cache_dir))
        .map_err(|e| format!("{:?}", e))
}

#[tauri::command]
fn cmd_cancel_model_download(
    state: State<SharedUiState>,
    model_name: String,
) {
    ui::commands::cancel_model_download(state.inner().clone(), model_name);
}

// ── Model library helpers ────────────────────────────────────────────────────

/// Where the operator's chosen model library is remembered.
fn library_pref_path(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_local_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {}", e))?;
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    Ok(base.join("model_library.txt"))
}

/// On-disk folder where GGUF models live.
///
/// Defaults to `<app data>/models`, but the operator can point it at their real
/// library. Models are multi-gigabyte and normally live on a separate disk;
/// requiring them inside the app's private data directory means copying 5 GB
/// per model just to make the app notice it.
fn model_library_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(p) = library_pref_path(app) {
        if let Ok(saved) = std::fs::read_to_string(&p) {
            let saved = saved.trim();
            // Fall through to the default if the saved path has gone away —
            // an unplugged drive should not make the app unusable.
            if !saved.is_empty() && PathBuf::from(saved).is_dir() {
                return Ok(PathBuf::from(saved));
            }
        }
    }
    let base = app.path().app_local_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {}", e))?;
    let dir = base.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Open a folder picker and remember the chosen model library.
///
/// `Err("cancelled")` means the operator dismissed the dialog; the caller
/// treats that as a no-op rather than something to report.
#[tauri::command]
async fn cmd_browse_model_library(app: AppHandle) -> Result<String, String> {
    let start = model_library_dir(&app).ok();
    let mut dlg = rfd::AsyncFileDialog::new().set_title("Select model library folder");
    if let Some(d) = start.as_ref() {
        dlg = dlg.set_directory(d);
    }
    let picked = dlg.pick_folder().await.ok_or_else(|| "cancelled".to_string())?;
    let path = picked.path().to_string_lossy().into_owned();
    std::fs::write(library_pref_path(&app)?, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Forget the chosen library and fall back to the app's own models folder.
#[tauri::command]
fn cmd_reset_model_library(app: AppHandle) -> Result<String, String> {
    if let Ok(p) = library_pref_path(&app) {
        let _ = std::fs::remove_file(p);
    }
    Ok(model_library_dir(&app)?.to_string_lossy().into_owned())
}

/// Set the library from a typed or pasted path.
///
/// Rejects a path that is not a directory instead of storing it: a saved bad
/// path would silently fall back to the default on next read, and the operator
/// would see the default without ever being told their entry was refused.
#[tauri::command]
fn cmd_set_model_library(app: AppHandle, path: String) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return cmd_reset_model_library(app);
    }
    let p = PathBuf::from(trimmed);
    if !p.is_dir() {
        return Err(format!("not a folder: {}", trimmed));
    }
    std::fs::write(library_pref_path(&app)?, trimmed).map_err(|e| e.to_string())?;
    Ok(trimmed.to_string())
}

/// Which library is in force, and whether it is the operator's choice or the
/// built-in default.
#[derive(Clone, Serialize)]
struct ModelLibraryInfo {
    path: String,
    default_path: String,
    is_override: bool,
}

/// Report the effective library AND the default, so the UI can say which one is
/// active rather than showing a bare path.
///
/// This matters because the fetcher downloads into the *default* folder while
/// the lanes list the *effective* one. Without saying which is which, a model
/// can be fetched successfully and then be invisible in the dropdowns, with
/// nothing on screen to explain it.
#[tauri::command]
fn cmd_model_library_info(app: AppHandle) -> Result<ModelLibraryInfo, String> {
    let base = app.path().app_local_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {}", e))?;
    let default_path = base.join("models");
    let effective = model_library_dir(&app)?;
    Ok(ModelLibraryInfo {
        is_override: effective != default_path,
        path: effective.to_string_lossy().into_owned(),
        default_path: default_path.to_string_lossy().into_owned(),
    })
}

/// On-disk folder where output report PDFs are saved.
fn output_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_local_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {}", e))?;
    let dir = base.join("output");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Derive a report filename from the source PDF filename.
/// "MyBook.pdf" → "MyBook_AiSmartGuy_Report.pdf"
#[allow(dead_code)]
fn report_filename(pdf_filename: &str) -> String {
    let stem = pdf_filename.strip_suffix(".pdf")
        .or_else(|| pdf_filename.strip_suffix(".PDF"))
        .unwrap_or(pdf_filename);
    // Sanitize: keep only alphanumeric, spaces, hyphens, underscores, dots.
    let safe: String = stem.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    format!("{}_AiSmartGuy_Report.pdf", safe)
}

#[derive(Clone, Serialize)]
struct ModelLibraryEntry {
    filename: String,
    path: String,
    size_mb: f64,
}

/// Return the absolute path to the model library so the UI can show it.
#[tauri::command]
fn cmd_get_model_library_path(app: AppHandle) -> Result<String, String> {
    let dir = model_library_dir(&app)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// The platform's "reveal this folder" command.
///
/// `explorer.exe` exists only on Windows. The Linux equivalent is `xdg-open`,
/// which hands off to whatever file manager the desktop registered (Nautilus
/// under GNOME). Spawning `explorer.exe` there fails with "No such file or
/// directory", so both buttons below simply appeared broken.
#[cfg(target_os = "windows")]
const FILE_MANAGER: &str = "explorer.exe";
#[cfg(target_os = "macos")]
const FILE_MANAGER: &str = "open";
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
const FILE_MANAGER: &str = "xdg-open";

/// Reveal `dir` in the desktop's file manager.
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new(FILE_MANAGER)
        .arg(dir.as_os_str())
        .spawn()
        .map_err(|e| format!("failed to open {} with {}: {}",
                             dir.display(), FILE_MANAGER, e))?;
    Ok(())
}

/// Open the model library folder in the desktop's file manager.
#[tauri::command]
fn cmd_open_model_library(app: AppHandle) -> Result<(), String> {
    let dir = model_library_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_in_file_manager(&dir)
}

/// Open the output folder in the desktop's file manager.
#[tauri::command]
fn cmd_open_output_folder(app: AppHandle, path: String) -> Result<(), String> {
    let dir = if path.is_empty() {
        output_dir(&app)?
    } else {
        PathBuf::from(&path)
    };
    if dir.is_dir() {
        open_in_file_manager(&dir)?;
    }
    Ok(())
}

/// List every .gguf file in the model library.
#[tauri::command]
fn cmd_list_model_library(app: AppHandle) -> Result<Vec<ModelLibraryEntry>, String> {
    let dir = model_library_dir(&app)?;
    let mut entries = Vec::new();
    let rd = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if name.to_lowercase().ends_with(".gguf") {
            let meta = item.metadata().map_err(|e| e.to_string())?;
            entries.push(ModelLibraryEntry {
                filename: name,
                path: item.path().to_string_lossy().into_owned(),
                size_mb: meta.len() as f64 / (1024.0 * 1024.0),
            });
        }
    }
    entries.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(entries)
}

/// A subfolder in the model library that contains at least one .gguf file.
#[derive(Clone, Serialize)]
struct LibrarySubfolder {
    name: String,
    path: String,
    gguf_files: Vec<ModelLibraryEntry>,
}

/// List subfolders of a given directory that contain .gguf files.
/// If `base_dir` is empty, defaults to the model library.
#[tauri::command]
fn cmd_list_library_subfolders(app: AppHandle, base_dir: String) -> Result<Vec<LibrarySubfolder>, String> {
    let dir = if base_dir.is_empty() {
        model_library_dir(&app)?
    } else {
        PathBuf::from(&base_dir)
    };

    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }

    let mut folders = Vec::new();
    let rd = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for item in rd.flatten() {
        // `DirEntry::file_type` does NOT follow symlinks, so a symlinked model
        // folder reports as a symlink and would be skipped. `path().is_dir()`
        // follows — which matters because linking a big library into place is
        // the obvious way to avoid copying gigabytes.
        if !item.path().is_dir() { continue; }
        let sub_name = item.file_name().to_string_lossy().into_owned();
        let sub_path = item.path();

        // Scan for .gguf files inside
        let mut gguf_files = Vec::new();
        if let Ok(sub_rd) = std::fs::read_dir(&sub_path) {
            for f in sub_rd.flatten() {
                let fname = f.file_name().to_string_lossy().into_owned();
                if fname.to_lowercase().ends_with(".gguf") {
                    let meta = f.metadata().map_err(|e| e.to_string())?;
                    gguf_files.push(ModelLibraryEntry {
                        filename: fname,
                        path: f.path().to_string_lossy().into_owned(),
                        size_mb: meta.len() as f64 / (1024.0 * 1024.0),
                    });
                }
            }
        }

        gguf_files.sort_by(|a, b| a.filename.cmp(&b.filename));
        folders.push(LibrarySubfolder {
            name: sub_name,
            path: sub_path.to_string_lossy().into_owned(),
            gguf_files,
        });
    }
    folders.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(folders)
}

/// Log-line event sent to the frontend during an HF download.
#[derive(Clone, Serialize)]
struct HfDownloadLog {
    line: String,
    done: bool,
    error: bool,
}

#[derive(Clone, Serialize)]
struct PartialDownloadEntry {
    filename: String,
    size_mb: f64,
}

/// Download a single GGUF file from a HuggingFace URL into the model library.
/// Streams the response in 1 MB chunks, emitting `hf-download-log` events that
/// the frontend renders in a terminal-style box.
#[tauri::command]
async fn cmd_download_hf_model(app: AppHandle, url: String) -> Result<(), String> {
    // Validate URL.
    if !url.starts_with("https://") {
        return Err("URL must start with https://".into());
    }

    // Reject repo browser pages — user needs a direct file link.
    if url.contains("/tree/") || url.contains("/blob/") {
        return Err("That is a repository page, not a direct file link. Use a /resolve/ URL or a direct .gguf download link.".into());
    }

    let lib_dir = model_library_dir(&app)?;

    // Derive filename from the URL's last path segment.
    let filename = url.rsplit('/')
        .next()
        .unwrap_or("model.gguf")
        .split('?')          // strip query string if any
        .next()
        .unwrap_or("model.gguf")
        .to_string();

    // Warn if the filename doesn't look like a GGUF file.
    if !filename.to_lowercase().ends_with(".gguf") {
        return Err(format!(
            "No .gguf file recognized in URL. Got filename '{}'. Paste a direct link to a .gguf file.",
            filename
        ));
    }

    // Build a subfolder from the filename stem so lane dropdowns can find it.
    let stem = filename.trim_end_matches(".gguf")
        .trim_end_matches(".GGUF")
        .to_string();
    let sub_dir = lib_dir.join(&stem);
    std::fs::create_dir_all(&sub_dir).map_err(|e| e.to_string())?;

    let dest = sub_dir.join(&filename);
    let part = sub_dir.join(format!("{}.part", filename));

    let app2 = app.clone();
    let log = move |msg: String, done: bool, error: bool| {
        app2.emit("hf-download-log", HfDownloadLog { line: msg, done, error }).ok();
    };

    // Run the blocking download on a background thread.
    let handle = tokio::task::spawn_blocking(move || -> Result<(), String> {
        // Check for existing .part file to resume from.
        let existing_bytes: u64 = if part.exists() {
            std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };

        if existing_bytes > 0 {
            log(format!("↻ Resuming from {:.1} MB", existing_bytes as f64 / 1_048_576.0), false, false);
        }
        log(format!("→ GET {}", url), false, false);
        log(format!("  target: {}", dest.display()), false, false);

        // Build agent with per-phase timeouts: 30s to connect, 5 min per read chunk.
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(30))
            .timeout_read(std::time::Duration::from_secs(300))
            .build();

        let mut req = agent.get(&url);
        if existing_bytes > 0 {
            req = req.set("Range", &format!("bytes={}-", existing_bytes));
        }

        let resp = req.call().map_err(|e| format!("HTTP error: {}", e))?;

        // Reject HTML responses — means we hit a web page, not a binary file.
        let content_type = resp.content_type().to_lowercase();
        if content_type.contains("text/html") || content_type.contains("application/json") {
            return Err("Server returned a web page, not a model file. Check your URL — you need a direct .gguf download link.".into());
        }

        let status = resp.status();
        // 206 = partial content (resume accepted), 200 = full file (server ignored range)
        let resumed = status == 206 && existing_bytes > 0;

        let content_len: Option<u64> = resp.header("content-length")
            .and_then(|v| v.parse().ok());

        // Total file size: if resumed, remaining + already downloaded; otherwise content-length.
        let total: Option<u64> = if resumed {
            content_len.map(|cl| cl + existing_bytes)
        } else {
            content_len
        };

        let start_offset: u64 = if resumed { existing_bytes } else { 0 };

        if let Some(t) = total {
            log(format!("  size: {:.1} MB", t as f64 / 1_048_576.0), false, false);
        }
        if resumed {
            log(format!("  server accepted resume at byte {}", existing_bytes), false, false);
        } else if existing_bytes > 0 {
            log("  server did not accept resume — restarting from scratch".into(), false, false);
        }

        let mut reader = resp.into_reader();

        // Open file: append if resumed, create/truncate otherwise.
        let mut file = if resumed {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(|e| e.to_string())?
        } else {
            std::fs::File::create(&part).map_err(|e| e.to_string())?
        };

        let mut downloaded: u64 = start_offset;
        let mut buf = vec![0u8; 1_048_576]; // 1 MB chunks
        let mut last_pct: u64 = if let Some(t) = total {
            if t > 0 { (start_offset * 100) / t } else { 0 }
        } else {
            0
        };

        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("read error: {}", e))?;
            if n == 0 { break; }
            std::io::Write::write_all(&mut file, &buf[..n])
                .map_err(|e| format!("write error: {}", e))?;
            downloaded += n as u64;

            if let Some(t) = total {
                let pct = if t > 0 { (downloaded * 100) / t } else { 0 };
                if pct != last_pct {
                    last_pct = pct;
                    log(format!("  {}% — {:.1} / {:.1} MB",
                        pct,
                        downloaded as f64 / 1_048_576.0,
                        t as f64 / 1_048_576.0,
                    ), false, false);
                }
            } else if downloaded % (10 * 1_048_576) == 0 {
                log(format!("  {:.1} MB downloaded", downloaded as f64 / 1_048_576.0), false, false);
            }
        }

        drop(file);
        // Atomic rename .part → final
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        log(format!("✓ Download complete: {}", filename), true, false);
        Ok(())
    });

    match handle.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            app.emit("hf-download-log", HfDownloadLog {
                line: format!("✗ {} — .part file kept for resume", e), done: true, error: true,
            }).ok();
            // Do NOT delete .part — keep it so the user can resume later.
            Err(e)
        }
        Err(e) => {
            let msg = format!("download task panicked: {}", e);
            app.emit("hf-download-log", HfDownloadLog {
                line: msg.clone(), done: true, error: true,
            }).ok();
            Err(msg)
        }
    }
}

// ── Context-size VRAM profile ────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
struct CtxVramProfile {
    /// Bytes of KV cache per token, read from the model's own GGUF metadata.
    kv_bytes_per_token: u64,
    /// The model's native training context — options above this are pointless.
    native_ctx: u32,
    /// Which lane model these numbers came from (the most expensive one).
    model_name: String,
}

/// KV-cache cost per token for the most expensive of the given model folders.
///
/// The context-size UI previously carried hardcoded VRAM labels ("32K ~4 GB")
/// that assumed roughly 128 KB/token. That is a large-model figure: a 1B with
/// grouped-query attention uses 24 KB/token, making those labels about 5x too
/// pessimistic and steering the operator to a smaller context than the hardware
/// can hold. The setting applies to every lane at once, so the worst case
/// across the selected models is the honest number to show.
///
/// Returns None when no folder yields readable metadata, so the UI can keep its
/// static text rather than display something invented.
#[tauri::command]
fn cmd_ctx_vram_profile(model_dirs: Vec<String>) -> Option<CtxVramProfile> {
    let mut worst: Option<CtxVramProfile> = None;
    for dir in model_dirs.iter().filter(|d| !d.trim().is_empty()) {
        let Ok(path) = first_gguf_in(dir) else { continue };
        let Some(kv) = model_loader::kv_bytes_per_token(&path) else { continue };
        let native = model_loader::gguf_context_length(&path).unwrap_or(0);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.clone());
        let better = match &worst {
            None => true,
            Some(w) => kv > w.kv_bytes_per_token,
        };
        if better {
            worst = Some(CtxVramProfile {
                kv_bytes_per_token: kv,
                native_ctx: native,
                model_name: name,
            });
        }
    }
    worst
}

// ── Begin Run — build manifest from lane selections and run pipeline ─────────

/// Resolve the first `.gguf` file inside `folder`. Returns the full path.
fn first_gguf_in(folder: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(folder);
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", folder));
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x.eq_ignore_ascii_case("gguf"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());
    entries
        .first()
        .map(|e| e.path())
        .ok_or_else(|| format!("no .gguf file found in {}", folder))
}

fn model_config_from_lane(folder: &str, ctx: u32) -> Result<ModelConfig, String> {
    let gguf_path = first_gguf_in(folder)?;
    let name = gguf_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok(ModelConfig {
        name,
        path: gguf_path.to_string_lossy().into_owned(),
        quantization: String::new(),
        context_length: Some(ctx),
        gpu_usage: Some("CPU".to_string()),
        n_gpu_layers: None,
        active: true,
        revision: None,
        sha256: None,
    })
}

/// List .part files in the models directory (incomplete downloads available for resume).
#[tauri::command]
fn cmd_list_partial_downloads(app: AppHandle) -> Result<Vec<PartialDownloadEntry>, String> {
    let dir = model_library_dir(&app)?;
    let mut entries = Vec::new();
    // Scan root level
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for item in rd.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name.to_lowercase().ends_with(".gguf.part") {
                let meta = item.metadata().map_err(|e| e.to_string())?;
                entries.push(PartialDownloadEntry {
                    filename: name.trim_end_matches(".part").to_string(),
                    size_mb: meta.len() as f64 / (1024.0 * 1024.0),
                });
            }
        }
    }
    // Scan one level of subfolders
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for item in rd.flatten() {
            if item.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                if let Ok(sub_rd) = std::fs::read_dir(item.path()) {
                    for sub in sub_rd.flatten() {
                        let name = sub.file_name().to_string_lossy().into_owned();
                        if name.to_lowercase().ends_with(".gguf.part") {
                            let meta = sub.metadata().map_err(|e| e.to_string())?;
                            entries.push(PartialDownloadEntry {
                                filename: name.trim_end_matches(".part").to_string(),
                                size_mb: meta.len() as f64 / (1024.0 * 1024.0),
                            });
                        }
                    }
                }
            }
        }
    }
    entries.sort_by(|a, b| a.filename.cmp(&b.filename));
    entries.dedup_by(|a, b| a.filename == b.filename);
    Ok(entries)
}

/// Delete a .part file from the models directory.
#[tauri::command]
fn cmd_delete_partial_download(app: AppHandle, filename: String) -> Result<(), String> {
    // Sanitize: only allow files ending in .gguf, and we append .part ourselves.
    if !filename.to_lowercase().ends_with(".gguf") {
        return Err("invalid filename".into());
    }
    let dir = model_library_dir(&app)?;
    // Check root level
    let part_root = dir.join(format!("{}.part", filename));
    if part_root.exists() {
        std::fs::remove_file(&part_root).map_err(|e| e.to_string())?;
        return Ok(());
    }
    // Check inside subfolder named after the stem
    let stem = filename.trim_end_matches(".gguf").trim_end_matches(".GGUF");
    let part_sub = dir.join(stem).join(format!("{}.part", filename));
    if part_sub.exists() {
        std::fs::remove_file(&part_sub).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Query VRAM via nvidia-smi and return the amount in MB (0 = no GPU).
#[tauri::command]
fn cmd_detect_vram() -> Result<u32, String> {
    Ok(gpu_mapper::query_vram_mb())
}

/// Link to Hot Rod Tuner running on localhost.
/// Sends our exe path + PID so HRT can e-stop this process.
#[tauri::command]
fn cmd_link_hrt(port: u16) -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot resolve exe path: {}", e))?
        .to_string_lossy()
        .into_owned();
    let pid = std::process::id();

    let url = format!("http://127.0.0.1:{}/link", port);
    let body = serde_json::json!({
        "app_name": "AiSmartGuy",
        "exe_path": exe,
        "pid": pid
    });

    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("HRT not reachable: {}", e))?;

    if resp.status() == 200 {
        Ok("HRT Link Successful".into())
    } else {
        Err(format!("HRT responded with status {}", resp.status()))
    }
}

#[tauri::command]
fn cmd_begin_run(
    app: AppHandle,
    state: State<SharedUiState>,
    mode: String,
    lane2: String,
    lane3: String,
    lane4: String,
    lane5: String,
    throttle_pct: Option<u32>,
    ctx_size: Option<u32>,
) -> Result<String, String> {
    // ── 1. Resolve PDF path from state ───────────────────────────
    let pdf_path = {
        let s = state.lock().unwrap();
        PathBuf::from(
            s.pdf_path
                .as_ref()
                .ok_or_else(|| "no PDF loaded".to_string())?
                .clone(),
        )
    };

    app.emit("pipeline-progress", PipelineProgress {
        percent: 5.0,
        message: "Building manifest from lane selections…".into(),
    }).ok();

    app.emit("run-progress", RunProgress {
        percent: 5.0,
        message: "Building manifest…".into(),
        output_path: None,
    }).ok();

    // ── 2. Map gating mode → RunMode + active model configs ──────
    let run_mode = match mode.as_str() {
        "1" => RunMode::Single,
        "2" => RunMode::Dual,
        _   => RunMode::Full,
    };

    let ctx = ctx_size.unwrap_or(16384);
    let throttle = throttle_pct.unwrap_or(75).clamp(25, 100);

    let mut model1 = Some(model_config_from_lane(&lane2, ctx)?);
    if let Some(ref mut m) = model1 { m.gpu_usage = Some("GPU".to_string()); }

    let mut model2 = if mode == "3" || mode == "full" {
        Some(model_config_from_lane(&lane3, ctx)?)
    } else {
        None
    };
    if let Some(ref mut m) = model2 { m.gpu_usage = Some("GPU".to_string()); }

    let mut model3 = if mode == "full" {
        Some(model_config_from_lane(&lane4, ctx)?)
    } else {
        None
    };
    if let Some(ref mut m) = model3 { m.gpu_usage = Some("GPU".to_string()); }

    let mut fusion = if mode != "1" {
        Some(model_config_from_lane(&lane5, ctx)?)
    } else {
        None
    };
    if let Some(ref mut m) = fusion { m.gpu_usage = Some("GPU".to_string()); }

    // ── 3. Assemble manifest ─────────────────────────────────────
    let run_id = {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = d.as_secs();
        let s = secs % 60;
        let m = (secs / 60) % 60;
        let h = (secs / 3600) % 24;
        let (y, mo, day) = epoch_days_to_ymd((secs / 86400) as i64);
        format!("Run_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", y, mo, day, h, m, s)
    };
    let timestamp = chrono_timestamp();
    let source_pdf = SourcePdf {
        filename: pdf_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        hash_sha256: None,
        page_count: None,
    };

    let manifest = Manifest {
        manifest_version: "1.0".into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
        run_id: run_id.clone(),
        timestamp,
        source_pdf,
        mode: run_mode,
        models: ModelSet { model1, model2, model3, fusion },
        rag_packets_used: RagPacketMap::new(),
        categories_active: hitlist::active_slugs(),
        optimization_state: OptimizationState::default(),
        resource_throttle: ResourceThrottle { throttle_pct: throttle },
        partial_run: None,
        notes: None,
    };

    // ── 4. Write manifest + create run dir ───────────────────────
    let run_dir = output_dir(&app)?
        .join(&run_id);
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;

    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(&manifest_path, &manifest_json).map_err(|e| e.to_string())?;

    // Store the manifest in shared state so config detection works on reload
    {
        let mut s = state.lock().unwrap();
        s.manifest = Some(manifest);
        s.config_detected = true;
    }

    app.emit("pipeline-progress", PipelineProgress {
        percent: 15.0,
        message: "Manifest written. Starting pipeline…".into(),
    }).ok();

    // Also emit run-progress so the processing screen picks it up
    app.emit("run-progress", RunProgress {
        percent: 15.0,
        message: "AiSmartGuy is reading the PDF…".into(),
        output_path: None,
    }).ok();

    // ── 5. Run the orchestrator pipeline ─────────────────────────
    // This calls: extract PDF → chunk → model inference (gated) → fusion → write final PDF
    // The final PDF has the manifest embedded as metadata.
    // Spawn on a background thread so the Tauri event loop stays alive.

    let bg_state = state.inner().clone();
    let bg_app = app.clone();
    let bg_run_id = run_id.clone();

    // Register a log callback so llama-cli stderr lines stream to the frontend.
    let log_app = app.clone();
    model_loader::set_log_callback(move |line| {
        log_app.emit("run-log", line).ok();
    });

    // Register a progress callback so orchestrator events reach the frontend.
    // Maps the orchestrator's 0.0–1.0 range into the 20–95% UI band
    // (0–15% was the pre-spawn manifest work; 100% is emitted on completion).
    // Ignore sub-step emissions (percent 0.0 or 1.0 from internal helpers like
    // ensure_model_ready) that would stomp the main pipeline progress.
    let cb_app = app.clone();
    let last_pct = std::sync::Arc::new(std::sync::Mutex::new(15.0_f32));
    let lp = last_pct.clone();
    orchestrator::set_progress_callback(move |ev| {
        // Sub-step helpers emit 0.0 and 1.0 as their own internal progress —
        // skip those so they don't reset or leap the main progress bar.
        if ev.percent <= 0.0 || ev.percent >= 1.0 {
            // Still forward the message so the user sees what's happening
            let current = *lp.lock().unwrap();
            cb_app.emit("run-progress", RunProgress {
                percent: current,
                message: ev.message.clone(),
                output_path: None,
            }).ok();
            return;
        }
        let ui_pct = 20.0 + (ev.percent * 75.0) as f32;
        *lp.lock().unwrap() = ui_pct;
        cb_app.emit("run-progress", RunProgress {
            percent: ui_pct,
            message: ev.message.clone(),
            output_path: None,
        }).ok();
    });

    std::thread::spawn(move || {
        // Enable backtraces so panics include location info.
        std::env::set_var("RUST_BACKTRACE", "1");

        // Catch panics so a crash in pdf-extract or the orchestrator doesn't
        // silently kill the thread, leaving the UI stuck forever.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // A cancelled run leaves the flag set; without clearing it here the
    // next run dies instantly with no explanation.
    model_loader::cancel::clear();
    ui::commands::start_run(bg_state, manifest_path, run_dir.clone())
        }));

        match result {
            Ok(Ok(())) => {
                orchestrator::clear_progress_callback();
                model_loader::clear_log_callback();
                let run_dir_str = run_dir.to_string_lossy().into_owned();

                bg_app.emit("pipeline-progress", PipelineProgress {
                    percent: 100.0,
                    message: "Run complete — output PDF generated with embedded configuration.".into(),
                }).ok();

                bg_app.emit("run-progress", RunProgress {
                    percent: 100.0,
                    message: "Report Complete".into(),
                    output_path: Some(run_dir_str),
                }).ok();
            }
            Ok(Err(e)) => {
                orchestrator::clear_progress_callback();
                model_loader::clear_log_callback();
                let msg = format!("Pipeline error: {:?}", e);
                eprintln!("[cmd_begin_run] {}", msg);

                bg_app.emit("run-progress", RunProgress {
                    percent: 0.0,
                    message: msg,
                    output_path: None,
                }).ok();
            }
            Err(panic_info) => {
                orchestrator::clear_progress_callback();
                model_loader::clear_log_callback();
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    format!("Pipeline crashed: {}", s)
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    format!("Pipeline crashed: {}", s)
                } else {
                    "Pipeline crashed (unknown panic)".to_string()
                };
                eprintln!("[cmd_begin_run] {}", msg);

                bg_app.emit("run-progress", RunProgress {
                    percent: 0.0,
                    message: msg,
                    output_path: None,
                }).ok();
            }
        }
    });

    Ok(run_id)
}

/// Simple timestamp without pulling in the chrono crate.
fn chrono_timestamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    // Approximate UTC: not locale-aware, but sufficient for run IDs.
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Good-enough year/month/day from epoch days.
    let (y, mo, day) = epoch_days_to_ymd(days as i64);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, day, h, m, s)
}

fn epoch_days_to_ymd(mut days: i64) -> (i64, i64, i64) {
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ── main ─────────────────────────────────────────────────────────────────────

// ── Llama.cpp bootstrap ──────────────────────────────────────────────────────

/// Check if llama-cli is installed. Returns the path if found, empty string if not.
#[tauri::command]
fn cmd_detect_llama() -> String {
    match model_loader::detect_llama() {
        Some(p) => p.to_string_lossy().into_owned(),
        None => String::new(),
    }
}

/// Download and install llama.cpp into ~/.aismartguy/llama-cpp/.
/// Picks CUDA 12.4 build if nvidia-smi works, otherwise CPU build.
/// Emits "llama-install-progress" events.
#[tauri::command]
fn cmd_install_llama(app: AppHandle) -> Result<String, String> {
    let install_dir = model_loader::llama_install_dir();
    std::fs::create_dir_all(&install_dir).map_err(|e| e.to_string())?;

    // Already installed?
    let local = model_loader::llama_local_path();
    if local.is_file() {
        return Ok(local.to_string_lossy().into_owned());
    }

    app.emit("llama-install-progress", serde_json::json!({
        "percent": 5, "message": "Detecting GPU…"
    })).ok();

    // Detect NVIDIA GPU
    let has_nvidia = std::process::Command::new("nvidia-smi")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    // Pick release asset name pattern.
    //
    // Upstream names assets per platform:
    //   Windows  llama-<build>-bin-win-<backend>-x64.zip
    //   Linux    llama-<build>-bin-ubuntu-<backend>-x64.tar.gz
    //
    // There is NO prebuilt CUDA build for Ubuntu — verified against release
    // b10237, whose ubuntu assets are plain / vulkan / rocm / sycl / openvino
    // only. So an NVIDIA GPU on Linux takes the VULKAN build, which is the
    // portable GPU backend upstream actually publishes there.
    #[cfg(target_os = "windows")]
    let (asset_keyword, fallback_keyword, archive_ext) = if has_nvidia {
        ("bin-win-cuda-12.4-x64", "bin-win-cpu-x64", ".zip")
    } else {
        ("bin-win-cpu-x64", "bin-win-cpu-x64", ".zip")
    };
    #[cfg(not(target_os = "windows"))]
    let (asset_keyword, fallback_keyword, archive_ext) = if has_nvidia {
        ("bin-ubuntu-vulkan-x64", "bin-ubuntu-x64", ".tar.gz")
    } else {
        ("bin-ubuntu-x64", "bin-ubuntu-x64", ".tar.gz")
    };

    app.emit("llama-install-progress", serde_json::json!({
        "percent": 10, "message": "Querying latest llama.cpp release…"
    })).ok();

    // Fetch latest release info from GitHub API.
    //
    // Two agents deliberately. The 30s budget is right for a JSON API call and
    // hopeless for the archive, which is hundreds of megabytes — sharing one
    // agent is what produced "read llama.cpp failed: timed out reading
    // response" on a perfectly healthy connection.
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let dl_agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(600))
        .build();

    let release: serde_json::Value = agent
        .get("https://api.github.com/repos/ggerganov/llama.cpp/releases/latest")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GitHub API error: {}", e))?
        .into_json()
        .map_err(|e| format!("JSON parse error: {}", e))?;

    let assets = release["assets"]
        .as_array()
        .ok_or("no assets in release")?;

    // Find the main binary zip (not cudart)
    let find_asset = |keyword: &str| -> Option<(String, String)> {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str().unwrap_or("");
            let url = a["browser_download_url"].as_str().unwrap_or("");
            if name.contains(keyword) && name.starts_with("llama-") && name.ends_with(archive_ext) {
                Some((name.to_string(), url.to_string()))
            } else {
                None
            }
        })
    };

    let (asset_name, download_url) = find_asset(asset_keyword)
        .or_else(|| find_asset(fallback_keyword))
        .ok_or("could not find a suitable llama.cpp release asset")?;

    // Also grab cudart if using CUDA
    #[cfg(not(target_os = "windows"))]
    let cudart_url: Option<String> = None;   // Windows-only asset: CUDA runtime DLLs

    #[cfg(target_os = "windows")]
    let cudart_url = if has_nvidia {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str().unwrap_or("");
            let url = a["browser_download_url"].as_str().unwrap_or("");
            if name.starts_with("cudart-") && name.contains("cuda-12.4") && name.ends_with(".zip") {
                Some(url.to_string())
            } else {
                None
            }
        })
    } else {
        None
    };

    app.emit("llama-install-progress", serde_json::json!({
        "percent": 15, "message": format!("Downloading {}…", asset_name)
    })).ok();

    // Download and extract helper
    let download_and_extract = |url: &str, label: &str| -> Result<(), String> {
        let resp = dl_agent.get(url)
            .call()
            .map_err(|e| format!("download {} failed: {}", label, e))?;

        let mut bytes = Vec::new();
        resp.into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read {} failed: {}", label, e))?;

        // Windows ships .zip, Linux .tar.gz. Both are FLATTENED into
        // install_dir: the Linux tarball nests everything under build/bin/, and
        // the binaries locate their shared objects via RUNPATH=$ORIGIN, so they
        // must end up adjacent — which is the layout the Windows zips already
        // have.
        #[cfg(target_os = "windows")]
        {
            let cursor = std::io::Cursor::new(&bytes);
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| format!("zip open {} failed: {}", label, e))?;

            for i in 0..archive.len() {
                let mut file = archive.by_index(i)
                    .map_err(|e| format!("zip entry error: {}", e))?;
                let name = file.name().to_string();
                if name.ends_with('/') { continue; }

                let file_name = std::path::Path::new(&name)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if file_name.is_empty() { continue; }

                let out_path = install_dir.join(&file_name);
                let mut out_file = std::fs::File::create(&out_path)
                    .map_err(|e| format!("create file {} failed: {}", file_name, e))?;
                std::io::copy(&mut file, &mut out_file)
                    .map_err(|e| format!("extract {} failed: {}", file_name, e))?;
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let dec = flate2::read::GzDecoder::new(std::io::Cursor::new(&bytes));
            let mut archive = tar::Archive::new(dec);
            archive.set_preserve_permissions(true);

            for entry in archive.entries()
                .map_err(|e| format!("tar open {} failed: {}", label, e))?
            {
                let mut entry = entry.map_err(|e| format!("tar entry error: {}", e))?;
                let entry_type = entry.header().entry_type();
                let path = entry.path()
                    .map_err(|e| format!("tar path error: {}", e))?
                    .into_owned();
                let file_name = match path.file_name() {
                    Some(f) if !f.is_empty() => f.to_owned(),
                    _ => continue,
                };
                let out_path = install_dir.join(&file_name);

                // SONAME symlinks are load-bearing. The tarball ships
                // libggml-base.so.0 -> libggml-base.so.0.18.0 and friends, and
                // binaries record DT_NEEDED by SONAME — drop the links and every
                // executable dies at startup with "error while loading shared
                // libraries", even though the real file is sitting right there.
                // Retargeted to the basename because this tree is flattened.
                if entry_type.is_symlink() || entry_type.is_hard_link() {
                    let target = entry.link_name()
                        .map_err(|e| format!("tar link error: {}", e))?
                        .ok_or_else(|| format!("link {} has no target",
                                               file_name.to_string_lossy()))?;
                    let target_name = match target.file_name() {
                        Some(t) if !t.is_empty() => t.to_owned(),
                        _ => continue,
                    };
                    let _ = std::fs::remove_file(&out_path);
                    std::os::unix::fs::symlink(&target_name, &out_path)
                        .map_err(|e| format!("link {} failed: {}",
                                             file_name.to_string_lossy(), e))?;
                    continue;
                }

                if !entry_type.is_file() { continue; }

                // unpack() applies the mode from the tar header — without the
                // executable bit the install "succeeds" and then every run fails
                // with Permission denied.
                let _ = std::fs::remove_file(&out_path);
                entry.unpack(&out_path)
                    .map_err(|e| format!("extract {} failed: {}",
                                         file_name.to_string_lossy(), e))?;
            }
        }

        Ok(())
    };

    // Download main binary
    download_and_extract(&download_url, "llama.cpp")?;

    app.emit("llama-install-progress", serde_json::json!({
        "percent": 70, "message": "Binaries extracted."
    })).ok();

    // Download CUDA runtime if applicable
    if let Some(ref cudart) = cudart_url {
        app.emit("llama-install-progress", serde_json::json!({
            "percent": 75, "message": "Downloading CUDA runtime…"
        })).ok();

        download_and_extract(cudart, "cudart")?;
    }

    // Verify
    let llama_path = model_loader::llama_local_path();
    if !llama_path.is_file() {
        return Err(format!("installation finished but {} not found in {}", 
            model_loader::LLAMA_BIN, install_dir.display()));
    }

    app.emit("llama-install-progress", serde_json::json!({
        "percent": 100, "message": "llama.cpp installed successfully."
    })).ok();

    Ok(llama_path.to_string_lossy().into_owned())
}

fn main() {
    let cancel_flag = CancelFlag(Arc::new(AtomicBool::new(false)));

    tauri::Builder::default()
        .manage(new_shared_state())
        .manage(cancel_flag)
        // Window-event tracing. There was no window-event handler at all, which
        // is why "the ✕ does nothing" could not be diagnosed: nothing recorded
        // whether the click ever reached the app. This separates the two
        // possibilities outright — if CloseRequested is logged, the click
        // arrives and something downstream ignores it; if nothing is logged,
        // the click never reaches the toolkit and the problem is below Tauri
        // (compositor, GTK, or a stray always-on-top surface).
        .on_window_event(|window, event| {
            let label = window.label();
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    eprintln!("[window:{}] CloseRequested", label)
                }
                tauri::WindowEvent::Destroyed => eprintln!("[window:{}] Destroyed", label),
                tauri::WindowEvent::Focused(focused) => {
                    eprintln!("[window:{}] Focused({})", label, focused)
                }
                tauri::WindowEvent::Resized(size) => {
                    eprintln!("[window:{}] Resized({}x{})", label, size.width, size.height)
                }
                tauri::WindowEvent::Moved(pos) => {
                    eprintln!("[window:{}] Moved({},{})", label, pos.x, pos.y)
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            startup_scan,
            cmd_load_pdf,
            cmd_auto_run,
            cmd_run_with_stored_config,
            cmd_apply_configuration,
            cmd_resolve_conflict,
            cmd_start_run,
            cmd_cancel_run,
            cmd_download_model,
            cmd_retry_model_download,
            cmd_cancel_model_download,
            cmd_get_model_library_path,
            cmd_browse_model_library,
            cmd_reset_model_library,
            cmd_set_model_library,
            cmd_model_library_info,
            cmd_open_model_library,
            cmd_open_output_folder,
            cmd_list_model_library,
            cmd_list_library_subfolders,
            cmd_ctx_vram_profile,
            cmd_download_hf_model,
            cmd_list_partial_downloads,
            cmd_delete_partial_download,
            cmd_begin_run,
            cmd_detect_vram,
            cmd_link_hrt,
            cmd_detect_llama,
            cmd_install_llama,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start AiSmartGuy");
}
