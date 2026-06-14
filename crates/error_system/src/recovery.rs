use crate::types::{ErrorClass, RecoveryAction, RecoveryContext};

/// Determine the next recovery action given an error class and the current run context.
/// All decisions are deterministic — same input always produces the same action.
pub fn recover(class: &ErrorClass, context: &mut RecoveryContext) -> RecoveryAction {
    match class {
        ErrorClass::Recoverable => {
            if context.recoverable_retries < 1 {
                context.recoverable_retries += 1;
                RecoveryAction::Retry
            } else {
                context.recoverable_retries = 0;
                RecoveryAction::MarkPartialRun
            }
        }
        ErrorClass::Retryable => {
            if context.retryable_retries < 3 {
                context.retryable_retries += 1;
                RecoveryAction::Retry
            } else {
                context.retryable_retries = 0;
                RecoveryAction::SkipModel
            }
        }
        ErrorClass::GpuFallback => {
            context.recoverable_retries = 0;
            context.retryable_retries = 0;
            RecoveryAction::DowngradeGpu
        }
        ErrorClass::RagSkip  => RecoveryAction::SkipRagPacket,
        ErrorClass::PartialRun => RecoveryAction::SkipChunk,
        ErrorClass::Fatal    => RecoveryAction::AbortRun,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recoverable_retries_once_then_marks_partial() {
        let mut ctx = RecoveryContext::default();
        assert_eq!(recover(&ErrorClass::Recoverable, &mut ctx), RecoveryAction::Retry);
        assert_eq!(recover(&ErrorClass::Recoverable, &mut ctx), RecoveryAction::MarkPartialRun);
    }

    #[test]
    fn retryable_retries_three_times_then_skips_model() {
        let mut ctx = RecoveryContext::default();
        for _ in 0..3 {
            assert_eq!(recover(&ErrorClass::Retryable, &mut ctx), RecoveryAction::Retry);
        }
        assert_eq!(recover(&ErrorClass::Retryable, &mut ctx), RecoveryAction::SkipModel);
    }

    #[test]
    fn fatal_aborts() {
        let mut ctx = RecoveryContext::default();
        assert_eq!(recover(&ErrorClass::Fatal, &mut ctx), RecoveryAction::AbortRun);
    }

    #[test]
    fn rag_skip_always_skips_packet() {
        let mut ctx = RecoveryContext::default();
        assert_eq!(recover(&ErrorClass::RagSkip, &mut ctx), RecoveryAction::SkipRagPacket);
        assert_eq!(recover(&ErrorClass::RagSkip, &mut ctx), RecoveryAction::SkipRagPacket);
    }
}
