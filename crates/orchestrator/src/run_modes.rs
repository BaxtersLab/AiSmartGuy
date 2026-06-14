use manifest::RunMode;

/// Returns a `RunMode` display label for logging.
pub fn mode_label(mode: &RunMode) -> &'static str {
    match mode {
        RunMode::Single => "Single",
        RunMode::Dual => "Dual",
        RunMode::Full => "Full",
        RunMode::Optimized => "Optimized",
    }
}

/// Returns true if the mode should include model3.
pub fn includes_model3(mode: &RunMode) -> bool {
    matches!(mode, RunMode::Full | RunMode::Optimized)
}

/// Returns true if the mode should include model2.
pub fn includes_model2(mode: &RunMode) -> bool {
    matches!(mode, RunMode::Dual | RunMode::Full | RunMode::Optimized)
}
