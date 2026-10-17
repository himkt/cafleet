use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration as StdDuration, Instant};

use cafleet::broker::{self, NewPlacement};
use cafleet::config::Settings;
use cafleet::delivery::{
    DELIVERY_CAPTURE_LINES, EXEC_RESUME_GRACE_SECONDS, HoldReason, Item, PaneIo, PaneOutcome,
    PingOutcome, claim_and_recheck, deliver_pane, observe_pane, ping_pane,
};
use cafleet::monitor::{MonitorMux, TickResult, monitor_tick};
use cafleet::multiplexer::{MultiplexerError, WakeEntry};
use cafleet::time::{Clock, format_utc};
use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::{Connection, TransactionBehavior, params};
use tempfile::TempDir;

const DIRECTOR_PANE: &str = "%0";
const MONITOR_PANE: &str = "%1";
const MEMBER_PANE: &str = "%2";

#[derive(Clone)]
struct FakeClock {
    utc: Rc<Cell<DateTime<Utc>>>,
    monotonic: Rc<Cell<Instant>>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            utc: Rc::new(Cell::new(
                Utc.with_ymd_and_hms(2026, 10, 10, 9, 0, 0).unwrap(),
            )),
            monotonic: Rc::new(Cell::new(Instant::now())),
        }
    }

    fn advance(&self, milliseconds: i64) {
        assert!(milliseconds >= 0);
        self.utc
            .set(self.utc.get() + Duration::milliseconds(milliseconds));
        self.monotonic
            .set(self.monotonic.get() + StdDuration::from_millis(milliseconds.try_into().unwrap()));
    }
}

impl Clock for FakeClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.utc.get()
    }
    fn monotonic_now(&self) -> Instant {
        self.monotonic.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Capture(String),
    Preview(String, i64, String),
    AutomaticPrompt(String, String),
    Trigger(String, String),
}

type CaptureHook = Box<dyn FnMut(&str, usize)>;

struct FakeIo {
    captures: RefCell<HashMap<String, VecDeque<Result<String, String>>>>,
    capture_counts: RefCell<HashMap<String, usize>>,
    capture_hook: RefCell<Option<CaptureHook>>,
    send_hook: RefCell<Option<Box<dyn FnMut()>>>,
    events: RefCell<Vec<Event>>,
    send_result: RefCell<Result<(), String>>,
    trigger_ok: Cell<bool>,
}

impl FakeIo {
    fn new() -> Self {
        Self {
            captures: RefCell::new(HashMap::new()),
            capture_counts: RefCell::new(HashMap::new()),
            capture_hook: RefCell::new(None),
            send_hook: RefCell::new(None),
            events: RefCell::new(Vec::new()),
            send_result: RefCell::new(Ok(())),
            trigger_ok: Cell::new(true),
        }
    }

    fn queue(&self, pane: &str, captures: Vec<Result<String, String>>) {
        assert!(
            self.captures
                .borrow_mut()
                .insert(pane.to_string(), captures.into())
                .is_none()
        );
    }

    fn safe(&self, pane: &str, count: usize) {
        self.queue(pane, vec![Ok(real_capture("empty")); count]);
    }

    fn sent(&self) -> Vec<Event> {
        self.events
            .borrow()
            .iter()
            .filter(|event| !matches!(event, Event::Capture(_)))
            .cloned()
            .collect()
    }

    fn captures_for(&self, pane: &str) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|event| **event == Event::Capture(pane.to_string()))
            .count()
    }

    fn before_send(&self) {
        if let Some(hook) = self.send_hook.borrow_mut().as_mut() {
            hook();
        }
    }
}

impl PaneIo for FakeIo {
    fn capture_pane(&self, pane: &str, lines: i64) -> Result<String, String> {
        assert_eq!(lines, DELIVERY_CAPTURE_LINES);
        self.events
            .borrow_mut()
            .push(Event::Capture(pane.to_string()));
        let ordinal = {
            let mut counts = self.capture_counts.borrow_mut();
            let count = counts.entry(pane.to_string()).or_default();
            *count += 1;
            *count
        };
        if let Some(hook) = self.capture_hook.borrow_mut().as_mut() {
            hook(pane, ordinal);
        }
        self.captures
            .borrow_mut()
            .get_mut(pane)
            .expect("the test configures every captured pane")
            .pop_front()
            .expect("every observation consumes a configured capture result")
    }

    fn send_inline_preview(
        &self,
        pane: &str,
        id: i64,
        _: i64,
        _: &str,
        text: &str,
    ) -> Result<(), String> {
        self.before_send();
        self.events
            .borrow_mut()
            .push(Event::Preview(pane.to_string(), id, text.to_string()));
        self.send_result.borrow().clone()
    }

    fn send_automatic_prompt(&self, pane: &str, text: &str) -> Result<(), String> {
        self.before_send();
        self.events
            .borrow_mut()
            .push(Event::AutomaticPrompt(pane.to_string(), text.to_string()));
        self.send_result.borrow().clone()
    }

    fn send_automatic_trigger(&self, pane: &str, payload: &str) -> bool {
        self.before_send();
        self.events
            .borrow_mut()
            .push(Event::Trigger(pane.to_string(), payload.to_string()));
        self.trigger_ok.get()
    }
}

impl MonitorMux for FakeIo {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
        Ok([DIRECTOR_PANE, MONITOR_PANE, MEMBER_PANE]
            .map(str::to_string)
            .into())
    }

    fn send_wake_entries(
        &self,
        _: &str,
        _: i64,
        _: &[WakeEntry<'_>],
        _: &WakeEntry<'_>,
    ) -> Result<bool, MultiplexerError> {
        panic!("monitor safety orchestration submits its prepared automatic trigger")
    }
}

fn real_capture(state: &str) -> String {
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/pane_state/claude-herdr-runtime.json"
    ))
    .unwrap();
    let content = fixtures
        .iter()
        .find(|entry| entry["state"] == state)
        .expect("the observed Claude state is recorded")["content"]
        .as_str()
        .expect("the capture contains text");
    let rows = content.lines().collect::<Vec<_>>();
    let bound = usize::try_from(DELIVERY_CAPTURE_LINES).expect("capture line bound is positive");
    rows[rows.len().saturating_sub(bound)..].join("\n")
}

fn unsafe_observations() -> Vec<(Result<String, String>, HoldReason)> {
    vec![
        (Ok(real_capture("draft-single")), HoldReason::Draft),
        (Ok(real_capture("draft-multiline")), HoldReason::Draft),
        (Ok(real_capture("draft-blank-first")), HoldReason::Draft),
        (Ok(real_capture("decision-menu")), HoldReason::AwaitingUser),
        (
            Ok("✻ Cogitating… (12s · esc to interrupt)".to_string()),
            HoldReason::Working,
        ),
        (
            Ok("An unsupported or truncated current interface".to_string()),
            HoldReason::Unclassified,
        ),
        (
            Err("capture subprocess failed: permission denied".to_string()),
            HoldReason::CaptureFailed("capture subprocess failed: permission denied".to_string()),
        ),
    ]
}

fn synthesized_blank_row_composer(agent: &str, extra_rows: usize) -> (String, String) {
    assert!(extra_rows > 0);
    let source = match agent {
        "claude" => include_str!("fixtures/pane_state/claude-herdr-runtime.json"),
        "opencode" => include_str!("fixtures/pane_state/opencode-unobstructed.json"),
        other => panic!("unsupported blank-row fixture agent: {other}"),
    };
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(source).unwrap();
    let empty = fixtures
        .iter()
        .find(|entry| entry["state"] == "empty")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    let bound = usize::try_from(DELIVERY_CAPTURE_LINES).unwrap();
    let bounded = |rows: &[String]| rows[rows.len().saturating_sub(bound)..].join("\n");
    let mut rows: Vec<String> = empty.lines().map(str::to_string).collect();
    let baseline = bounded(&rows);
    assert_eq!(
        cafleet::pane_state::classify(agent, &baseline),
        cafleet::pane_state::PaneState::Finished
    );
    let (insertion, blank) = match agent {
        "claude" => (rows.iter().rposition(|row| row == "❯").unwrap() + 1, ""),
        "opencode" => {
            let model = rows
                .iter()
                .rposition(|row| row.starts_with("  ┃  ") && row.ends_with(" OpenCode"))
                .unwrap();
            assert_eq!(&rows[model - 3..model], &["  ┃", "  ┃", "  ┃"]);
            (model, "  ┃")
        }
        other => panic!("unsupported blank-row fixture agent: {other}"),
    };
    rows.splice(
        insertion..insertion,
        std::iter::repeat_n(blank.to_string(), extra_rows),
    );
    (baseline, bounded(&rows))
}

fn blank_row_observations(
    baseline: String,
    draft: String,
    final_observation: bool,
) -> Vec<Result<String, String>> {
    if final_observation {
        vec![Ok(baseline), Ok(draft)]
    } else {
        vec![Ok(draft.clone()), Ok(draft)]
    }
}

fn set_target_agent(f: &Fixture, member_id: i64, agent: &str) {
    assert_eq!(
        f.conn
            .execute(
                "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                params![agent, member_id]
            )
            .unwrap(),
        1
    );
    assert_eq!(f.target(member_id).coding_agent, agent);
}

fn check_blank_row_owed_items(agent: &str, final_observation: bool) {
    for kind in KINDS {
        for extra_rows in [1, 2, 5] {
            let (baseline, draft) = synthesized_blank_row_composer(agent, extra_rows);
            let mut f = Fixture::new();
            set_target_agent(&f, f.member_id, agent);
            let item = f.item(kind);
            let io = FakeIo::new();
            io.queue(
                MEMBER_PANE,
                blank_row_observations(baseline, draft, final_observation),
            );
            held(deliver(&mut f, &io), HoldReason::Draft);
            assert_no_send(&io, item, &f.conn);
            assert_eq!(
                io.captures_for(MEMBER_PANE),
                if final_observation { 2 } else { 1 }
            );
            assert_eq!(
                claim_stamp(&f.conn, f.member_id).is_some(),
                final_observation
            );
        }
    }
}

fn check_blank_row_ping(agent: &str, final_observation: bool) {
    for extra_rows in [1, 2, 5] {
        let (baseline, draft) = synthesized_blank_row_composer(agent, extra_rows);
        let mut f = Fixture::new();
        set_target_agent(&f, f.member_id, agent);
        let item = f.item(Kind::Preview);
        let io = FakeIo::new();
        io.queue(
            MEMBER_PANE,
            blank_row_observations(baseline, draft, final_observation),
        );
        ping_held(ping(&mut f, &io).unwrap(), HoldReason::Draft);
        assert_no_send(&io, item, &f.conn);
        assert_eq!(
            io.captures_for(MEMBER_PANE),
            if final_observation { 2 } else { 1 }
        );
        assert_eq!(
            claim_stamp(&f.conn, f.member_id).is_some(),
            final_observation
        );
    }
}

fn check_blank_row_wakes(agent: &str, final_observation: bool) {
    for requested in [false, true] {
        for extra_rows in [1, 2, 5] {
            let (baseline, draft) = synthesized_blank_row_composer(agent, extra_rows);
            let mut f = Fixture::new();
            set_target_agent(&f, f.monitor_id, agent);
            f.start_monitor(requested);
            let before = wake_stamps(&f);
            let io = FakeIo::new();
            io.queue(
                MONITOR_PANE,
                blank_row_observations(baseline, draft, final_observation),
            );
            f.tick(&io);
            assert!(io.sent().is_empty());
            assert_eq!(wake_stamps(&f), before);
            assert_eq!(
                io.captures_for(MONITOR_PANE),
                if final_observation { 2 } else { 1 }
            );
            assert_eq!(
                claim_stamp(&f.conn, f.monitor_id).is_some(),
                final_observation
            );
            assert_eq!(
                broker::read_monitor_runtime(&f.conn, f.fleet_id)
                    .unwrap()
                    .unwrap()
                    .last_tick_at,
                Some(format_utc(f.clock.utc_now()))
            );
        }
    }
}

#[test]
fn claude_blank_rows_hold_every_owed_kind_at_initial_observation() {
    check_blank_row_owed_items("claude", false);
}

#[test]
fn claude_blank_rows_hold_every_owed_kind_at_final_observation() {
    check_blank_row_owed_items("claude", true);
}

#[test]
fn opencode_blank_rows_hold_every_owed_kind_at_initial_observation() {
    check_blank_row_owed_items("opencode", false);
}

#[test]
fn opencode_blank_rows_hold_every_owed_kind_at_final_observation() {
    check_blank_row_owed_items("opencode", true);
}

#[test]
fn claude_blank_rows_hold_ping_at_initial_observation() {
    check_blank_row_ping("claude", false);
}

#[test]
fn claude_blank_rows_hold_ping_at_final_observation() {
    check_blank_row_ping("claude", true);
}

#[test]
fn opencode_blank_rows_hold_ping_at_initial_observation() {
    check_blank_row_ping("opencode", false);
}

#[test]
fn opencode_blank_rows_hold_ping_at_final_observation() {
    check_blank_row_ping("opencode", true);
}

#[test]
fn claude_blank_rows_hold_scheduled_and_requested_wakes_at_initial_observation() {
    check_blank_row_wakes("claude", false);
}

#[test]
fn claude_blank_rows_hold_scheduled_and_requested_wakes_at_final_observation() {
    check_blank_row_wakes("claude", true);
}

#[test]
fn opencode_blank_rows_hold_scheduled_and_requested_wakes_at_initial_observation() {
    check_blank_row_wakes("opencode", false);
}

#[test]
fn opencode_blank_rows_hold_scheduled_and_requested_wakes_at_final_observation() {
    check_blank_row_wakes("opencode", true);
}

fn settings() -> Settings {
    Settings::from_lookup(|name| {
        (name == "CAFLEET_DATABASE_URL").then(|| "sqlite:///unused.db".to_string())
    })
    .unwrap()
}

struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    conn: Connection,
    fleet_id: i64,
    director_id: i64,
    monitor_id: i64,
    member_id: i64,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("delivery.db");
        let mut conn = cafleet::db::connect(&format!("sqlite:///{}", path.display())).unwrap();
        cafleet::db::migrate_to_head(&mut conn).unwrap();
        let fleet = broker::create_fleet(
            &mut conn,
            Some("alpha"),
            "main",
            "@1",
            DIRECTOR_PANE,
            "claude",
            "tmux",
            "monitor",
            "Monitor member",
            |_, _, _| Ok(MONITOR_PANE.to_string()),
        )
        .unwrap();
        let fleet_id = fleet["fleet_id"].as_i64().unwrap();
        let director_id = fleet["director"]["member_id"].as_i64().unwrap();
        let monitor_id = broker::active_monitor_member_id(&conn, fleet_id)
            .unwrap()
            .unwrap();
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
        Self {
            _dir: dir,
            path,
            conn,
            fleet_id,
            director_id,
            monitor_id,
            member_id,
            clock: FakeClock::new(),
        }
    }

    fn other_connection(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }

    fn target(&self, id: i64) -> broker::records::PaneTarget {
        broker::pane_target(&self.conn, id)
            .unwrap()
            .expect("the fixture member has a live pane")
    }

    fn item(&mut self, kind: Kind) -> TestItem {
        self.item_for(kind, self.member_id)
    }

    fn item_for(&mut self, kind: Kind, member_id: i64) -> TestItem {
        let old = self.clock.utc_now() - Duration::days(3650);
        let id = match kind {
            Kind::Preview => {
                let sender = if member_id == self.director_id {
                    self.member_id
                } else {
                    self.director_id
                };
                assert_ne!(
                    sender, member_id,
                    "an owed preview comes from another member"
                );
                let message = broker::send_message(
                    &mut self.conn,
                    sender,
                    &member_id.to_string(),
                    "preserve this queued payload",
                )
                .unwrap();
                self.conn
                    .execute(
                        "UPDATE messages SET created_at=?1,status_timestamp=?1 WHERE message_id=?2",
                        params![format_utc(old), message.message_id],
                    )
                    .unwrap();
                message.message_id
            }
            Kind::Dispatch | Kind::Resume => {
                let finished = matches!(kind, Kind::Resume);
                let end = self.clock.utc_now() - Duration::seconds(EXEC_RESUME_GRACE_SECONDS + 1);
                self.conn.execute("INSERT INTO member_execs (member_id,command,created_at,dispatched_at,started_at,finished_at,exit_code) VALUES (?1,'mise //cafleet:test',?2,?3,?3,?4,?5)", params![member_id, format_utc(old), finished.then(|| format_utc(end - Duration::seconds(1))), finished.then(|| format_utc(end)), finished.then_some(7)]).unwrap();
                self.conn.last_insert_rowid()
            }
        };
        TestItem { kind, id }
    }

    fn start_monitor(&mut self, requested: bool) {
        assert!(
            broker::claim_monitor_runtime(
                &mut self.conn,
                self.fleet_id,
                i64::from(std::process::id()),
                Some(5),
                Some(600),
                &format_utc(self.clock.utc_now() - Duration::seconds(601))
            )
            .unwrap()
        );
        if requested {
            self.conn
                .execute(
                    "UPDATE monitor_runtime SET wake_requested_at=?1 WHERE fleet_id=?2",
                    params![format_utc(self.clock.utc_now()), self.fleet_id],
                )
                .unwrap();
        }
    }

    fn tick(&mut self, io: &FakeIo) -> String {
        let mut out = Vec::new();
        assert!(matches!(
            monitor_tick(
                &mut self.conn,
                io,
                &mut out,
                &settings(),
                self.fleet_id,
                i64::from(std::process::id()),
                &self.clock
            )
            .unwrap(),
            TickResult::Continue
        ));
        String::from_utf8(out).unwrap()
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Preview,
    Dispatch,
    Resume,
}
const KINDS: [Kind; 3] = [Kind::Preview, Kind::Dispatch, Kind::Resume];

#[derive(Clone, Copy)]
struct TestItem {
    kind: Kind,
    id: i64,
}

impl TestItem {
    fn stamp(&self, conn: &Connection) -> Option<String> {
        let sql = match self.kind {
            Kind::Preview => "SELECT notified_at FROM messages WHERE message_id=?1",
            Kind::Dispatch => "SELECT dispatched_at FROM member_execs WHERE exec_id=?1",
            Kind::Resume => "SELECT resumed_at FROM member_execs WHERE exec_id=?1",
        };
        conn.query_row(sql, [self.id], |row| row.get(0)).unwrap()
    }

    fn expected(&self) -> Item {
        match self.kind {
            Kind::Preview => Item::Preview {
                message_id: self.id,
            },
            Kind::Dispatch => Item::ExecDispatch { exec_id: self.id },
            Kind::Resume => Item::ExecResume { exec_id: self.id },
        }
    }
}

fn deliver(f: &mut Fixture, io: &FakeIo) -> PaneOutcome {
    deliver_pane(&mut f.conn, io, &settings(), f.member_id, &f.clock).unwrap()
}

fn held(outcome: PaneOutcome, reason: HoldReason) {
    assert!(
        matches!(outcome, PaneOutcome::Held(ref actual) if *actual == reason),
        "{outcome:?}, expected {reason:?}"
    );
}

fn fired(outcome: PaneOutcome, expected: Item) {
    assert!(
        matches!(outcome, PaneOutcome::Fired { item } if item == expected),
        "{outcome:?}"
    );
}

fn ping_held(outcome: PingOutcome, reason: HoldReason) {
    match outcome {
        PingOutcome::Held(actual) => assert_eq!(actual, reason),
        PingOutcome::Sent => panic!("held ping sent a trigger"),
    }
}

fn claim_stamp(conn: &Connection, member: i64) -> Option<String> {
    conn.query_row(
        "SELECT keystroke_at FROM member_placements WHERE member_id=?1",
        [member],
        |row| row.get(0),
    )
    .unwrap()
}

fn assert_no_send(io: &FakeIo, item: TestItem, conn: &Connection) {
    assert!(io.sent().is_empty(), "{:?}", io.events.borrow());
    assert_eq!(item.stamp(conn), None);
}

#[test]
fn shared_observation_preserves_typed_unsafe_reasons_and_capture_diagnostics() {
    for (capture, reason) in unsafe_observations() {
        let io = FakeIo::new();
        io.queue(MEMBER_PANE, vec![capture]);
        assert_eq!(observe_pane(&io, MEMBER_PANE, "claude"), Err(reason));
        assert!(io.sent().is_empty());
    }
}

#[test]
fn shared_observation_strips_ansi_and_accepts_only_the_recognized_empty_layout() {
    let io = FakeIo::new();
    io.queue(
        MEMBER_PANE,
        vec![Ok(format!("\u{1b}[32m{}\u{1b}[0m", real_capture("empty")))],
    );
    assert_eq!(observe_pane(&io, MEMBER_PANE, "claude"), Ok(()));
}

#[test]
fn every_owed_kind_holds_unsafe_initial_observations_regardless_of_age() {
    for kind in KINDS {
        for (capture, reason) in unsafe_observations() {
            if matches!(kind, Kind::Resume) && reason == HoldReason::Working {
                continue;
            }
            let mut f = Fixture::new();
            let item = f.item(kind);
            let io = FakeIo::new();
            io.queue(MEMBER_PANE, vec![capture]);
            held(deliver(&mut f, &io), reason);
            assert_no_send(&io, item, &f.conn);
            assert_eq!(claim_stamp(&f.conn, f.member_id), None);
        }
    }
}

#[test]
fn every_owed_kind_holds_unsafe_or_failed_final_observations_without_stamping() {
    for kind in KINDS {
        for (capture, reason) in unsafe_observations() {
            let mut f = Fixture::new();
            let item = f.item(kind);
            let io = FakeIo::new();
            io.queue(MEMBER_PANE, vec![Ok(real_capture("empty")), capture]);
            held(deliver(&mut f, &io), reason);
            assert_no_send(&io, item, &f.conn);
            assert!(claim_stamp(&f.conn, f.member_id).is_some());
        }
    }
}

#[test]
fn every_owed_item_stays_pending_across_old_draft_holds_and_delivers_after_clearing() {
    for kind in KINDS {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        let (draft, reason) = unsafe_observations()
            .into_iter()
            .find(|(_, reason)| *reason == HoldReason::Draft)
            .unwrap();
        let mut captures = Vec::new();
        for _ in 0..3 {
            captures.push(Ok(real_capture("empty")));
            captures.push(draft.clone());
        }
        captures.extend([Ok(real_capture("empty")), Ok(real_capture("empty"))]);
        io.queue(MEMBER_PANE, captures);
        for attempt in 1..=3 {
            held(deliver(&mut f, &io), reason.clone());
            assert_no_send(&io, item, &f.conn);
            assert_eq!(io.captures_for(MEMBER_PANE), attempt * 2);
            f.clock.advance(Duration::days(36500).num_milliseconds());
        }
        fired(deliver(&mut f, &io), item.expected());
        if matches!(kind, Kind::Dispatch) {
            held(deliver(&mut f, &io), HoldReason::ExecRunning);
        } else {
            assert!(matches!(deliver(&mut f, &io), PaneOutcome::Idle));
        }
        assert_eq!(io.captures_for(MEMBER_PANE), 8);
        assert_eq!(io.sent().len(), 1);
        assert!(item.stamp(&f.conn).is_some());
    }
}

#[test]
fn broadcast_deliveries_and_broker_notices_hold_unsafe_final_captures_and_remain_pollable() {
    for notice in [false, true] {
        for (capture, reason) in unsafe_observations() {
            let mut f = Fixture::new();
            let (member_id, pane, message_id) = if notice {
                let message =
                    broker::post_notice(&mut f.conn, f.fleet_id, "exec completion notice").unwrap();
                (f.director_id, DIRECTOR_PANE, message.message_id)
            } else {
                let broadcast =
                    broker::broadcast_message(&mut f.conn, f.director_id, "broadcast preview")
                        .unwrap();
                let message_id = broadcast
                    .deliveries
                    .into_iter()
                    .find(|(recipient, _)| *recipient == f.member_id)
                    .unwrap()
                    .1;
                (f.member_id, MEMBER_PANE, message_id)
            };
            let item = TestItem {
                kind: Kind::Preview,
                id: message_id,
            };
            let io = FakeIo::new();
            io.queue(pane, vec![Ok(real_capture("empty")), capture]);
            held(
                deliver_pane(&mut f.conn, &io, &settings(), member_id, &f.clock).unwrap(),
                reason,
            );
            assert_no_send(&io, item, &f.conn);
            assert_eq!(io.captures_for(pane), 2);
            assert!(
                broker::poll_messages(&f.conn, member_id)
                    .unwrap()
                    .iter()
                    .any(|message| message.message_id == message_id)
            );
        }
    }
}

#[test]
fn valid_delivery_claims_between_observations_and_commits_the_item_before_transport() {
    for kind in KINDS {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        let other = f.other_connection();
        let member = f.member_id;
        *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
            assert_eq!(item.stamp(&other), None);
            assert_eq!(claim_stamp(&other, member).is_some(), ordinal == 2);
        }));
        let mut other = f.other_connection();
        *io.send_hook.borrow_mut() = Some(Box::new(move || {
            assert!(item.stamp(&other).is_some());
            other
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap()
                .commit()
                .unwrap();
        }));
        fired(deliver(&mut f, &io), item.expected());
        let events = io.events.borrow();
        assert_eq!(events.len(), 3);
        assert_eq!(
            &events[..2],
            &[
                Event::Capture(MEMBER_PANE.to_string()),
                Event::Capture(MEMBER_PANE.to_string())
            ]
        );
        assert_eq!(io.sent().len(), 1);
    }
}

#[test]
fn exec_dispatch_and_resume_submit_the_exact_payload_through_the_automatic_operation() {
    for kind in [Kind::Dispatch, Kind::Resume] {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        fired(deliver(&mut f, &io), item.expected());
        let payload = match kind {
            Kind::Dispatch => format!("! cafleet member exec-run {}", item.id),
            Kind::Resume => format!(
                "[cafleet] exec {} finished with exit 7. Continue your work using its output above.",
                item.id
            ),
            Kind::Preview => unreachable!(),
        };
        assert_eq!(
            *io.events.borrow(),
            vec![
                Event::Capture(MEMBER_PANE.into()),
                Event::Capture(MEMBER_PANE.into()),
                Event::AutomaticPrompt(MEMBER_PANE.into(), payload),
            ]
        );
        assert!(item.stamp(&f.conn).is_some());
    }
}

#[test]
fn expired_receipts_hold_every_item_at_the_spacing_boundary_and_beyond() {
    for kind in KINDS {
        for milliseconds in [3000, 3001, 9000] {
            let mut f = Fixture::new();
            let item = f.item(kind);
            let io = FakeIo::new();
            io.safe(MEMBER_PANE, 2);
            let clock = f.clock.clone();
            *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
                if ordinal == 2 {
                    clock.advance(milliseconds);
                }
            }));
            held(deliver(&mut f, &io), HoldReason::Busy);
            assert_no_send(&io, item, &f.conn);
            assert!(claim_stamp(&f.conn, f.member_id).is_some());
        }
    }
}

#[test]
fn a_timely_receipt_remains_valid_just_inside_the_spacing_window() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 2);
    let clock = f.clock.clone();
    *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
        if ordinal == 2 {
            clock.advance(2999);
        }
    }));
    fired(deliver(&mut f, &io), item.expected());
}

#[test]
fn backwards_utc_during_final_capture_holds_every_item() {
    for kind in KINDS {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        let clock = f.clock.clone();
        *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
            if ordinal == 2 {
                clock.utc.set(clock.utc_now() - Duration::milliseconds(1));
            }
        }));
        held(deliver(&mut f, &io), HoldReason::Busy);
        assert_no_send(&io, item, &f.conn);
    }
}

#[test]
fn superseded_receipts_preserve_the_new_writer_and_leave_every_item_pending() {
    for kind in KINDS {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        let other = f.other_connection();
        let member = f.member_id;
        let replacement = format_utc(f.clock.utc_now() + Duration::milliseconds(1));
        let expected = replacement.clone();
        *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
            if ordinal == 2 {
                other
                    .execute(
                        "UPDATE member_placements SET keystroke_at=?1 WHERE member_id=?2",
                        params![replacement, member],
                    )
                    .unwrap();
            }
        }));
        held(deliver(&mut f, &io), HoldReason::Busy);
        assert_no_send(&io, item, &f.conn);
        assert_eq!(
            claim_stamp(&f.conn, f.member_id).as_deref(),
            Some(expected.as_str())
        );
    }
}

#[test]
fn an_expired_attempt_retries_with_two_new_observations_and_a_new_claim() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 4);
    let clock = f.clock.clone();
    *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
        if ordinal == 2 {
            clock.advance(3000);
        }
    }));
    held(deliver(&mut f, &io), HoldReason::Busy);
    let old = claim_stamp(&f.conn, f.member_id);
    fired(deliver(&mut f, &io), item.expected());
    assert_eq!(io.captures_for(MEMBER_PANE), 4);
    assert_ne!(claim_stamp(&f.conn, f.member_id), old);
    assert_eq!(io.sent().len(), 1);
}

#[test]
fn a_busy_claim_skips_the_final_observation_and_preserves_item_stamps() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    assert!(broker::claim_pane(&mut f.conn, f.member_id, f.clock.utc_now()).unwrap());
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 1);
    held(deliver(&mut f, &io), HoldReason::Busy);
    assert_no_send(&io, item, &f.conn);
    assert_eq!(io.captures_for(MEMBER_PANE), 1);
}

#[test]
fn ack_during_the_final_capture_prevents_preview_submission() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 2);
    let mut other = f.other_connection();
    *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
        if ordinal == 2 {
            broker::ack_message(&mut other, item.id).unwrap();
        }
    }));
    held(deliver(&mut f, &io), HoldReason::Busy);
    assert_no_send(&io, item, &f.conn);
}

#[test]
fn another_writer_stamping_the_selected_item_prevents_duplicate_submission() {
    for kind in KINDS {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        let other = f.other_connection();
        let when = format_utc(f.clock.utc_now());
        let expected = when.clone();
        *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
            if ordinal == 2 {
                let sql = match kind {
                    Kind::Preview => "UPDATE messages SET notified_at=?1 WHERE message_id=?2",
                    Kind::Dispatch => "UPDATE member_execs SET dispatched_at=?1 WHERE exec_id=?2",
                    Kind::Resume => "UPDATE member_execs SET resumed_at=?1 WHERE exec_id=?2",
                };
                other.execute(sql, params![when, item.id]).unwrap();
            }
        }));
        held(deliver(&mut f, &io), HoldReason::Busy);
        assert!(io.sent().is_empty());
        assert_eq!(item.stamp(&f.conn), Some(expected));
    }
}

#[test]
fn transport_failures_restore_every_owed_item_stamp_and_preserve_diagnostics() {
    for kind in KINDS {
        let mut f = Fixture::new();
        let item = f.item(kind);
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        *io.send_result.borrow_mut() =
            Err("backend Enter timeout: submission uncertain".to_string());
        held(
            deliver(&mut f, &io),
            HoldReason::KeystrokeFailed("backend Enter timeout: submission uncertain".to_string()),
        );
        assert_eq!(item.stamp(&f.conn), None);
        assert_eq!(io.sent().len(), 1);
    }
}

#[test]
fn running_exec_excludes_automatic_delivery_before_any_capture() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    let exec = f.item(Kind::Dispatch);
    f.conn
        .execute(
            "UPDATE member_execs SET dispatched_at=?1 WHERE exec_id=?2",
            params![format_utc(f.clock.utc_now()), exec.id],
        )
        .unwrap();
    let io = FakeIo::new();
    held(deliver(&mut f, &io), HoldReason::ExecRunning);
    assert_no_send(&io, item, &f.conn);
    assert!(io.events.borrow().is_empty());
}

#[test]
fn a_working_turn_consumes_finished_exec_output_without_a_resume_keystroke() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Resume);
    let io = FakeIo::new();
    io.queue(
        MEMBER_PANE,
        vec![Ok("✻ Cogitating… (12s · esc to interrupt)".to_string())],
    );
    held(deliver(&mut f, &io), HoldReason::Working);
    assert!(item.stamp(&f.conn).is_some());
    assert!(io.sent().is_empty());
    assert_eq!(io.captures_for(MEMBER_PANE), 1);
}

#[test]
fn resume_grace_holds_before_claiming_or_final_capture() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Resume);
    f.conn
        .execute(
            "UPDATE member_execs SET finished_at=?1 WHERE exec_id=?2",
            params![
                format_utc(f.clock.utc_now() - Duration::seconds(EXEC_RESUME_GRACE_SECONDS - 1)),
                item.id
            ],
        )
        .unwrap();
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 1);
    held(deliver(&mut f, &io), HoldReason::ResumeGrace);
    assert_no_send(&io, item, &f.conn);
    assert_eq!(claim_stamp(&f.conn, f.member_id), None);
}

#[test]
fn safe_transition_after_a_draft_delivers_the_pending_item_once() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Resume);
    let io = FakeIo::new();
    io.queue(
        MEMBER_PANE,
        vec![
            Ok(real_capture("draft-single")),
            Ok(real_capture("empty")),
            Ok(real_capture("empty")),
        ],
    );
    held(deliver(&mut f, &io), HoldReason::Draft);
    assert_no_send(&io, item, &f.conn);
    fired(deliver(&mut f, &io), item.expected());
    assert!(matches!(deliver(&mut f, &io), PaneOutcome::Idle));
    assert_eq!(io.sent().len(), 1);
}

#[test]
fn owed_items_preserve_resume_then_dispatch_then_oldest_preview_ordering() {
    let mut f = Fixture::new();
    let preview = f.item(Kind::Preview);
    let dispatch = f.item(Kind::Dispatch);
    let resume = f.item(Kind::Resume);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 6);
    fired(deliver(&mut f, &io), resume.expected());
    f.clock.advance(3000);
    fired(deliver(&mut f, &io), dispatch.expected());
    f.conn
        .execute(
            "UPDATE member_execs SET finished_at=?1,exit_code=0,resumed_at=?1 WHERE exec_id=?2",
            params![format_utc(f.clock.utc_now()), dispatch.id],
        )
        .unwrap();
    f.clock.advance(3000);
    fired(deliver(&mut f, &io), preview.expected());
}

#[test]
fn acked_messages_and_empty_queues_produce_no_observation_or_input() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    broker::ack_message(&mut f.conn, item.id).unwrap();
    let io = FakeIo::new();
    assert!(matches!(deliver(&mut f, &io), PaneOutcome::Idle));
    assert!(io.events.borrow().is_empty());
}

#[test]
fn automatic_claim_receipts_validate_ownership_and_lifetime_in_an_immediate_transaction() {
    let mut f = Fixture::new();
    let claim = broker::claim_automatic_pane(&mut f.conn, f.member_id, &f.clock)
        .unwrap()
        .unwrap();
    let tx = f
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(broker::validate_automatic_claim(&tx, &claim, &f.clock).unwrap());
    tx.commit().unwrap();
    f.clock.advance(3000);
    let tx = f
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(!broker::validate_automatic_claim(&tx, &claim, &f.clock).unwrap());
}

#[test]
fn claim_and_recheck_returns_the_final_observation_hold_reason() {
    let mut f = Fixture::new();
    let target = f.target(f.member_id);
    let io = FakeIo::new();
    io.queue(MEMBER_PANE, vec![Ok(real_capture("draft-single"))]);
    match claim_and_recheck(&mut f.conn, &io, &f.clock, &target).unwrap() {
        Err(reason) => assert_eq!(reason, HoldReason::Draft),
        Ok(_) => panic!("unsafe final capture produced a valid receipt"),
    }
    assert!(io.sent().is_empty());
}

#[test]
fn claim_database_failures_retain_the_concrete_sqlite_diagnostic() {
    let mut f = Fixture::new();
    f.conn
        .execute_batch("DROP TABLE member_placements")
        .unwrap();
    match broker::claim_automatic_pane(&mut f.conn, f.member_id, &f.clock) {
        Err(error) => assert!(error.to_string().contains("member_placements"), "{error}"),
        Ok(_) => panic!("missing required claim table must fail"),
    }
}

#[derive(Clone, Copy, Debug)]
enum InvalidReceipt {
    Boundary,
    LongCapture,
    MonotonicOnly,
    UtcOnly,
    BackwardsUtc,
    Superseded,
}
const INVALID_RECEIPTS: [InvalidReceipt; 6] = [
    InvalidReceipt::Boundary,
    InvalidReceipt::LongCapture,
    InvalidReceipt::MonotonicOnly,
    InvalidReceipt::UtcOnly,
    InvalidReceipt::BackwardsUtc,
    InvalidReceipt::Superseded,
];

fn invalidate_final(
    io: &FakeIo,
    f: &Fixture,
    pane: &'static str,
    member: i64,
    change: InvalidReceipt,
) {
    let clock = f.clock.clone();
    let other = f.other_connection();
    *io.capture_hook.borrow_mut() = Some(Box::new(move |observed, ordinal| {
        if observed != pane || ordinal != 2 {
            return;
        }
        match change {
            InvalidReceipt::Boundary => clock.advance(3000),
            InvalidReceipt::LongCapture => clock.advance(9000),
            InvalidReceipt::MonotonicOnly => clock
                .monotonic
                .set(clock.monotonic_now() + StdDuration::from_secs(3)),
            InvalidReceipt::UtcOnly => clock.utc.set(clock.utc_now() + Duration::seconds(3)),
            InvalidReceipt::BackwardsUtc => {
                clock.utc.set(clock.utc_now() - Duration::milliseconds(1))
            }
            InvalidReceipt::Superseded => {
                other
                    .execute(
                        "UPDATE member_placements SET keystroke_at=?1 WHERE member_id=?2",
                        params![
                            format_utc(clock.utc_now() + Duration::milliseconds(1)),
                            member
                        ],
                    )
                    .unwrap();
            }
        }
    }));
}

#[test]
fn either_clock_expiring_independently_prevents_queued_delivery() {
    for change in [InvalidReceipt::MonotonicOnly, InvalidReceipt::UtcOnly] {
        for kind in KINDS {
            let mut f = Fixture::new();
            let item = f.item(kind);
            let io = FakeIo::new();
            io.safe(MEMBER_PANE, 2);
            invalidate_final(&io, &f, MEMBER_PANE, f.member_id, change);
            held(deliver(&mut f, &io), HoldReason::Busy);
            assert_no_send(&io, item, &f.conn);
        }
    }
}

#[test]
fn a_retry_after_partial_transport_failure_observes_the_new_draft_and_holds() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    let io = FakeIo::new();
    io.queue(
        MEMBER_PANE,
        vec![
            Ok(real_capture("empty")),
            Ok(real_capture("empty")),
            Ok(real_capture("draft-single")),
        ],
    );
    *io.send_result.borrow_mut() = Err("partial insertion before timeout".to_string());
    held(
        deliver(&mut f, &io),
        HoldReason::KeystrokeFailed("partial insertion before timeout".to_string()),
    );
    f.clock.advance(3000);
    held(deliver(&mut f, &io), HoldReason::Draft);
    assert_eq!(item.stamp(&f.conn), None);
    assert_eq!(io.sent().len(), 1);
}

fn ping(f: &mut Fixture, io: &FakeIo) -> Result<PingOutcome, cafleet::error::CafleetError> {
    let target = f.target(f.member_id);
    ping_pane(&mut f.conn, io, &f.clock, &target)
}

#[test]
fn ping_holds_every_unsafe_initial_observation_without_claiming() {
    for (capture, reason) in unsafe_observations() {
        if capture.is_err() {
            continue;
        }
        let mut f = Fixture::new();
        let io = FakeIo::new();
        io.queue(MEMBER_PANE, vec![capture]);
        ping_held(ping(&mut f, &io).unwrap(), reason);
        assert!(io.sent().is_empty());
        assert_eq!(claim_stamp(&f.conn, f.member_id), None);
    }
}

#[test]
fn ping_rechecks_and_holds_every_unsafe_final_observation() {
    for (capture, reason) in unsafe_observations() {
        if capture.is_err() {
            continue;
        }
        let mut f = Fixture::new();
        let io = FakeIo::new();
        io.queue(MEMBER_PANE, vec![Ok(real_capture("empty")), capture]);
        ping_held(ping(&mut f, &io).unwrap(), reason);
        assert!(io.sent().is_empty());
        assert!(claim_stamp(&f.conn, f.member_id).is_some());
    }
}

#[test]
fn ping_capture_failures_preserve_concrete_errors_at_either_observation() {
    for final_capture in [false, true] {
        let mut f = Fixture::new();
        let io = FakeIo::new();
        let failure = Err("capture command timeout: pane %2".to_string());
        let captures = if final_capture {
            vec![Ok(real_capture("empty")), failure]
        } else {
            vec![failure]
        };
        io.queue(MEMBER_PANE, captures);
        match ping(&mut f, &io) {
            Err(error) => assert!(
                error
                    .to_string()
                    .contains("capture command timeout: pane %2"),
                "{error}"
            ),
            Ok(_) => panic!("explicit ping capture failure must retain its error contract"),
        }
        assert!(io.sent().is_empty());
    }
}

#[test]
fn ping_sends_the_prepared_poll_trigger_only_after_both_observations() {
    let mut f = Fixture::new();
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 2);
    assert!(matches!(ping(&mut f, &io).unwrap(), PingOutcome::Sent));
    let events = io.events.borrow();
    assert_eq!(events.len(), 3);
    assert_eq!(
        &events[..2],
        &[
            Event::Capture(MEMBER_PANE.to_string()),
            Event::Capture(MEMBER_PANE.to_string())
        ]
    );
    match &events[2] {
        Event::Trigger(pane, payload) => {
            assert_eq!(pane, MEMBER_PANE);
            assert!(payload.contains(&format!("cafleet message poll {}", f.member_id)));
        }
        event => panic!("unexpected ping transport: {event:?}"),
    }
}

#[test]
fn ping_holds_expired_superseded_and_backwards_clock_receipts() {
    for change in INVALID_RECEIPTS {
        let mut f = Fixture::new();
        let io = FakeIo::new();
        io.safe(MEMBER_PANE, 2);
        invalidate_final(&io, &f, MEMBER_PANE, f.member_id, change);
        ping_held(ping(&mut f, &io).unwrap(), HoldReason::Busy);
        assert!(io.sent().is_empty());
        assert!(claim_stamp(&f.conn, f.member_id).is_some());
    }
}

#[test]
fn ping_busy_claim_refusal_sends_nothing_and_skips_the_second_capture() {
    let mut f = Fixture::new();
    assert!(broker::claim_pane(&mut f.conn, f.member_id, f.clock.utc_now()).unwrap());
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 1);
    ping_held(ping(&mut f, &io).unwrap(), HoldReason::Busy);
    assert!(io.sent().is_empty());
    assert_eq!(io.captures_for(MEMBER_PANE), 1);
}

#[test]
fn ping_running_exec_exclusion_precedes_observation() {
    let mut f = Fixture::new();
    let exec = f.item(Kind::Dispatch);
    f.conn
        .execute(
            "UPDATE member_execs SET dispatched_at=?1 WHERE exec_id=?2",
            params![format_utc(f.clock.utc_now()), exec.id],
        )
        .unwrap();
    let io = FakeIo::new();
    ping_held(ping(&mut f, &io).unwrap(), HoldReason::ExecRunning);
    assert!(io.events.borrow().is_empty());
}

#[test]
fn ping_retry_after_expiration_repeats_both_observations() {
    let mut f = Fixture::new();
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 4);
    invalidate_final(&io, &f, MEMBER_PANE, f.member_id, InvalidReceipt::Boundary);
    ping_held(ping(&mut f, &io).unwrap(), HoldReason::Busy);
    assert!(matches!(ping(&mut f, &io).unwrap(), PingOutcome::Sent));
    assert_eq!(io.captures_for(MEMBER_PANE), 4);
    assert_eq!(io.sent().len(), 1);
}

fn wake_stamps(f: &Fixture) -> (Option<String>, Option<String>) {
    let row = broker::read_monitor_runtime(&f.conn, f.fleet_id)
        .unwrap()
        .unwrap();
    (row.last_wake_at, row.wake_requested_at)
}

#[test]
fn scheduled_and_requested_wakes_hold_unsafe_initial_observations_and_keep_due_state() {
    for requested in [false, true] {
        for (capture, _) in unsafe_observations() {
            let mut f = Fixture::new();
            f.start_monitor(requested);
            let before = wake_stamps(&f);
            let io = FakeIo::new();
            io.queue(MONITOR_PANE, vec![capture]);
            f.tick(&io);
            assert!(io.sent().is_empty());
            assert_eq!(wake_stamps(&f), before);
            assert_eq!(claim_stamp(&f.conn, f.monitor_id), None);
            assert_eq!(
                broker::read_monitor_runtime(&f.conn, f.fleet_id)
                    .unwrap()
                    .unwrap()
                    .last_tick_at,
                Some(format_utc(f.clock.utc_now()))
            );
        }
    }
}

#[test]
fn scheduled_and_requested_wakes_hold_unsafe_or_failed_final_observations() {
    for requested in [false, true] {
        for (capture, _) in unsafe_observations() {
            let mut f = Fixture::new();
            f.start_monitor(requested);
            let before = wake_stamps(&f);
            let io = FakeIo::new();
            io.queue(MONITOR_PANE, vec![Ok(real_capture("empty")), capture]);
            f.tick(&io);
            assert!(io.sent().is_empty());
            assert_eq!(wake_stamps(&f), before);
            assert!(claim_stamp(&f.conn, f.monitor_id).is_some());
        }
    }
}

#[test]
fn wake_success_stamps_only_after_a_valid_claim_and_prepared_trigger_submission() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    let io = FakeIo::new();
    io.safe(MONITOR_PANE, 2);
    let other = f.other_connection();
    let fleet = f.fleet_id;
    *io.send_hook.borrow_mut() = Some(Box::new(move || {
        let row = broker::read_monitor_runtime(&other, fleet)
            .unwrap()
            .unwrap();
        assert_eq!(row.last_wake_at, None);
        assert!(row.wake_requested_at.is_some());
        other.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").unwrap();
    }));
    f.tick(&io);
    assert_eq!(wake_stamps(&f), (Some(format_utc(f.clock.utc_now())), None));
    let events = io.events.borrow();
    assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], Event::Capture(pane) if pane == MONITOR_PANE));
    assert!(matches!(&events[1], Event::Capture(pane) if pane == MONITOR_PANE));
    match &events[2] {
        Event::Trigger(pane, payload) => {
            assert_eq!(pane, MONITOR_PANE);
            assert!(payload.contains("[cafleet] tick:"));
            assert!(payload.contains("Director:"));
        }
        event => panic!("unexpected wake transport: {event:?}"),
    }
}

#[test]
fn wake_expired_superseded_or_backwards_receipts_preserve_due_and_request_stamps() {
    for requested in [false, true] {
        for change in INVALID_RECEIPTS {
            let mut f = Fixture::new();
            f.start_monitor(requested);
            let before = wake_stamps(&f);
            let io = FakeIo::new();
            io.safe(MONITOR_PANE, 2);
            invalidate_final(&io, &f, MONITOR_PANE, f.monitor_id, change);
            f.tick(&io);
            assert!(io.sent().is_empty());
            assert_eq!(wake_stamps(&f), before);
            assert!(claim_stamp(&f.conn, f.monitor_id).is_some());
        }
    }
}

#[test]
fn wake_expiration_retry_uses_fresh_captures_before_clearing_the_request() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    let before = wake_stamps(&f);
    let io = FakeIo::new();
    io.safe(MONITOR_PANE, 4);
    invalidate_final(
        &io,
        &f,
        MONITOR_PANE,
        f.monitor_id,
        InvalidReceipt::Boundary,
    );
    f.tick(&io);
    assert_eq!(wake_stamps(&f), before);
    f.tick(&io);
    assert_eq!(io.captures_for(MONITOR_PANE), 4);
    assert_eq!(io.sent().len(), 1);
    assert_eq!(wake_stamps(&f), (Some(format_utc(f.clock.utc_now())), None));
}

#[test]
fn wake_send_failure_preserves_request_and_retries_with_new_safe_observations() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    let before = wake_stamps(&f);
    let io = FakeIo::new();
    io.safe(MONITOR_PANE, 4);
    io.trigger_ok.set(false);
    f.tick(&io);
    assert_eq!(wake_stamps(&f), before);
    f.clock.advance(3000);
    io.trigger_ok.set(true);
    f.tick(&io);
    assert_eq!(io.captures_for(MONITOR_PANE), 4);
    assert_eq!(io.sent().len(), 2);
    assert_eq!(wake_stamps(&f), (Some(format_utc(f.clock.utc_now())), None));
}

#[test]
fn a_held_wake_preserves_heartbeats_and_delivers_other_panes_owed_work() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    let item = f.item(Kind::Preview);
    let before = wake_stamps(&f);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 2);
    io.queue(MONITOR_PANE, vec![Ok(real_capture("draft-single"))]);
    f.tick(&io);
    assert!(item.stamp(&f.conn).is_some());
    assert_eq!(io.sent().len(), 1);
    assert_eq!(wake_stamps(&f), before);
    assert_eq!(
        broker::read_monitor_runtime(&f.conn, f.fleet_id)
            .unwrap()
            .unwrap()
            .last_tick_at,
        Some(format_utc(f.clock.utc_now()))
    );
}

#[test]
fn repeated_due_ticks_coalesce_into_one_wake_after_the_draft_clears() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    let before = wake_stamps(&f);
    let io = FakeIo::new();
    io.queue(
        MONITOR_PANE,
        vec![
            Ok(real_capture("draft-single")),
            Ok(real_capture("draft-single")),
            Ok(real_capture("draft-single")),
            Ok(real_capture("empty")),
            Ok(real_capture("empty")),
        ],
    );
    for _ in 0..3 {
        f.tick(&io);
        assert_eq!(wake_stamps(&f), before);
        f.clock.advance(1000);
    }
    assert!(io.sent().is_empty());
    f.tick(&io);
    assert_eq!(io.sent().len(), 1);
    assert_eq!(wake_stamps(&f).1, None);
    f.tick(&io);
    assert_eq!(io.sent().len(), 1);
}

#[test]
fn a_requested_wake_bypasses_the_schedule_but_still_requires_two_safe_captures() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    f.conn
        .execute(
            "UPDATE monitor_runtime SET last_wake_at=?1 WHERE fleet_id=?2",
            params![format_utc(f.clock.utc_now()), f.fleet_id],
        )
        .unwrap();
    let io = FakeIo::new();
    io.safe(MONITOR_PANE, 2);
    f.tick(&io);
    assert_eq!(io.captures_for(MONITOR_PANE), 2);
    assert_eq!(io.sent().len(), 1);
    assert_eq!(wake_stamps(&f).1, None);
}

#[test]
fn wake_running_exec_exclusion_precedes_any_capture() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    let exec = f.item_for(Kind::Dispatch, f.monitor_id);
    f.conn
        .execute(
            "UPDATE member_execs SET dispatched_at=?1 WHERE exec_id=?2",
            params![format_utc(f.clock.utc_now()), exec.id],
        )
        .unwrap();
    let before = wake_stamps(&f);
    let io = FakeIo::new();
    f.tick(&io);
    assert!(io.events.borrow().is_empty());
    assert_eq!(wake_stamps(&f), before);
}

#[test]
fn wake_busy_claim_refusal_keeps_the_request_and_skips_final_capture() {
    let mut f = Fixture::new();
    f.start_monitor(true);
    assert!(broker::claim_pane(&mut f.conn, f.monitor_id, f.clock.utc_now()).unwrap());
    let before = wake_stamps(&f);
    let io = FakeIo::new();
    io.safe(MONITOR_PANE, 1);
    f.tick(&io);
    assert_eq!(io.captures_for(MONITOR_PANE), 1);
    assert!(io.sent().is_empty());
    assert_eq!(wake_stamps(&f), before);
}

#[test]
fn sqlite_claim_wait_time_counts_toward_the_monotonic_receipt_budget() {
    thread_local! {
        static WAIT: RefCell<Option<(Connection, FakeClock)>> = const { RefCell::new(None) };
    }
    let mut f = Fixture::new();
    let other = f.other_connection();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    WAIT.with(|slot| *slot.borrow_mut() = Some((other, f.clock.clone())));
    f.conn
        .busy_handler(Some(|_| {
            WAIT.with(|slot| match slot.borrow_mut().take() {
                Some((other, clock)) => {
                    clock
                        .monotonic
                        .set(clock.monotonic_now() + StdDuration::from_secs(3));
                    other.execute_batch("ROLLBACK").unwrap();
                    true
                }
                None => false,
            })
        }))
        .unwrap();
    let receipt = broker::claim_automatic_pane(&mut f.conn, f.member_id, &f.clock)
        .unwrap()
        .unwrap();
    f.conn.busy_handler(None).unwrap();
    assert!(
        WAIT.with(|slot| slot.borrow().is_none()),
        "the conditional claim must encounter the held write lock"
    );
    let tx = f
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(!broker::validate_automatic_claim(&tx, &receipt, &f.clock).unwrap());
}

#[test]
fn oldest_pending_previews_are_selected_in_order_and_delivered_once() {
    let mut f = Fixture::new();
    let first = f.item(Kind::Preview);
    let second = f.item(Kind::Preview);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 4);
    fired(deliver(&mut f, &io), first.expected());
    assert_eq!(second.stamp(&f.conn), None);
    f.clock.advance(3000);
    fired(deliver(&mut f, &io), second.expected());
    assert!(matches!(deliver(&mut f, &io), PaneOutcome::Idle));
    assert_eq!(io.sent().len(), 2);
}

#[test]
fn director_previews_use_the_same_initial_and_final_safety_checks() {
    let mut f = Fixture::new();
    let item = f.item_for(Kind::Preview, f.director_id);
    let io = FakeIo::new();
    io.queue(
        DIRECTOR_PANE,
        vec![Ok(real_capture("empty")), Ok(real_capture("draft-single"))],
    );
    let outcome = deliver_pane(&mut f.conn, &io, &settings(), f.director_id, &f.clock).unwrap();
    held(outcome, HoldReason::Draft);
    assert_no_send(&io, item, &f.conn);
    assert_eq!(io.captures_for(DIRECTOR_PANE), 2);
}

#[test]
fn validation_database_errors_are_reported_without_transport_or_item_stamp() {
    let mut f = Fixture::new();
    let item = f.item(Kind::Preview);
    let io = FakeIo::new();
    io.safe(MEMBER_PANE, 2);
    let other = f.other_connection();
    *io.capture_hook.borrow_mut() = Some(Box::new(move |_, ordinal| {
        if ordinal == 2 {
            other.execute_batch("DROP TABLE member_placements").unwrap();
        }
    }));
    match deliver_pane(&mut f.conn, &io, &settings(), f.member_id, &f.clock) {
        Err(error) => assert!(error.to_string().contains("member_placements"), "{error}"),
        Ok(_) => panic!("a failed ownership query must retain its database error"),
    }
    assert_no_send(&io, item, &f.conn);
}
