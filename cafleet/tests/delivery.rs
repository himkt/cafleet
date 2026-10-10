use std::cell::RefCell;
use std::collections::HashMap;

use cafleet::broker::{self, NewPlacement};
use cafleet::config::Settings;
use cafleet::delivery::{HoldReason, Item, PaneIo, PaneOutcome, deliver_pane};
use cafleet::time::format_utc;
use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::Connection;
use tempfile::TempDir;

const MEMBER_PANE: &str = "%2";
const HOLD_TIMEOUT: i64 = 300;
const DELIVERY_CAPTURE_LINES: i64 = 40;
const KEYSTROKE_SPACING_SECONDS: i64 = 3;
const EXEC_RESUME_GRACE_SECONDS: i64 = 10;

const FINISHED: &str = "⏺ Done.\n\n> ";
const AWAITING_USER: &str = " Do you want to proceed?\n ❯ 1. Yes\n   2. No";
const WORKING: &str = "✻ Cogitating… (12s · esc to interrupt)\n\n> ";
const UNCLASSIFIED: &str = "⏺ Bash(cargo build)\n  ⎿  Compiling cafleet";

const RESUME_CLAUSE: &str = "[cafleet] Resume your work if something was still running.";
const BROKER_NOTE: &str = "[cafleet] This delivery's Escape dismissed a pending prompt in your pane; \
     that rejection did not come from the user. Re-issue the tool call if you still need it.";

#[derive(Debug, PartialEq)]
struct Preview {
    pane_id: String,
    message_id: i64,
    sender_id: i64,
    ts: String,
    text: String,
    note: Option<String>,
}

struct FakeIo {
    captures: RefCell<HashMap<String, Result<String, String>>>,
    preview_result: RefCell<Result<(), String>>,
    capture_calls: RefCell<Vec<(String, i64)>>,
    previews: RefCell<Vec<Preview>>,
    prompts: RefCell<Vec<(String, String)>>,
}

impl FakeIo {
    fn showing(content: &str) -> Self {
        let io = FakeIo {
            captures: RefCell::new(HashMap::new()),
            preview_result: RefCell::new(Ok(())),
            capture_calls: RefCell::new(Vec::new()),
            previews: RefCell::new(Vec::new()),
            prompts: RefCell::new(Vec::new()),
        };
        io.show(content);
        io
    }

    fn show(&self, content: &str) {
        self.captures
            .borrow_mut()
            .insert(MEMBER_PANE.to_string(), Ok(content.to_string()));
    }

    fn preview_ids(&self) -> Vec<i64> {
        self.previews
            .borrow()
            .iter()
            .map(|preview| preview.message_id)
            .collect()
    }
}

impl PaneIo for FakeIo {
    fn capture_pane(&self, pane_id: &str, lines: i64) -> Result<String, String> {
        self.capture_calls
            .borrow_mut()
            .push((pane_id.to_string(), lines));
        self.captures.borrow()[pane_id].clone()
    }

    fn send_inline_preview(
        &self,
        pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        self.previews.borrow_mut().push(Preview {
            pane_id: pane_id.to_string(),
            message_id,
            sender_id,
            ts: ts.to_string(),
            text: text.to_string(),
            note: note.map(str::to_string),
        });
        self.preview_result.borrow().clone()
    }

    fn send_prompt(&self, pane_id: &str, text: &str) -> Result<(), String> {
        self.prompts
            .borrow_mut()
            .push((pane_id.to_string(), text.to_string()));
        Ok(())
    }
}

struct Fixture {
    _dir: TempDir,
    conn: Connection,
    director_id: i64,
    member_id: i64,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let url = format!("sqlite:///{}", dir.path().join("delivery.db").display());
    let mut conn = cafleet::db::connect(&url).unwrap();
    cafleet::db::migrate_to_head(&mut conn).unwrap();
    let fleet = broker::create_fleet(
        &mut conn,
        Some("alpha"),
        "main",
        "@1",
        "%0",
        "claude",
        "tmux",
        "monitor",
        "Monitor member for this fleet",
        |_, _, _| Ok("%1".to_string()),
    )
    .unwrap();
    let fleet_id = fleet["fleet_id"].as_i64().unwrap();
    let director_id = fleet["director"]["member_id"].as_i64().unwrap();
    let member_id = broker::register_member(
        &mut conn,
        fleet_id,
        "worker",
        "test member",
        &[],
        Some(&NewPlacement {
            backend: "tmux".to_string(),
            mux_session: "main".to_string(),
            mux_window_id: "@1".to_string(),
            mux_pane_id: Some(MEMBER_PANE.to_string()),
            coding_agent: "claude".to_string(),
        }),
        false,
    )
    .unwrap()
    .member_id;
    Fixture {
        _dir: dir,
        conn,
        director_id,
        member_id,
    }
}

fn settings(hold_timeout: i64) -> Settings {
    Settings::from_lookup(|name| match name {
        "CAFLEET_DATABASE_URL" => Some("sqlite:///unused.db".to_string()),
        "CAFLEET_DELIVERY_HOLD_TIMEOUT" => Some(hold_timeout.to_string()),
        _ => None,
    })
    .unwrap()
}

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 9, 0, 0).unwrap()
}

fn send_at(fixture: &mut Fixture, text: &str, at: DateTime<Utc>) -> i64 {
    let message = broker::send_message(
        &mut fixture.conn,
        fixture.director_id,
        &fixture.member_id.to_string(),
        text,
    )
    .unwrap();
    fixture
        .conn
        .execute(
            "UPDATE messages SET created_at=?1, status_timestamp=?1 WHERE message_id=?2",
            rusqlite::params![format_utc(at), message.message_id],
        )
        .unwrap();
    message.message_id
}

fn notified_at(conn: &Connection, message_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT notified_at FROM messages WHERE message_id=?1",
        [message_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn forced_at(conn: &Connection, member_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT forced_at FROM member_placements WHERE member_id=?1",
        [member_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[derive(Default)]
struct ExecTimes {
    dispatched_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    exit_code: Option<i64>,
    resumed_at: Option<DateTime<Utc>>,
}

impl ExecTimes {
    /// An exec whose command ended at `finished_at` with `exit_code` and that
    /// still awaits its resume.
    fn finished(finished_at: DateTime<Utc>, exit_code: i64) -> Self {
        ExecTimes {
            dispatched_at: Some(finished_at - Duration::seconds(20)),
            started_at: Some(finished_at - Duration::seconds(19)),
            finished_at: Some(finished_at),
            exit_code: Some(exit_code),
            resumed_at: None,
        }
    }
}

fn insert_exec(fixture: &Fixture, created_at: DateTime<Utc>, times: ExecTimes) -> i64 {
    let pid = times.started_at.map(|_| i64::from(std::process::id()));
    fixture
        .conn
        .execute(
            "INSERT INTO member_execs (member_id, command, created_at, dispatched_at, \
             started_at, pid, finished_at, exit_code, resumed_at) \
             VALUES (?1, 'mise //cafleet:test', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                fixture.member_id,
                format_utc(created_at),
                times.dispatched_at.map(format_utc),
                times.started_at.map(format_utc),
                pid,
                times.finished_at.map(format_utc),
                times.exit_code,
                times.resumed_at.map(format_utc),
            ],
        )
        .unwrap();
    fixture.conn.last_insert_rowid()
}

fn resumed_at(conn: &Connection, exec_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT resumed_at FROM member_execs WHERE exec_id=?1",
        [exec_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn resume_line(exec_id: i64, exit_code: i64) -> (String, String) {
    (
        MEMBER_PANE.to_string(),
        format!(
            "[cafleet] exec {exec_id} finished with exit {exit_code}. \
             Continue your work using its output above."
        ),
    )
}

fn describe(outcome: &PaneOutcome) -> String {
    match outcome {
        PaneOutcome::Fired { item, forced } => {
            let item = match item {
                Item::Preview { message_id } => format!("preview {message_id}"),
                Item::ExecDispatch { exec_id } => format!("exec dispatch {exec_id}"),
                Item::ExecResume { exec_id } => format!("exec resume {exec_id}"),
            };
            format!("fired {item} forced={forced}")
        }
        PaneOutcome::Held(reason) => match reason {
            HoldReason::ExecRunning => "held exec_running".to_string(),
            HoldReason::ResumeGrace => "held resume_grace".to_string(),
            HoldReason::AwaitingUser => "held awaiting_user".to_string(),
            HoldReason::Working => "held working".to_string(),
            HoldReason::Unclassified => "held unclassified".to_string(),
            HoldReason::CaptureFailed(error) => format!("held capture_failed: {error}"),
            HoldReason::Busy => "held busy".to_string(),
            HoldReason::KeystrokeFailed(error) => format!("held keystroke_failed: {error}"),
        },
        PaneOutcome::Idle => "idle".to_string(),
    }
}

fn deliver(fixture: &mut Fixture, io: &FakeIo, hold_timeout: i64, now: DateTime<Utc>) -> String {
    let outcome = deliver_pane(
        &mut fixture.conn,
        io,
        &settings(hold_timeout),
        fixture.member_id,
        now,
    )
    .unwrap();
    describe(&outcome)
}

fn fired(message_id: i64, forced: bool) -> String {
    format!("fired preview {message_id} forced={forced}")
}

#[test]
fn ordinary_delivery_fires_the_base_payload_into_a_finished_pane() {
    let mut fixture = fixture();
    let sent = base_time();
    let message_id = send_at(&mut fixture, "task one", sent);
    let io = FakeIo::showing(FINISHED);

    let outcome = deliver(&mut fixture, &io, HOLD_TIMEOUT, sent + Duration::seconds(1));
    assert_eq!(outcome, fired(message_id, false));
    assert_eq!(
        *io.capture_calls.borrow(),
        [(MEMBER_PANE.to_string(), DELIVERY_CAPTURE_LINES)]
    );
    assert_eq!(
        *io.previews.borrow(),
        [Preview {
            pane_id: MEMBER_PANE.to_string(),
            message_id,
            sender_id: fixture.director_id,
            ts: format_utc(sent),
            text: "task one".to_string(),
            note: None,
        }]
    );
    assert!(notified_at(&fixture.conn, message_id).is_some());
    assert_eq!(forced_at(&fixture.conn, fixture.member_id), None);
}

#[test]
fn a_pane_that_is_not_at_rest_holds_the_preview_below_the_timeout() {
    for (content, held) in [
        (AWAITING_USER, "held awaiting_user"),
        (WORKING, "held working"),
        (UNCLASSIFIED, "held unclassified"),
    ] {
        let mut fixture = fixture();
        let sent = base_time();
        let message_id = send_at(&mut fixture, "task one", sent);
        let io = FakeIo::showing(content);

        let outcome = deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            sent + Duration::seconds(HOLD_TIMEOUT - 1),
        );
        assert_eq!(outcome, held);
        assert!(io.previews.borrow().is_empty(), "{held} sends no keystroke");
        assert_eq!(notified_at(&fixture.conn, message_id), None);
    }
}

#[test]
fn a_hold_at_the_timeout_is_forced_with_the_payload_for_the_observed_state() {
    for (content, note) in [
        (AWAITING_USER, Some(BROKER_NOTE)),
        (WORKING, Some(RESUME_CLAUSE)),
        (UNCLASSIFIED, None),
    ] {
        let mut fixture = fixture();
        let sent = base_time();
        let message_id = send_at(&mut fixture, "task one", sent);
        let io = FakeIo::showing(content);

        let outcome = deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            sent + Duration::seconds(HOLD_TIMEOUT),
        );
        assert_eq!(outcome, fired(message_id, true), "{content}");
        let previews = io.previews.borrow();
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].text, "task one");
        assert_eq!(previews[0].note.as_deref(), note, "{content}");
        assert!(notified_at(&fixture.conn, message_id).is_some());
        assert!(forced_at(&fixture.conn, fixture.member_id).is_some());
    }
}

#[test]
fn a_finished_pane_past_the_timeout_still_gets_an_ordinary_delivery() {
    let mut fixture = fixture();
    let sent = base_time();
    let message_id = send_at(&mut fixture, "task one", sent);
    let io = FakeIo::showing(FINISHED);

    let outcome = deliver(
        &mut fixture,
        &io,
        HOLD_TIMEOUT,
        sent + Duration::seconds(HOLD_TIMEOUT * 2),
    );
    assert_eq!(outcome, fired(message_id, false));
    assert_eq!(io.previews.borrow()[0].note, None);
    assert_eq!(forced_at(&fixture.conn, fixture.member_id), None);
}

#[test]
fn a_zero_timeout_never_forces() {
    for (content, held) in [
        (AWAITING_USER, "held awaiting_user"),
        (WORKING, "held working"),
        (UNCLASSIFIED, "held unclassified"),
    ] {
        let mut fixture = fixture();
        let sent = base_time();
        send_at(&mut fixture, "task one", sent);
        let io = FakeIo::showing(content);

        let outcome = deliver(&mut fixture, &io, 0, sent + Duration::days(30));
        assert_eq!(outcome, held);
        assert!(io.previews.borrow().is_empty());
    }
}

#[test]
fn overdue_previews_on_an_awaiting_pane_force_one_keystroke_per_timeout() {
    let mut fixture = fixture();
    let sent = base_time();
    let ids = [
        send_at(&mut fixture, "one", sent),
        send_at(&mut fixture, "two", sent + Duration::seconds(1)),
        send_at(&mut fixture, "three", sent + Duration::seconds(2)),
    ];
    let io = FakeIo::showing(AWAITING_USER);
    let at = |seconds: i64| sent + Duration::seconds(seconds);

    assert_eq!(
        deliver(&mut fixture, &io, HOLD_TIMEOUT, at(HOLD_TIMEOUT)),
        fired(ids[0], true)
    );
    for (first_forced, next) in [(HOLD_TIMEOUT, ids[1]), (HOLD_TIMEOUT * 2, ids[2])] {
        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                at(first_forced + KEYSTROKE_SPACING_SECONDS)
            ),
            "held awaiting_user",
            "the hold clock restarts at the pane's last forced delivery"
        );
        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                at(first_forced + HOLD_TIMEOUT - 1)
            ),
            "held awaiting_user"
        );
        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                at(first_forced + HOLD_TIMEOUT)
            ),
            fired(next, true)
        );
    }
    assert_eq!(io.preview_ids(), ids);
    assert!(
        io.previews
            .borrow()
            .iter()
            .all(|preview| preview.note.as_deref() == Some(BROKER_NOTE))
    );
}

#[test]
fn the_oldest_pending_preview_fires_first() {
    let mut fixture = fixture();
    let sent = base_time();
    let older = send_at(&mut fixture, "older", sent);
    let newer = send_at(&mut fixture, "newer", sent + Duration::seconds(5));
    let io = FakeIo::showing(FINISHED);

    let first = sent + Duration::seconds(10);
    assert_eq!(
        deliver(&mut fixture, &io, HOLD_TIMEOUT, first),
        fired(older, false)
    );
    assert_eq!(notified_at(&fixture.conn, newer), None);
    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            first + Duration::seconds(KEYSTROKE_SPACING_SECONDS)
        ),
        fired(newer, false)
    );
    assert_eq!(io.preview_ids(), [older, newer]);
}

#[test]
fn a_failed_keystroke_clears_the_stamp_and_leaves_the_preview_owed() {
    let mut fixture = fixture();
    let sent = base_time();
    let message_id = send_at(&mut fixture, "task one", sent);
    let io = FakeIo::showing(FINISHED);
    *io.preview_result.borrow_mut() = Err("send-keys failed: boom".to_string());

    let first = sent + Duration::seconds(1);
    assert_eq!(
        deliver(&mut fixture, &io, HOLD_TIMEOUT, first),
        "held keystroke_failed: send-keys failed: boom"
    );
    assert_eq!(notified_at(&fixture.conn, message_id), None);

    *io.preview_result.borrow_mut() = Ok(());
    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            first + Duration::seconds(KEYSTROKE_SPACING_SECONDS)
        ),
        fired(message_id, false)
    );
    assert!(notified_at(&fixture.conn, message_id).is_some());
}

#[test]
fn an_acked_message_is_never_keystroked() {
    let mut fixture = fixture();
    let sent = base_time();
    let message_id = send_at(&mut fixture, "task one", sent);
    broker::ack_message(&mut fixture.conn, message_id).unwrap();
    let io = FakeIo::showing(FINISHED);

    for seconds in [1, HOLD_TIMEOUT, HOLD_TIMEOUT * 10] {
        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                sent + Duration::seconds(seconds)
            ),
            "idle"
        );
    }
    assert!(io.capture_calls.borrow().is_empty());
    assert!(io.previews.borrow().is_empty());
    assert_eq!(notified_at(&fixture.conn, message_id), None);
}

#[test]
fn a_second_delivery_inside_the_spacing_window_sends_no_second_keystroke() {
    let mut fixture = fixture();
    let sent = base_time();
    let first_id = send_at(&mut fixture, "one", sent);
    let second_id = send_at(&mut fixture, "two", sent + Duration::seconds(1));
    let io = FakeIo::showing(FINISHED);

    let first = sent + Duration::seconds(10);
    assert_eq!(
        deliver(&mut fixture, &io, HOLD_TIMEOUT, first),
        fired(first_id, false)
    );
    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            first + Duration::seconds(KEYSTROKE_SPACING_SECONDS - 1)
        ),
        "held busy"
    );
    assert_eq!(io.preview_ids(), [first_id]);
    assert_eq!(notified_at(&fixture.conn, second_id), None);

    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            first + Duration::seconds(KEYSTROKE_SPACING_SECONDS)
        ),
        fired(second_id, false)
    );
    assert_eq!(io.preview_ids(), [first_id, second_id]);
}

#[test]
fn an_exec_in_flight_holds_every_item_without_forcing() {
    let sent = base_time();
    let dispatched_at = Some(sent + Duration::seconds(1));
    for started_at in [None, Some(sent + Duration::seconds(2))] {
        for content in [FINISHED, AWAITING_USER, WORKING] {
            let mut fixture = fixture();
            let message_id = send_at(&mut fixture, "task one", sent);
            insert_exec(
                &fixture,
                sent,
                ExecTimes {
                    dispatched_at,
                    started_at,
                    ..ExecTimes::default()
                },
            );
            let io = FakeIo::showing(content);

            for seconds in [HOLD_TIMEOUT - 1, HOLD_TIMEOUT, HOLD_TIMEOUT * 10] {
                assert_eq!(
                    deliver(
                        &mut fixture,
                        &io,
                        HOLD_TIMEOUT,
                        sent + Duration::seconds(seconds)
                    ),
                    "held exec_running",
                    "{content}"
                );
            }
            assert!(io.previews.borrow().is_empty());
            assert!(io.prompts.borrow().is_empty());
            assert_eq!(notified_at(&fixture.conn, message_id), None);
        }
    }
}

#[test]
fn the_hold_age_restarts_at_the_finished_at_of_the_panes_latest_exec() {
    let mut fixture = fixture();
    let sent = base_time();
    let message_id = send_at(&mut fixture, "queued behind the command", sent);
    let exec_end = sent + Duration::seconds(HOLD_TIMEOUT * 4);
    insert_exec(
        &fixture,
        sent,
        ExecTimes {
            resumed_at: Some(exec_end),
            ..ExecTimes::finished(exec_end, 0)
        },
    );
    let io = FakeIo::showing(WORKING);

    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            exec_end + Duration::seconds(HOLD_TIMEOUT - 1)
        ),
        "held working",
        "a long command does not force the keystroke that waited for it"
    );
    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            exec_end + Duration::seconds(HOLD_TIMEOUT)
        ),
        fired(message_id, true)
    );
    assert_eq!(io.previews.borrow()[0].note.as_deref(), Some(RESUME_CLAUSE));
}

#[test]
fn a_finished_exec_on_a_working_pane_closes_without_a_keystroke() {
    for seconds in [1, EXEC_RESUME_GRACE_SECONDS, HOLD_TIMEOUT] {
        let mut fixture = fixture();
        let finished_at = base_time();
        let exec_id = insert_exec(
            &fixture,
            finished_at - Duration::seconds(30),
            ExecTimes::finished(finished_at, 0),
        );
        let io = FakeIo::showing(WORKING);

        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                finished_at + Duration::seconds(seconds),
            ),
            "held working"
        );
        assert!(
            resumed_at(&fixture.conn, exec_id).is_some(),
            "the harness answered the command's output itself ({seconds} s)"
        );
        assert!(io.prompts.borrow().is_empty());
        assert!(io.previews.borrow().is_empty());
    }
}

#[test]
fn a_finished_exec_holds_for_the_resume_grace_while_the_pane_is_not_working() {
    for content in [FINISHED, AWAITING_USER, UNCLASSIFIED] {
        let mut fixture = fixture();
        let finished_at = base_time();
        let exec_id = insert_exec(
            &fixture,
            finished_at - Duration::seconds(30),
            ExecTimes::finished(finished_at, 0),
        );
        let io = FakeIo::showing(content);

        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                finished_at + Duration::seconds(EXEC_RESUME_GRACE_SECONDS - 1)
            ),
            "held resume_grace",
            "{content}"
        );
        assert_eq!(resumed_at(&fixture.conn, exec_id), None);
        assert!(io.prompts.borrow().is_empty());
    }
}

#[test]
fn a_finished_exec_past_the_grace_gets_the_resume_line_in_a_pane_at_rest() {
    let mut fixture = fixture();
    let finished_at = base_time();
    let exec_id = insert_exec(
        &fixture,
        finished_at - Duration::seconds(30),
        ExecTimes::finished(finished_at, 7),
    );
    let io = FakeIo::showing(FINISHED);

    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            finished_at + Duration::seconds(EXEC_RESUME_GRACE_SECONDS)
        ),
        format!("fired exec resume {exec_id} forced=false")
    );
    assert_eq!(*io.prompts.borrow(), [resume_line(exec_id, 7)]);
    assert!(resumed_at(&fixture.conn, exec_id).is_some());
    assert_eq!(forced_at(&fixture.conn, fixture.member_id), None);
}

#[test]
fn a_resume_line_past_the_grace_holds_and_forces_through_the_gate_table() {
    let mut fixture = fixture();
    let finished_at = base_time();
    let exec_id = insert_exec(
        &fixture,
        finished_at - Duration::seconds(30),
        ExecTimes::finished(finished_at, 0),
    );
    let io = FakeIo::showing(AWAITING_USER);

    for seconds in [EXEC_RESUME_GRACE_SECONDS, HOLD_TIMEOUT - 1] {
        assert_eq!(
            deliver(
                &mut fixture,
                &io,
                HOLD_TIMEOUT,
                finished_at + Duration::seconds(seconds)
            ),
            "held awaiting_user"
        );
    }
    assert_eq!(resumed_at(&fixture.conn, exec_id), None);
    assert!(io.prompts.borrow().is_empty());

    assert_eq!(
        deliver(
            &mut fixture,
            &io,
            HOLD_TIMEOUT,
            finished_at + Duration::seconds(HOLD_TIMEOUT)
        ),
        format!("fired exec resume {exec_id} forced=true")
    );
    assert_eq!(*io.prompts.borrow(), [resume_line(exec_id, 0)]);
    assert!(resumed_at(&fixture.conn, exec_id).is_some());
    assert!(forced_at(&fixture.conn, fixture.member_id).is_some());
}
