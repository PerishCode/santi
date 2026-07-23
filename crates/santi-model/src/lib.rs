use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub mod budget;
mod codec;
mod edge;
mod ledger;
pub mod stream;

pub use edge::{downstream, drive, effect, event, ingest, receipt, webhook};
pub use ledger::{compact, material, message, soul, strand, thinking, tool, turn};

pub type Timestamp = String;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Health {
    pub ok: bool,
    pub service: String,
    pub degraded: bool,
    pub incidents: i64,
}

pub fn now() -> Timestamp {
    use jiff::fmt::temporal::DateTimePrinter;

    let now = jiff::Timestamp::now();
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&now, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    buf
}

pub fn stamped(system_time: std::time::SystemTime) -> Result<Timestamp, String> {
    use jiff::fmt::temporal::DateTimePrinter;

    let timestamp = jiff::Timestamp::try_from(system_time).map_err(|error| error.to_string())?;
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&timestamp, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    Ok(buf)
}

pub fn tag(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}
