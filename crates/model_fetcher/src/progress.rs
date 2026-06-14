use crate::types::FetchProgressEvent;

/// Emit a progress event to whatever sink is configured.
///
/// In the current implementation this writes a structured log line to stdout
/// so Tauri can capture it via the process stdio bridge.  When the UI crate
/// is wired up in Phase 8, this will be replaced by a Tauri event emission.
pub fn emit_progress(event: &FetchProgressEvent) {
    // JSON-style structured line that the UI process can parse.
    println!(
        "{{\"event\":\"model_download_progress\",\"filename\":\"{}\",\
         \"percent\":{:.1},\"total_percent\":{:.1},\
         \"downloaded_bytes\":{},\"total_bytes\":{}}}",
        event.filename,
        event.percent * 100.0,
        event.total_percent * 100.0,
        event.downloaded_bytes,
        event.total_bytes
    );
}
