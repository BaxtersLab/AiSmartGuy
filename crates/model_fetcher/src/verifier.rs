use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::cache::model_cache_dir;
use crate::errors::{FetchResult, ModelFetchError};
use crate::state::ModelFetchState;
use crate::types::ModelFetchRequest;

/// Verify all downloaded shards for a request.
///
/// For each shard:
///   1. Confirm the file exists on disk.
///   2. Confirm the file size matches the expected size.
///   3. If a SHA-256 is available from HF metadata, compute and compare.
pub fn verify_shards(
    state: &ModelFetchState,
    request: &ModelFetchRequest,
) -> FetchResult<()> {
    let dir = model_cache_dir(request);

    for shard in &state.shards {
        let path = dir.join(&shard.filename);

        if !path.exists() {
            return Err(ModelFetchError::MissingShard(format!(
                "shard not found on disk: {}",
                shard.filename
            )));
        }

        // File size check.
        if shard.size > 0 {
            let on_disk = std::fs::metadata(&path)
                .map_err(|e| ModelFetchError::IoError(e.to_string()))?
                .len();
            if on_disk != shard.size {
                return Err(ModelFetchError::IntegrityError(format!(
                    "{}: expected {} bytes, found {} bytes",
                    shard.filename, shard.size, on_disk
                )));
            }
        }

        // SHA-256 check (only when the HF API provided a checksum).
        if let Some(expected_hex) = &shard.sha256 {
            let actual_hex = sha256_of_file(&path)?;
            if actual_hex != *expected_hex {
                return Err(ModelFetchError::IntegrityError(format!(
                    "{}: SHA-256 mismatch — expected {}, got {}",
                    shard.filename, expected_hex, actual_hex
                )));
            }
        }
    }

    Ok(())
}

// ---- helpers ---------------------------------------------------------------

/// Compute the SHA-256 of a file using a streaming BufReader (64 KB buffer).
fn sha256_of_file(path: &Path) -> FetchResult<String> {
    let file = std::fs::File::open(path)
        .map_err(|e| ModelFetchError::IoError(e.to_string()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| ModelFetchError::IoError(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sha256_known_value() {
        let dir = std::env::temp_dir();
        let path = dir.join("aismartguy_hash_test.bin");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello world").unwrap();
        }
        let hash = sha256_of_file(&path).unwrap();
        // sha256("hello world") = b94d27b9934d3e08...
        assert!(hash.starts_with("b94d27b9"));
        let _ = std::fs::remove_file(&path);
    }
}
