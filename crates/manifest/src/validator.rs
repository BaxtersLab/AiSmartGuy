use error_system::ManifestError;

use crate::schema::Manifest;

pub fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    // run_id must be non-empty and start with "Run_"
    if manifest.run_id.is_empty() {
        return Err(ManifestError::ValidationFailure(
            "run_id is empty".to_string(),
        ));
    }
    if !manifest.run_id.starts_with("Run_") {
        return Err(ManifestError::ValidationFailure(format!(
            "run_id format invalid: {}",
            manifest.run_id
        )));
    }

    // timestamp must be non-empty
    if manifest.timestamp.is_empty() {
        return Err(ManifestError::ValidationFailure(
            "timestamp is empty".to_string(),
        ));
    }

    // manifest_version must be non-empty
    if manifest.manifest_version.is_empty() {
        return Err(ManifestError::ValidationFailure(
            "manifest_version is empty".to_string(),
        ));
    }

    // at least one model must be active
    let any_model = manifest.models.model1.as_ref().map(|m| m.active).unwrap_or(false)
        || manifest.models.model2.as_ref().map(|m| m.active).unwrap_or(false)
        || manifest.models.model3.as_ref().map(|m| m.active).unwrap_or(false)
        || manifest.models.fusion.as_ref().map(|m| m.active).unwrap_or(false);

    if !any_model {
        return Err(ManifestError::ValidationFailure(
            "no active models defined".to_string(),
        ));
    }

    // source_pdf filename must be non-empty
    if manifest.source_pdf.filename.is_empty() {
        return Err(ManifestError::ValidationFailure(
            "source_pdf.filename is empty".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::default_manifest;
    use crate::schema::{ModelConfig, RunMode};

    fn valid_manifest() -> Manifest {
        let mut m = default_manifest();
        m.run_id = "Run_2026-04-02_10-00-00".to_string();
        m.timestamp = "2026-04-02T10:00:00Z".to_string();
        m.source_pdf.filename = "test.pdf".to_string();
        m.models.model1 = Some(ModelConfig {
            active: true,
            name: "test".to_string(),
            path: "/models/test.gguf".to_string(),
            quantization: "Q4_K_M".to_string(),
            ..Default::default()
        });
        m
    }

    #[test]
    fn valid_passes() {
        assert!(validate(&valid_manifest()).is_ok());
    }

    #[test]
    fn empty_run_id_fails() {
        let mut m = valid_manifest();
        m.run_id = String::new();
        assert!(validate(&m).is_err());
    }

    #[test]
    fn bad_run_id_format_fails() {
        let mut m = valid_manifest();
        m.run_id = "bad-id".to_string();
        assert!(validate(&m).is_err());
    }

    #[test]
    fn no_active_model_fails() {
        let mut m = valid_manifest();
        m.models.model1 = None;
        assert!(validate(&m).is_err());
    }
}
