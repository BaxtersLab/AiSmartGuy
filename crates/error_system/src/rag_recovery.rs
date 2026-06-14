use std::path::Path;

use crate::types::RecoveryAction;

/// Handle a malformed RAG packet: always skip, never abort.
/// The packet path and error message are accepted for logging by the caller.
pub fn handle_malformed_packet(
    _packet_path: &Path,
    _error: &str,
) -> RecoveryAction {
    RecoveryAction::SkipRagPacket
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_returns_skip() {
        let action = handle_malformed_packet(
            Path::new("/some/packet.json"),
            "missing field",
        );
        assert_eq!(action, RecoveryAction::SkipRagPacket);
    }
}
