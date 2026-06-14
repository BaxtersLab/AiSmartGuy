use crate::types::RecoveryContext;

/// Record a model failure and/or a failed chunk into the recovery context.
/// Does not touch the manifest — the orchestrator applies context → manifest updates.
pub fn mark_partial_run(
    context: &mut RecoveryContext,
    model_name: &str,
    chunk_id: Option<usize>,
) {
    let model_key = model_name.to_string();
    if !context.model_failures.contains(&model_key) {
        context.model_failures.push(model_key);
    }

    if let Some(id) = chunk_id {
        let chunk_key = format!("chunk_{:03}", id);
        if !context.failed_chunks.contains(&chunk_key) {
            context.failed_chunks.push(chunk_key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_model_failure_once() {
        let mut ctx = RecoveryContext::default();
        mark_partial_run(&mut ctx, "model1", None);
        mark_partial_run(&mut ctx, "model1", None);
        assert_eq!(ctx.model_failures.len(), 1);
    }

    #[test]
    fn records_chunk_failure() {
        let mut ctx = RecoveryContext::default();
        mark_partial_run(&mut ctx, "model1", Some(6));
        assert!(ctx.failed_chunks.contains(&"chunk_006".to_string()));
    }

    #[test]
    fn deduplicates_chunks() {
        let mut ctx = RecoveryContext::default();
        mark_partial_run(&mut ctx, "model1", Some(1));
        mark_partial_run(&mut ctx, "model1", Some(1));
        assert_eq!(ctx.failed_chunks.len(), 1);
    }
}
