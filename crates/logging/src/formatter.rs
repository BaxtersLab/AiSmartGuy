use std::collections::HashMap;

use crate::types::{LogEntry, LogLevel};
use utils::time::now_iso8601;

pub fn format_entry(
    level: LogLevel,
    subsystem: &str,
    message: &str,
    metadata: Option<HashMap<String, String>>,
) -> LogEntry {
    LogEntry {
        timestamp: now_iso8601(),
        subsystem: subsystem.to_lowercase(),
        level,
        message: message.to_string(),
        metadata,
    }
}

pub fn format_line(entry: &LogEntry) -> String {
    let header = format!(
        "[{}] [{}] [{}]",
        entry.timestamp, entry.subsystem, entry.level
    );
    let body = format!("message: \"{}\"", entry.message);

    match &entry.metadata {
        Some(m) if !m.is_empty() => {
            let pairs: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("\"{}\": \"{}\"", k, v))
                .collect();
            format!("{}\n{}\nmetadata: {{ {} }}\n", header, body, pairs.join(", "))
        }
        _ => format!("{}\n{}\n", header, body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_info_no_meta() {
        let entry = format_entry(LogLevel::Info, "orchestrator", "test message", None);
        let line = format_line(&entry);
        assert!(line.contains("[INFO]"));
        assert!(line.contains("[orchestrator]"));
        assert!(line.contains("test message"));
    }

    #[test]
    fn format_line_with_metadata() {
        let mut meta = HashMap::new();
        meta.insert("chunk".to_string(), "003".to_string());
        let entry = format_entry(LogLevel::Debug, "model_loader", "chunk done", Some(meta));
        let line = format_line(&entry);
        assert!(line.contains("metadata:"));
        assert!(line.contains("chunk"));
    }
}
