use std::path::{Path, PathBuf};

use crate::errors::UtilsError;

/// Resolve to a canonical absolute path. Requires the path to already exist.
pub fn normalize(path: &Path) -> Result<PathBuf, UtilsError> {
    std::fs::canonicalize(path)
        .map_err(|e| UtilsError::PathError(e.to_string()))
}

pub fn file_exists(path: &Path) -> bool {
    path.is_file()
}

pub fn dir_exists(path: &Path) -> bool {
    path.is_dir()
}

/// Join a base path with a relative child segment.
pub fn join(base: &Path, child: &str) -> PathBuf {
    base.join(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_exists_returns_false_for_missing() {
        assert!(!file_exists(Path::new("/no/such/file.xyz")));
    }

    #[test]
    fn dir_exists_returns_false_for_missing() {
        assert!(!dir_exists(Path::new("/no/such/directory_xyz")));
    }

    #[test]
    fn join_builds_path() {
        let base = PathBuf::from("/tmp");
        let result = join(&base, "child");
        assert_eq!(result, PathBuf::from("/tmp/child"));
    }
}
