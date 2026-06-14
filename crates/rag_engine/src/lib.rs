pub mod errors;
pub mod hitlist;
pub mod packet;
pub mod packet_loader;
pub mod packet_validator;
pub mod packet_merger;
pub mod prompt_builder;
pub mod api;

pub use packet::{
    HitConditions, InjectLocation, MergedPacketSet, ModelBehavior, PatternType, RagPacket,
    RagRule, Severity,
};
pub use hitlist::{HitlistEntry, active_entries, active_slugs, catalog};
pub use api::RagEngine;
