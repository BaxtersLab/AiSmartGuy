use crate::errors::{FetchResult, ModelFetchError};
use crate::types::{HfFile, ModelShard};

/// Detect all GGUF shards from a flat list of repo files.
///
/// Rules (from Module N §6):
///   - include files whose name ends with `.gguf` OR matches `*.gguf.<N>` (shard index)
///   - sort shards: the base `.gguf` file comes first, then `.gguf.1`, `.gguf.2`, …
///   - verify there are no index gaps
///   - require at least one shard
pub fn detect_shards(files: &[HfFile]) -> FetchResult<Vec<ModelShard>> {
    // Split files into "base" gguf and indexed shards.
    let mut base: Vec<&HfFile> = Vec::new();
    let mut shards_indexed: Vec<(u32, &HfFile)> = Vec::new();

    for file in files {
        let name = &file.rfilename;
        if is_shard_index(name) {
            let idx = parse_shard_index(name).unwrap();
            shards_indexed.push((idx, file));
        } else if name.ends_with(".gguf") {
            base.push(file);
        }
    }

    // Sort indexed shards by index value, deterministically.
    shards_indexed.sort_by_key(|(idx, _)| *idx);

    // Build ordered list: base file(s) first, then indexed shards.
    let mut ordered: Vec<&HfFile> = base;
    for (_, file) in &shards_indexed {
        ordered.push(file);
    }

    if ordered.is_empty() {
        return Err(ModelFetchError::MissingShard(
            "no .gguf files found in repo".to_string(),
        ));
    }

    // Verify shard index continuity (1, 2, 3, … with no gaps).
    if !shards_indexed.is_empty() {
        let indices: Vec<u32> = shards_indexed.iter().map(|(i, _)| *i).collect();
        let min = *indices.first().unwrap();
        let max = *indices.last().unwrap();
        let expected_count = (max - min + 1) as usize;
        if indices.len() != expected_count {
            return Err(ModelFetchError::MissingShard(format!(
                "shard index gap detected (expected {}, found {}): {:?}",
                expected_count,
                indices.len(),
                indices
            )));
        }
    }

    let result = ordered
        .into_iter()
        .map(|f| ModelShard {
            filename: f.rfilename.clone(),
            size: f.size.unwrap_or(0),
            sha256: f.sha256.clone(),
            downloaded: false,
        })
        .collect();

    Ok(result)
}

// ---- helpers ---------------------------------------------------------------

/// Returns true if the filename looks like `something.gguf.<number>`.
fn is_shard_index(name: &str) -> bool {
    parse_shard_index(name).is_some()
}

/// Parse the numeric shard index from `something.gguf.<N>`.
fn parse_shard_index(name: &str) -> Option<u32> {
    // Match: ends with `.gguf.<digits>`
    let dot_pos = name.rfind('.')?;
    let suffix = &name[dot_pos + 1..];
    let index: u32 = suffix.parse().ok()?;
    let prefix = &name[..dot_pos];
    if prefix.ends_with(".gguf") {
        Some(index)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HfFile;

    fn hf(name: &str, size: u64) -> HfFile {
        HfFile {
            rfilename: name.to_string(),
            size: Some(size),
            sha256: None,
        }
    }

    #[test]
    fn single_gguf_file() {
        let files = vec![hf("model.gguf", 1_000_000)];
        let shards = detect_shards(&files).unwrap();
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].filename, "model.gguf");
    }

    #[test]
    fn multi_shard_sorted() {
        let files = vec![
            hf("model.gguf.3", 1000),
            hf("model.gguf.1", 1000),
            hf("model.gguf.2", 1000),
        ];
        let shards = detect_shards(&files).unwrap();
        assert_eq!(shards.len(), 3);
        assert_eq!(shards[0].filename, "model.gguf.1");
        assert_eq!(shards[1].filename, "model.gguf.2");
        assert_eq!(shards[2].filename, "model.gguf.3");
    }

    #[test]
    fn gap_in_shards_returns_error() {
        let files = vec![
            hf("model.gguf.1", 1000),
            hf("model.gguf.3", 1000), // gap: missing .2
        ];
        assert!(detect_shards(&files).is_err());
    }

    #[test]
    fn no_gguf_files_returns_error() {
        let files = vec![hf("README.md", 100), hf("config.json", 200)];
        assert!(detect_shards(&files).is_err());
    }

    #[test]
    fn non_gguf_files_ignored() {
        let files = vec![
            hf("model.gguf", 5_000_000),
            hf("README.md", 100),
            hf("tokenizer.json", 200),
        ];
        let shards = detect_shards(&files).unwrap();
        assert_eq!(shards.len(), 1);
    }
}
