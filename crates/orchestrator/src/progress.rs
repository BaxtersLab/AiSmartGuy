use std::sync::Mutex;
use crate::types::OrchestratorProgressEvent;

type ProgressCb = Box<dyn Fn(&OrchestratorProgressEvent) + Send + 'static>;
static PROGRESS_CB: Mutex<Option<ProgressCb>> = Mutex::new(None);

/// Register a callback that receives every orchestrator progress event.
pub fn set_progress_callback<F>(cb: F)
where
    F: Fn(&OrchestratorProgressEvent) + Send + 'static,
{
    *PROGRESS_CB.lock().unwrap() = Some(Box::new(cb));
}

/// Remove any registered progress callback.
pub fn clear_progress_callback() {
    *PROGRESS_CB.lock().unwrap() = None;
}

/// Emit a progress event to stdout as a structured JSON line,
/// and invoke the registered callback (if any).
pub fn emit_progress(event: &OrchestratorProgressEvent) {
    // Call the registered callback if any
    if let Ok(guard) = PROGRESS_CB.lock() {
        if let Some(cb) = guard.as_ref() {
            cb(event);
        }
    }

    // Also print to stdout for logging/debugging.
    println!(
        r#"{{"event":"orchestrator_progress","stage":"{}","message":"{}","percent":{:.4}}}"#,
        escape_json(&event.stage),
        escape_json(&event.message),
        event.percent,
    );
}

/// JSON string escaping for control characters, backslash, and double-quote.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"'  => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                // Escape control chars as \u00XX.
                for unit in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out
}
