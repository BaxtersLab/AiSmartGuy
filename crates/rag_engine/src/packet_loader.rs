use std::path::Path;

use error_system::RagError;

use crate::packet::RagPacket;
use crate::packet_validator::validate_packet;

/// Load all `.json` RAG packet files from `model_path`, parse each, validate,
/// and return the successfully loaded packets. Malformed packets are skipped
/// (logged by caller via the returned error list).
pub fn load_packets(
    model_path: &Path,
) -> Result<(Vec<RagPacket>, Vec<(String, RagError)>), RagError> {
    let mut packets = Vec::new();
    let mut skipped = Vec::new();

    let read_dir = std::fs::read_dir(model_path).map_err(|e| {
        RagError::MissingPacket(format!("cannot read directory {:?}: {}", model_path, e))
    })?;

    let mut paths: Vec<std::path::PathBuf> = read_dir
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();

    // Sort files by name for deterministic ordering
    paths.sort();

    // Guard: reject directories with an unreasonable number of files.
    const MAX_PACKET_FILES: usize = 500;
    if paths.len() > MAX_PACKET_FILES {
        return Err(RagError::MalformedPacket(format!(
            "too many RAG packet files ({}) — max is {}",
            paths.len(),
            MAX_PACKET_FILES,
        )));
    }

    /// Max size for a single packet file (10 MB).
    const MAX_PACKET_BYTES: u64 = 10 * 1024 * 1024;

    for path in paths {
        // Check file size before reading to avoid multi-GB allocations.
        let file_size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) => {
                skipped.push((
                    path.display().to_string(),
                    RagError::MalformedPacket(format!("cannot stat file: {}", e)),
                ));
                continue;
            }
        };
        if file_size > MAX_PACKET_BYTES {
            skipped.push((
                path.display().to_string(),
                RagError::MalformedPacket(format!(
                    "file too large ({} bytes, max {})",
                    file_size, MAX_PACKET_BYTES
                )),
            ));
            continue;
        }

        let json = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                skipped.push((
                    path.display().to_string(),
                    RagError::MalformedPacket(e.to_string()),
                ));
                continue;
            }
        };

        let mut packet: RagPacket = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((
                    path.display().to_string(),
                    RagError::MalformedPacket(e.to_string()),
                ));
                continue;
            }
        };

        packet.source_path = path.clone();

        if let Err(e) = validate_packet(&packet) {
            skipped.push((path.display().to_string(), e));
            continue;
        }

        packets.push(packet);
    }

    Ok((packets, skipped))
}
