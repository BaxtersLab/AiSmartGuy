use crate::schema::Manifest;

/// Upgrade a manifest to the current version if necessary.
/// For now: v1.0 is the only version — passthrough.
/// Future versions add fields as `Option` so old manifests load fine.
pub fn upgrade_if_needed(manifest: Manifest) -> Manifest {
    // When v1.1 is introduced:
    //   if manifest.manifest_version == "1.0" { fill new optional fields }
    manifest
}
