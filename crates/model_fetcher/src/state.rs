use crate::types::ModelShard;

/// Live download state — updated by the downloader as bytes arrive.
#[derive(Debug, Default)]
pub struct ModelFetchState {
    pub shards: Vec<ModelShard>,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub complete: bool,
}

impl ModelFetchState {
    /// Build a fresh state from a shard list (none downloaded yet).
    pub fn from_shards(shards: Vec<ModelShard>) -> Self {
        let total_bytes = shards.iter().map(|s| s.size).sum();
        ModelFetchState {
            shards,
            total_bytes,
            downloaded_bytes: 0,
            complete: false,
        }
    }

    /// Overall download progress in [0.0, 1.0].
    pub fn progress(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.downloaded_bytes as f32 / self.total_bytes as f32
    }

    /// Mark a shard downloaded and credit its bytes.
    pub fn mark_shard_done(&mut self, filename: &str, bytes: u64) {
        for shard in &mut self.shards {
            if shard.filename == filename {
                shard.downloaded = true;
                break;
            }
        }
        self.downloaded_bytes += bytes;
        self.complete = self.shards.iter().all(|s| s.downloaded);
    }
}
