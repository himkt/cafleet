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
    text: &dyn Fn(&[ScanEntry]) -> String,
) -> Result<(), CafleetError> {
    let rendered = if json {
        output::format_json(&presentation::scan_json(entries))
    } else {
        text(entries)
    };
    writeln!(out, "{rendered}").map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))
}
