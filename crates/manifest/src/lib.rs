pub mod schema;
pub mod errors;
pub mod defaults;
pub mod serializer;
pub mod deserializer;
pub mod validator;
pub mod versioning;
pub mod merger;

pub use schema::{
    Manifest, ModelConfig, ModelSet, OptimizationState, PartialRunInfo,
    RagPacketMap, ResourceThrottle, RunMode, SourcePdf,
};
pub use defaults::default_manifest;
pub use serializer::{serialize, write_to_file};
pub use deserializer::{deserialize, load_from_file};
pub use validator::validate;
pub use versioning::upgrade_if_needed;
pub use merger::merge;
