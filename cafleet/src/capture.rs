//! Mode-exact capture facts and output dispatch shared by the two CLI paths.
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::io::Write;

use crate::{broker::records::MemberKind, error::CafleetError, output, presentation, time};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureSnapshot {
    pub content: String,
    pub captured_at: String,
    pub content_sha256: String,
}
impl CaptureSnapshot {
    pub(crate) fn from_raw(raw: &str, ansi: bool, now: DateTime<Utc>) -> Self {
        let content = if ansi {
            raw.to_owned()
        } else {
            output::strip_ansi(raw)
        };
        let content_sha256 = Sha256::digest(content.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            content,
            captured_at: time::format_utc(now),
            content_sha256,
        }
    }
}

pub(crate) struct MemberCapture {
    pub member_id: i64,
    pub pane_id: String,
    pub lines: i64,
    pub snapshot: CaptureSnapshot,
}
pub(crate) struct ScanEntry {
    pub member_id: i64,
    pub name: String,
    pub kind: MemberKind,
    pub coding_agent: String,
    pub pane_id: Option<String>,
    pub lines: i64,
    pub outcome: Result<CaptureSnapshot, String>,
}

pub(crate) fn write_member_capture(
    out: &mut dyn Write,
    capture: &MemberCapture,
    json: bool,
) -> Result<(), CafleetError> {
    let result = if json {
        writeln!(
            out,
            "{}",
            output::format_json(&presentation::member_capture(capture))
        )
    } else {
        out.write_all(capture.snapshot.content.as_bytes())
    };
    result.map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))
}
pub(crate) fn write_scan(
    out: &mut dyn Write,
    entries: &[ScanEntry],
    json: bool,
) -> Result<(), CafleetError> {
    let rendered = if json {
        output::format_json(&presentation::scan_json(entries))
    } else {
        presentation::scan_text(entries)
    };
    writeln!(out, "{rendered}").map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker;
    use serde_json::json;
    use std::io;
    const WHEN: &str = "2026-09-06T01:02:03.000007+00:00";
    const PLAIN_HASH: &str = "526a35d455e79594452b17d2378d09d811ce0fb0ece884ac1198c3fc619c10f4";
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(WHEN)
            .unwrap()
            .with_timezone(&Utc)
    }
    fn capture() -> MemberCapture {
        MemberCapture {
            member_id: 7,
            pane_id: "%9".into(),
            lines: 12,
            snapshot: CaptureSnapshot::from_raw("雪\n緑", false, now()),
        }
    }
    #[test]
    fn snapshot_strips_csi_and_cr_rewrites_before_hashing_utf8() {
        let snapshot = CaptureSnapshot::from_raw("old\r\x1b[31m雪\x1b[0m\nold\r緑", false, now());
        assert_eq!(
            snapshot,
            CaptureSnapshot {
                content: "雪\n緑".into(),
                captured_at: WHEN.into(),
                content_sha256: PLAIN_HASH.into()
            }
        );
    }
    #[test]
    fn snapshot_ansi_preserves_every_raw_byte_and_hashes_that_content() {
        let raw = "old\r\x1b[31m雪\x1b[0m\nold\r緑";
        let snapshot = CaptureSnapshot::from_raw(raw, true, now());
        assert_eq!(snapshot.content, raw);
        assert_eq!(snapshot.captured_at, WHEN);
        assert_eq!(
            snapshot.content_sha256,
            "b9bd2b04f3d96774f4916e31055b70707a34e21a07fb96125c9214df2b1ad191"
        );
    }
    #[test]
    fn empty_snapshot_has_the_standard_empty_sha256_in_both_modes() {
        for ansi in [false, true] {
            let snapshot = CaptureSnapshot::from_raw("", ansi, now());
            assert_eq!(snapshot.content, "");
            assert_eq!(snapshot.captured_at, WHEN);
            assert_eq!(
                snapshot.content_sha256,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            );
        }
    }
    #[test]
    fn member_presenter_and_writers_preserve_complete_wire_and_text_bytes() {
        let capture = capture();
        let expected = format!(
            "{{\"member_id\":7,\"pane_id\":\"%9\",\"lines\":12,\"content\":\"雪\\n緑\",\"captured_at\":\"{WHEN}\",\"content_sha256\":\"{PLAIN_HASH}\"}}"
        );
        assert_eq!(presentation::member_capture(&capture).to_string(), expected);
        for json in [false, true] {
            let mut out = Vec::new();
            write_member_capture(&mut out, &capture, json).unwrap();
            assert_eq!(
                out,
                if json {
                    format!("{expected}\n").into_bytes()
                } else {
                    "雪\n緑".as_bytes().to_vec()
                }
            );
        }
    }
    #[test]
    fn scan_presenters_keep_error_nulls_and_exact_text() {
        let entries = vec![
            ScanEntry {
                member_id: 7,
                name: "Director".into(),
                kind: broker::records::MemberKind::Director,
                coding_agent: "codex".into(),
                pane_id: Some("%9".into()),
                lines: 12,
                outcome: Ok(capture().snapshot),
            },
            ScanEntry {
                member_id: 8,
                name: "waiting".into(),
                kind: broker::records::MemberKind::Member,
                coding_agent: "claude".into(),
                pane_id: None,
                lines: 12,
                outcome: Err("pane not available (pending placement)".into()),
            },
        ];
        let expected = json!([
            {"member_id":7,"name":"Director","kind":"director","coding_agent":"codex","pane_id":"%9","lines":12,"content":"雪\n緑","captured_at":WHEN,"content_sha256":PLAIN_HASH,"error":null},
            {"member_id":8,"name":"waiting","kind":"member","coding_agent":"claude","pane_id":null,"lines":12,"content":null,"captured_at":null,"content_sha256":null,"error":"pane not available (pending placement)"}
        ]);
        assert_eq!(
            presentation::scan_json(&entries).to_string(),
            expected.to_string()
        );
        let expected_text = format!(
            "=== 7 (Director; kind=director; coding_agent=codex; pane=%9; captured_at={WHEN}) ===\n雪\n緑\n\n=== 8 (waiting; kind=member; coding_agent=claude; pane=—) ===\npane not available (pending placement)"
        );
        assert_eq!(presentation::scan_text(&entries), expected_text);
        let mut out = Vec::new();
        write_scan(&mut out, &entries, true).unwrap();
        assert_eq!(out, format!("{expected}\n").into_bytes());
        out.clear();
        write_scan(&mut out, &entries, false).unwrap();
        assert_eq!(out, format!("{expected_text}\n").into_bytes());
    }
    #[test]
    fn output_failures_propagate_from_both_capture_dispatchers() {
        struct Fail;
        impl io::Write for Fail {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("output unavailable"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for json in [false, true] {
            assert!(
                write_member_capture(&mut Fail, &capture(), json)
                    .unwrap_err()
                    .to_string()
                    .contains("output unavailable")
            );
            assert!(
                write_scan(&mut Fail, &[], json)
                    .unwrap_err()
                    .to_string()
                    .contains("output unavailable")
            );
        }
    }
}
