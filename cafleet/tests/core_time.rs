//! Step 3 contract tests: `cafleet::time` — the pinned UTC timestamp writer
//! and the lenient parser (SPEC §5.1, amendment A4).
//!
//! Expected public API:
//! - `format_utc(chrono::DateTime<chrono::Utc>) -> String` — always emits
//!   `YYYY-MM-DDTHH:MM:SS.ffffff+00:00`: fixed-width 6-digit microseconds
//!   (including when zero) and the literal `+00:00` offset
//! - `parse_lenient(&str) -> Result<chrono::DateTime<chrono::Utc>, CafleetError>`
//!   — accepts a missing fractional part and any UTC offset spelling
//! - `now_utc() -> chrono::DateTime<chrono::Utc>` (wall clock; not tested here)

use cafleet::time::{format_utc, parse_lenient};
use chrono::{Duration, TimeZone, Utc};

#[test]
fn format_emits_six_digit_microseconds_and_offset() {
    let dt = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap()
        + Duration::microseconds(198_561);
    assert_eq!(format_utc(dt), "2026-07-30T09:53:43.198561+00:00");
}

#[test]
fn format_emits_zero_microseconds_as_000000() {
    let dt = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
    assert_eq!(format_utc(dt), "2026-07-30T09:53:43.000000+00:00");
}

#[test]
fn format_zero_pads_short_microsecond_values() {
    let dt = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap() + Duration::microseconds(7);
    assert_eq!(format_utc(dt), "2026-01-02T03:04:05.000007+00:00");
}

#[test]
fn format_round_trips_through_parse() {
    let dt = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap()
        + Duration::microseconds(198_561);
    assert_eq!(parse_lenient(&format_utc(dt)).unwrap(), dt);
}

#[test]
fn parse_accepts_missing_fractional_part() {
    let expected = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
    assert_eq!(parse_lenient("2026-07-30T09:53:43+00:00").unwrap(), expected);
}

#[test]
fn parse_accepts_z_offset_spelling() {
    let expected = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
    assert_eq!(parse_lenient("2026-07-30T09:53:43Z").unwrap(), expected);
}

#[test]
fn parse_rejects_garbage() {
    assert!(parse_lenient("yesterday at noon").is_err());
    assert!(parse_lenient("").is_err());
}

#[test]
fn formatted_timestamps_order_lexicographically() {
    let base = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
    let earlier = format_utc(base + Duration::microseconds(99_999));
    let later = format_utc(base + Duration::microseconds(100_000));
    assert!(earlier < later);
    let end_of_second = format_utc(base + Duration::microseconds(999_999));
    let next_second = format_utc(base + Duration::seconds(1));
    assert!(end_of_second < next_second);
}
