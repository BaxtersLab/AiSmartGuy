use std::sync::Mutex;

type LogCb = Box<dyn Fn(&str) + Send + 'static>;
static LOG_CB: Mutex<Option<LogCb>> = Mutex::new(None);

/// Register a callback that receives each stderr line from llama-cli.
pub fn set_log_callback<F>(cb: F)
where
    F: Fn(&str) + Send + 'static,
{
    *LOG_CB.lock().unwrap() = Some(Box::new(cb));
}

/// Remove any registered log callback.
pub fn clear_log_callback() {
    *LOG_CB.lock().unwrap() = None;
}

/// Send a line to the registered callback (if any).
pub fn emit_log_line(line: &str) {
    if let Ok(guard) = LOG_CB.lock() {
        if let Some(cb) = guard.as_ref() {
            cb(line);
        }
    }
}
