use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

use crate::errors::UtilsError;

/// Compute the SHA-256 hex digest of a file using streaming reads.
/// Never loads the entire file into memory.
pub fn sha256_of_file(path: &Path) -> Result<String, UtilsError> {
    let file = std::fs::File::open(path)
        .map_err(|e| UtilsError::HashError(e.to_string()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| UtilsError::HashError(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Compute the SHA-256 hex digest of a string slice.
pub fn sha256_of_str(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_str_deterministic() {
        let a = sha256_of_str("hello");
        let b = sha256_of_str("hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn sha256_of_str_unique() {
        assert_ne!(sha256_of_str("hello"), sha256_of_str("world"));
    }
}
