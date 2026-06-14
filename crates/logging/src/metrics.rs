use std::collections::HashMap;
use std::path::Path;

use crate::file_writer::write_line;
use utils::time::now_iso8601;

/// Log a performance metric to the dedicated metrics.log file.
/// Failures are silently ignored — metrics must never abort execution.
pub fn log_metric(
    metrics_path: &Path,
    name: &str,
    value: f64,
    metadata: Option<HashMap<String, String>>,
) {
    let meta_str = match metadata {
        Some(m) if !m.is_empty() => {
            let pairs: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("\"{}\": \"{}\"", k, v))
                .collect();
            format!("metadata: {{ {} }}\n", pairs.join(", "))
        }
        _ => String::new(),
    };

    let line = format!(
        "[{}]\nmetric: \"{}\"\nvalue: {}\n{}",
        now_iso8601(),
        name,
        value,
        meta_str
    );

    let _ = write_line(metrics_path, &line);
}
