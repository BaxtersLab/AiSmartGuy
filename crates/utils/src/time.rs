use chrono::Utc;

/// Returns the current UTC time as an ISO 8601 string, e.g. "2026-04-01T17:40:22Z".
pub fn now_iso8601() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Generates a unique run ID based on current UTC time, e.g. "Run_2026-04-01_17-40-22".
pub fn generate_run_id() -> String {
    Utc::now().format("Run_%Y-%m-%d_%H-%M-%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_format() {
        let ts = now_iso8601();
        // Basic format check: 20 chars, ends with Z, contains T
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
    }

    #[test]
    fn run_id_format() {
        let id = generate_run_id();
        assert!(id.starts_with("Run_"));
        assert!(id.len() >= 23);
    }
}
