// AiSmartGuy — model_fetcher crate  (Phase 4)

pub mod api;
pub mod cache;
pub mod downloader;
pub mod errors;
pub mod hf_client;
pub mod progress;
pub mod shard_detector;
pub mod state;
pub mod types;
pub mod verifier;

pub use api::ModelFetcher;
pub use errors::{FetchResult, ModelFetchError};
pub use types::{FetchProgressEvent, HfFile, ModelFetchRequest, ModelShard};
