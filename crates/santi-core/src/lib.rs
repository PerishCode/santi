mod assembly;
mod context;
pub mod environment;

pub mod service;
pub mod workspace;

pub use assembly::timeline::{Preview as ProviderPreview, provider_input, provider_preview};
pub use santi_error as error;
pub use santi_error::*;
pub use santi_estate::{InboxDraft, Store};
pub use santi_model::{
    Health, Timestamp, budget, compact, downstream, drive, effect, environ, event, ingest, job,
    material, message, now, receipt, soul, stamped, strand, stream, tag, thinking, tool, trace,
    turn, webhook,
};
pub use santi_object as object;
pub use santi_provider::Item;
pub use workspace::{MEMORY, SOULSPACE, STRANDSPACE, housed, parsed, soulward, strandward};

pub const GENESIS: &str = "soul_default";
pub const SYSTEM: &str = "santi";
