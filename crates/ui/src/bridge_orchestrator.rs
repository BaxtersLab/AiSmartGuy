use std::path::PathBuf;
use orchestrator::Orchestrator;
use crate::errors::{UiError, UiResult};

/// Run the full orchestrator pipeline: load manifest, process PDF, write output.
///
/// Returns the path to the final output PDF.
pub fn run_orchestrator(
    manifest_path: PathBuf,
    run_dir: PathBuf,
    pdf_path: PathBuf,
) -> UiResult<PathBuf> {
    let mut orchestrator = Orchestrator::new(manifest_path, run_dir)
        .map_err(|e| UiError::OrchestratorError(format!("{:?}", e)))?;

    let output_path = orchestrator
        .run(pdf_path)
        .map_err(|e| UiError::OrchestratorError(format!("{:?}", e)))?;

    Ok(output_path)
}
