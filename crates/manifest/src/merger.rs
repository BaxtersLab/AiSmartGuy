use crate::schema::{Manifest, PartialRunInfo};

/// Merge two `PartialRunInfo` values, unioning their failure lists.
fn merge_partial_run(
    base: Option<PartialRunInfo>,
    incoming: Option<PartialRunInfo>,
) -> Option<PartialRunInfo> {
    match (base, incoming) {
        (None, None) => None,
        (Some(p), None) | (None, Some(p)) => Some(p),
        (Some(base), Some(incoming)) => {
            let mut model_failures = base.model_failures;
            for m in incoming.model_failures {
                if !model_failures.contains(&m) {
                    model_failures.push(m);
                }
            }
            let mut failed_chunks = base.failed_chunks;
            for c in incoming.failed_chunks {
                if !failed_chunks.contains(&c) {
                    failed_chunks.push(c);
                }
            }
            Some(PartialRunInfo {
                model_failures,
                failed_chunks,
                fusion_partial: base.fusion_partial || incoming.fusion_partial,
            })
        }
    }
}

/// Merge two manifests: `incoming` fields override `base`.
/// Missing fields in incoming are filled from base.
/// categories_active and rag_packets_used are unioned.
pub fn merge(base: Manifest, incoming: Manifest) -> Manifest {
    // Union categories
    let mut categories = base.categories_active.clone();
    for cat in &incoming.categories_active {
        if !categories.contains(cat) {
            categories.push(cat.clone());
        }
    }

    // Union rag_packets_used
    let mut rag_packets = base.rag_packets_used.clone();
    for (model_key, packets) in &incoming.rag_packets_used {
        let entry = rag_packets.entry(model_key.clone()).or_default();
        for p in packets {
            if !entry.contains(p) {
                entry.push(p.clone());
            }
        }
    }

    Manifest {
        manifest_version: incoming.manifest_version,
        engine_version: incoming.engine_version,
        run_id: incoming.run_id,
        timestamp: incoming.timestamp,
        source_pdf: incoming.source_pdf,
        mode: incoming.mode,
        models: incoming.models,
        rag_packets_used: rag_packets,
        categories_active: categories,
        optimization_state: incoming.optimization_state,
        resource_throttle: incoming.resource_throttle,
        partial_run: merge_partial_run(base.partial_run, incoming.partial_run),
        notes: incoming.notes.or(base.notes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::default_manifest;

    #[test]
    fn merge_unions_categories() {
        let mut base = default_manifest();
        base.categories_active = vec!["fallacies".to_string()];
        let mut incoming = default_manifest();
        incoming.categories_active = vec!["euphemisms".to_string()];
        let merged = merge(base, incoming);
        assert!(merged.categories_active.contains(&"fallacies".to_string()));
        assert!(merged.categories_active.contains(&"euphemisms".to_string()));
    }

    #[test]
    fn merge_no_category_duplication() {
        let mut base = default_manifest();
        base.categories_active = vec!["fallacies".to_string()];
        let mut incoming = default_manifest();
        incoming.categories_active = vec!["fallacies".to_string()];
        let merged = merge(base, incoming);
        let count = merged
            .categories_active
            .iter()
            .filter(|c| *c == "fallacies")
            .count();
        assert_eq!(count, 1);
    }
}
