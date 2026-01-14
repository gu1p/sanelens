use std::time::{SystemTime, UNIX_EPOCH};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::support::constants::PROJECT_PREFIX;

pub fn new_run_id() -> String {
    let mut bytes = [0u8; 3];
    if getrandom::getrandom(&mut bytes).is_ok() {
        return format!("run_{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2]);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = u64::try_from(now.as_nanos()).unwrap_or(u64::MAX);
    let pid = u64::from(std::process::id());
    let mixed = nanos ^ (pid << 16);
    let hex = format!("{:06x}", mixed & 0x00ff_ffff);
    format!("run_{hex}")
}

pub fn project_name_from_run_id(run_id: &str) -> String {
    format!("{PROJECT_PREFIX}{run_id}")
}

pub fn run_started_at() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::now_utc().unix_timestamp().to_string())
}
