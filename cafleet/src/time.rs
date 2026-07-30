use chrono::{DateTime, Utc};

use crate::error::CafleetError;

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

/// Emit the canonical storage form: fixed-width 6-digit microseconds and the
/// literal `+00:00` offset, so rows sort lexicographically (SPEC §5.1).
pub fn format_utc(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
}

/// Lenient reader for externally supplied values: accepts a missing fractional
/// part and any UTC offset spelling. Production always uses [`format_utc`].
pub fn parse_lenient(value: &str) -> Result<DateTime<Utc>, CafleetError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| CafleetError::Usage(format!("invalid timestamp '{value}': {e}")))
}
