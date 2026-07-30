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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[test]
    fn format_emits_six_digit_microseconds_and_offset() {
        let dt =
            Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap() + Duration::microseconds(198_561);
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
        let dt =
            Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap() + Duration::microseconds(198_561);
        assert_eq!(parse_lenient(&format_utc(dt)).unwrap(), dt);
    }

    #[test]
    fn parse_accepts_missing_fractional_part() {
        let expected = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
        assert_eq!(
            parse_lenient("2026-07-30T09:53:43+00:00").unwrap(),
            expected
        );
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
}
