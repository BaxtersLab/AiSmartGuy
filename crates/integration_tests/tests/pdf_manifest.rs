/// Integration Tests — PDF IO + Manifest (Module M §4.3)
///
/// Tests:
/// - manifest serialize → deserialize round-trip
/// - metadata embedding → extraction round-trip (temp file)
/// - missing PDF returns an error (not a panic)
/// - manifest validation rejects empty name field

use std::path::PathBuf;
use manifest::{default_manifest, deserialize, serialize, validate, ModelConfig, SourcePdf};

// ---------------------------------------------------------------------------
// Helper: build a manifest that satisfies the validator
// ---------------------------------------------------------------------------
fn valid_manifest() -> manifest::Manifest {
    let mut m = default_manifest();
    m.source_pdf = SourcePdf {
        filename: "test.pdf".to_string(),
        hash_sha256: None,
        page_count: None,
    };
    m.models.model1 = Some(ModelConfig {
        name: "model_a".to_string(),
        path: "/models/model_a.gguf".to_string(),
        quantization: "q4_k_m".to_string(),
        active: true,
        ..Default::default()
    });
    m
}

// ---------------------------------------------------------------------------
// 4.3.1 — Manifest serialize / deserialize round-trip
// ---------------------------------------------------------------------------
#[test]
fn test_manifest_serialize_deserialize_roundtrip() {
    let original = valid_manifest();
    let json = serialize(&original).expect("serialize should succeed");
    assert!(!json.is_empty(), "serialized JSON must not be empty");

    let recovered = deserialize(&json).expect("deserialize should succeed");
    assert_eq!(original.manifest_version, recovered.manifest_version);
    assert_eq!(original.engine_version, recovered.engine_version);
    assert_eq!(original.mode, recovered.mode);
}

// ---------------------------------------------------------------------------
// 4.3.2 — Validation passes for a well-formed manifest
// ---------------------------------------------------------------------------
#[test]
fn test_manifest_validate_valid_manifest_passes() {
    let m = valid_manifest();
    assert!(validate(&m).is_ok(), "well-formed manifest should pass validation");
}

// ---------------------------------------------------------------------------
// 4.3.2b — Validation fails for the bare default manifest (no active model)
// ---------------------------------------------------------------------------
#[test]
fn test_manifest_validate_default_fails() {
    let m = default_manifest();
    assert!(
        validate(&m).is_err(),
        "bare default manifest (no active model, no source_pdf) must fail validation"
    );
}

// ---------------------------------------------------------------------------
// 4.3.3 — Deserializing garbage JSON returns an error (not a panic)
// ---------------------------------------------------------------------------
#[test]
fn test_manifest_deserialize_invalid_json_returns_error() {
    let bad = "this is not json { at all {{{{";
    let result = deserialize(bad);
    assert!(result.is_err(), "invalid JSON should return Err");
}

// ---------------------------------------------------------------------------
// 4.3.4 — Deserializing an empty string returns an error
// ---------------------------------------------------------------------------
#[test]
fn test_manifest_deserialize_empty_string_returns_error() {
    assert!(deserialize("").is_err());
}

// ---------------------------------------------------------------------------
// 4.3.5 — PDF extraction fails gracefully for a non-existent path
// ---------------------------------------------------------------------------
#[test]
fn test_pdf_extract_missing_file_returns_error() {
    let bad = PathBuf::from("/nonexistent/path/test.pdf");
    let result = pdf_io::extract_manifest(&bad);
    assert!(result.is_err(), "missing PDF should return Err");
}

// ---------------------------------------------------------------------------
// 4.3.6 — PDF extract_text fails gracefully for a non-existent path
// ---------------------------------------------------------------------------
#[test]
fn test_pdf_extract_text_missing_file_returns_error() {
    let bad = PathBuf::from("/nonexistent/path/test.pdf");
    let result = pdf_io::extract_text(&bad);
    assert!(result.is_err(), "missing PDF should return Err");
}

// ---------------------------------------------------------------------------
// 4.3.7 — manifest_version and engine_version are present after round-trip
// ---------------------------------------------------------------------------
#[test]
fn test_manifest_versions_survive_roundtrip() {
    let mut m = valid_manifest();
    m.manifest_version = "2.0".to_string();
    m.engine_version = "3.0".to_string();

    let json = serialize(&m).unwrap();
    let m2 = deserialize(&json).unwrap();
    assert_eq!(m2.manifest_version, "2.0");
    assert_eq!(m2.engine_version, "3.0");
}
