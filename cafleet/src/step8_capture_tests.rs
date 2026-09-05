//! Step 8 approved-API contracts. Phase B connects this module from lib.rs.
use crate::{
    broker::{self, test_support},
    capture::{CaptureSnapshot, MemberCapture, ScanEntry, write_member_capture, write_scan},
    cli::{member::capture_with_dependencies, monitor::scan_with_dependencies},
    multiplexer::{
        RunError, TmuxMultiplexer,
        test_support::{FakeRunner, env},
    },
    presentation,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::{cell::Cell, io, rc::Rc};

const WHEN: &str = "2026-09-06T01:02:03.000007+00:00";
const PLAIN_HASH: &str = "526a35d455e79594452b17d2378d09d811ce0fb0ece884ac1198c3fc619c10f4";
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(WHEN)
        .unwrap()
        .with_timezone(&Utc)
}
fn fixture() -> (tempfile::TempDir, rusqlite::Connection, i64) {
    let dir = tempfile::Builder::new()
        .prefix(".step8-capture-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let mut conn = test_support::migrated_conn(&dir);
    let (fleet, _) = test_support::create_fleet(&mut conn, "capture");
    (dir, conn, fleet)
}
fn mux(runner: Rc<FakeRunner>) -> TmuxMultiplexer {
    TmuxMultiplexer::new(runner, env(&[("TMUX", "fixture")]))
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
fn scan_presenters_keep_error_nulls_and_exact_text_and_skip_text_builder_for_json() {
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
    write_scan(&mut out, &entries, true, &|_| {
        panic!("JSON must not construct headings")
    })
    .unwrap();
    assert_eq!(out, format!("{expected}\n").into_bytes());
    out.clear();
    let calls = Cell::new(0);
    write_scan(&mut out, &entries, false, &|rows| {
        calls.set(calls.get() + 1);
        presentation::scan_text(rows)
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(out, format!("{expected_text}\n").into_bytes());
}
#[test]
fn capture_stamps_once_after_success_and_never_after_pending_or_failed_read() {
    let (_dir, conn, _) = fixture();
    let runner = FakeRunner::with_binary("tmux");
    runner.respond(Ok("雪\n緑".into()));
    let calls = Cell::new(0);
    let stamp = || {
        assert_eq!(runner.run_argvs().len(), 1);
        calls.set(calls.get() + 1);
        now()
    };
    let result =
        capture_with_dependencies(&conn, 1, 12, false, || Ok(mux(runner.clone())), &stamp).unwrap();
    assert_eq!(result.member_id, 1);
    assert_eq!(result.pane_id, "%0");
    assert_eq!(result.lines, 12);
    assert_eq!(result.snapshot.content_sha256, PLAIN_HASH);
    assert_eq!(calls.get(), 1);
    runner.respond(Err(RunError::Failed {
        stderr: "pane read broke".into(),
    }));
    assert!(
        capture_with_dependencies(&conn, 1, 12, false, || Ok(mux(runner.clone())), &|| panic!(
            "failed read has no timestamp"
        ))
        .is_err()
    );
    conn.execute(
        "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=1",
        [],
    )
    .unwrap();
    assert!(
        capture_with_dependencies(&conn, 1, 12, false, || Ok(mux(runner.clone())), &|| panic!(
            "pending has no timestamp"
        ))
        .is_err()
    );
    assert_eq!(runner.run_argvs().len(), 2);
}
#[test]
fn scan_uses_real_roster_continues_after_errors_and_stamps_only_each_success() {
    let (_dir, mut conn, fleet) = fixture();
    let pending = test_support::register(&mut conn, fleet, "pending", None);
    let worker = test_support::register(&mut conn, fleet, "worker", Some("%8"));
    let absent = test_support::register(&mut conn, fleet, "without-placement", Some("%6"));
    conn.execute("DELETE FROM member_placements WHERE member_id=?1", [absent])
        .unwrap();
    let runner = FakeRunner::with_binary("tmux");
    runner.respond(Ok("雪\n緑".into()));
    runner.respond(Err(RunError::Failed {
        stderr: "broken monitor pane".into(),
    }));
    runner.respond(Ok("worker result".into()));
    let calls = Cell::new(0);
    let rows = scan_with_dependencies(&conn, fleet, 12, false, || Ok(mux(runner.clone())), &|| {
        let index = calls.get();
        assert_eq!(runner.run_argvs().len(), if index == 0 { 1 } else { 3 });
        calls.set(index + 1);
        now() + chrono::Duration::seconds(index)
    })
    .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.member_id).collect::<Vec<_>>(),
        [1, 2, pending, worker]
    );
    assert!(rows.iter().all(|r| r.lines == 12));
    assert_eq!(rows[0].outcome.as_ref().unwrap().captured_at, WHEN);
    assert!(
        rows[1]
            .outcome
            .as_ref()
            .unwrap_err()
            .starts_with("capture failed:")
    );
    assert_eq!(
        rows[2].outcome.as_ref().unwrap_err(),
        "pane not available (pending placement)"
    );
    assert_eq!(
        rows[3].outcome.as_ref().unwrap().captured_at,
        "2026-09-06T01:02:04.000007+00:00"
    );
    assert_eq!(calls.get(), 2);
    assert_eq!(
        runner
            .run_argvs()
            .iter()
            .map(|a| a[4].as_str())
            .collect::<Vec<_>>(),
        ["%0", "%1", "%8"]
    );
}
#[test]
fn capture_and_scan_dependency_guards_preserve_opposite_orders() {
    let (_dir, conn, _) = fixture();
    let runner = FakeRunner::without_binaries();
    let error =
        capture_with_dependencies(&conn, 999, 12, false, || Ok(mux(runner.clone())), &|| {
            panic!("guard failed")
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("tmux binary not found"));
    let error = scan_with_dependencies::<TmuxMultiplexer>(
        &conn,
        999,
        12,
        false,
        || panic!("fleet check comes first"),
        &|| panic!("guard failed"),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("fleet 999 not found"));
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
            write_scan(&mut Fail, &[], json, &presentation::scan_text)
                .unwrap_err()
                .to_string()
                .contains("output unavailable")
        );
    }
}
