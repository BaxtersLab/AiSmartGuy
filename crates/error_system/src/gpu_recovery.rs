/// GPU fallback: if the current setting is GPU (auto), fall back to CPU-only.
/// Returns None if already on CPU.
pub fn downgrade_gpu(current_setting: &str) -> Option<String> {
    let s = current_setting.to_uppercase();
    if s == "CPU" {
        None
    } else {
        Some("CPU".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_downgrades_to_cpu() {
        assert_eq!(downgrade_gpu("GPU"), Some("CPU".to_string()));
        assert_eq!(downgrade_gpu("12GB"), Some("CPU".to_string()));
        assert_eq!(downgrade_gpu("auto"), Some("CPU".to_string()));
    }

    #[test]
    fn cpu_returns_none() {
        assert_eq!(downgrade_gpu("CPU"), None);
        assert_eq!(downgrade_gpu("cpu"), None);
    }
}
