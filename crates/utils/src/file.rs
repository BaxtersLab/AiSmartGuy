use std::path::Path;

use crate::errors::UtilsError;

pub fn read_to_string(path: &Path) -> Result<String, UtilsError> {
    std::fs::read_to_string(path)
        .map_err(|e| UtilsError::IoError(e.to_string()))
}

pub fn write_string(path: &Path, contents: &str) -> Result<(), UtilsError> {
    std::fs::write(path, contents)
        .map_err(|e| UtilsError::IoError(e.to_string()))
}

pub fn append_string(path: &Path, contents: &str) -> Result<(), UtilsError> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| UtilsError::IoError(e.to_string()))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| UtilsError::IoError(e.to_string()))?;
    file.flush()
        .map_err(|e| UtilsError::IoError(e.to_string()))
}

pub fn ensure_dir(path: &Path) -> Result<(), UtilsError> {
    std::fs::create_dir_all(path)
        .map_err(|e| UtilsError::IoError(e.to_string()))
}

pub fn copy_file(src: &Path, dst: &Path) -> Result<(), UtilsError> {
    std::fs::copy(src, dst)
        .map(|_| ())
        .map_err(|e| UtilsError::IoError(e.to_string()))
}

/// Write `contents` to `path` atomically:
/// write to a `.tmp` sibling, then rename over the original.
pub fn atomic_write(path: &Path, contents: &str) -> Result<(), UtilsError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)
        .map_err(|e| UtilsError::IoError(format!("atomic write (tmp): {}", e)))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| UtilsError::IoError(format!("atomic rename: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn write_and_read_roundtrip() {
        let path = tmp_path("utils_test_write_read.txt");
        write_string(&path, "hello").unwrap();
        let s = read_to_string(&path).unwrap();
        assert_eq!(s, "hello");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ensure_dir_creates_nested() {
        let dir = tmp_path("utils_test_nested/a/b/c");
        ensure_dir(&dir).unwrap();
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(tmp_path("utils_test_nested"));
    }

    #[test]
    fn atomic_write_replaces_existing() {
        let path = tmp_path("utils_test_atomic.txt");
        write_string(&path, "old").unwrap();
        atomic_write(&path, "new").unwrap();
        assert_eq!(read_to_string(&path).unwrap(), "new");
        let _ = std::fs::remove_file(&path);
    }
}
