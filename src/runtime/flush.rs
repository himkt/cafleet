use crate::broker;
use crate::broker::MemberKind;
use crate::broker::MemberRecord;
use crate::broker::MemberStatus;
use crate::broker::runtime as storage;
use crate::broker::runtime::RunRecord;
use crate::broker::runtime::RunStatus;
use crate::config::Settings;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::PressKeys;
use crate::runtime::ControlIdentity;
use crate::runtime::FlushAction;
use crate::runtime::FlushReply;
use crate::runtime::FlushWriteOutcome;
use crate::runtime::InputControlOperation;
use crate::runtime::InputControlReply;
use crate::runtime::InputControlRequest;
use crate::runtime::InputExpiration;
use crate::runtime::RealInputDeadlineClock;
use crate::runtime::RuntimePaths;
use crate::time::Deadline;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;

/// The command's whole budget: lock wait, both exchanges, both writes and the settle interval.
pub(crate) const FLUSH_DEADLINE: Duration = Duration::from_secs(10);
/// How long past the command's deadline the runner keeps the reservation before expiring it.
const RESERVATION_GRACE: Duration = Duration::from_secs(1);

/// Carries one flush control action to the member's runner and returns its reply.
pub(crate) trait FlushChannel {
    fn exchange(
        &self,
        run: &RunRecord,
        action: FlushAction,
        deadline: Deadline,
    ) -> Result<FlushReply, CafleetError>;
}

/// The production channel over the run's control socket.
pub(crate) struct ControlFlushChannel;
impl FlushChannel for ControlFlushChannel {
    fn exchange(
        &self,
        run: &RunRecord,
        action: FlushAction,
        deadline: Deadline,
    ) -> Result<FlushReply, CafleetError> {
        let until = deadline.instant();
        let expiration = InputExpiration::capture(until, &mut RealInputDeadlineClock)?;
        let request = InputControlRequest {
            identity: ControlIdentity::from_run(run),
            expires_at_mono_ns: expiration.monotonic_ns(),
            operation: InputControlOperation::Flush(action),
        };
        let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
        let mut client =
            crate::runtime::control::ControlClient::connect(&paths.control_socket, until)?;
        match client.exchange_input(&request, until)? {
            InputControlReply::Flush(reply) => Ok(reply),
            _ => Err(CafleetError::App(
                "runner control: a flush action received another reply kind".into(),
            )),
        }
    }
}

pub(crate) struct FlushOutcome {
    pub(crate) member_id: i64,
    pub(crate) pane_id: String,
    pub(crate) flush_input_id: Option<i64>,
    pub(crate) flushed_input_ids: Vec<i64>,
    pub(crate) message_ids: Vec<i64>,
    pub(crate) remaining_input_ids: Vec<i64>,
}

impl FlushOutcome {
    pub(crate) fn to_json(&self) -> Value {
        json!({"member_id":self.member_id,"pane_id":self.pane_id,
            "flush_input_id":self.flush_input_id,"flushed_input_ids":self.flushed_input_ids,
            "message_ids":self.message_ids,"remaining_input_ids":self.remaining_input_ids,
            "delivery":self.flush_input_id.map(|_| json!({"status":"submitted","method":"forced","reason":null}))})
    }

    pub(crate) fn text(&self) -> String {
        let Some(flush_input_id) = self.flush_input_id else {
            return format!("No queued previews for member {}", self.member_id);
        };
        let mut text = format!(
            "Flushed {} queued previews to member {} (input {flush_input_id}): messages {}",
            self.flushed_input_ids.len(),
            self.member_id,
            joined(&self.message_ids)
        );
        if !self.remaining_input_ids.is_empty() {
            text.push_str(&format!(
                "\nStill queued: {}",
                joined(&self.remaining_input_ids)
            ));
        }
        text
    }
}

fn joined(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

fn refused(member_id: i64, cause: impl std::fmt::Display) -> CafleetError {
    CafleetError::App(format!(
        "queued input was not flushed to member {member_id}: {cause}"
    ))
}

fn uncertain(member_id: i64, cause: impl std::fmt::Display, flush_input_id: i64) -> CafleetError {
    CafleetError::App(format!(
        "queued input flush to member {member_id} is uncertain: {cause} (input_id={flush_input_id}). Inspect the pane, then reconcile input {flush_input_id}."
    ))
}

pub(crate) fn flush_member(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    deadline: Deadline,
) -> Result<FlushOutcome, CafleetError> {
    let (member, mux) = with_database_budget(conn, deadline, |conn| {
        let fleet_id = match broker::member_fleet_status(conn, member_id)? {
            Some((fleet_id, MemberStatus::Active)) => fleet_id,
            Some(_) => {
                return Err(refused(
                    member_id,
                    format!("member {member_id} is not active"),
                ));
            }
            None => return Err(CafleetError::App(format!("Member {member_id} not found"))),
        };
        let member = broker::get_member(conn, member_id, fleet_id)?
            .ok_or_else(|| refused(member_id, format!("member {member_id} is not active")))?;
        let mux = crate::runtime::mux_for_fleet(conn, member.fleet_id, Some(deadline))?;
        Ok((member, mux))
    })?;
    flush_member_with(
        conn,
        settings,
        member,
        deadline,
        &*mux,
        &ControlFlushChannel,
        std::thread::sleep,
    )
}

/// The `member flush` procedure: lock, target check, runner reservation, the literal and submit
/// key through the outer pane, then the runner settlement of the write outcome.
pub(crate) fn flush_member_with(
    conn: &mut Connection,
    settings: &Settings,
    member: MemberRecord,
    deadline: Deadline,
    mux: &dyn Multiplexer,
    channel: &impl FlushChannel,
    sleep: impl Fn(Duration),
) -> Result<FlushOutcome, CafleetError> {
    let member_id = member.member_id;
    if member.kind == MemberKind::Director {
        return Err(refused(member_id, "cannot flush the Director's own pane"));
    }
    let placement = &member.placement;
    let pane_id = placement.mux_pane_id.clone().ok_or_else(|| {
        refused(
            member_id,
            format!("member {member_id} has no pane yet (pending placement)"),
        )
    })?;
    let run = with_database_budget(conn, deadline, |conn| {
        let run_id: i64 = conn
            .query_row(
                "SELECT run_id FROM member_runs WHERE member_id=?1 ORDER BY run_id DESC LIMIT 1",
                [member_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| refused(member_id, "the member has no recorded run"))?;
        storage::get_run(conn, run_id)
    })?;
    if run.status != RunStatus::Running {
        return Err(refused(
            member_id,
            format!("the member's run is {}", run.status.as_str()),
        ));
    }
    let locked = crate::pane_lock::lock_member_with_reservation(conn, settings, &member, deadline)?
        .ok_or_else(|| {
            refused(
                member_id,
                format!("member {member_id}'s pane input remained busy"),
            )
        })?;
    mux.validate_key_target(placement, deadline).map_err(|_| {
        refused(
            member_id,
            "press target context does not match the recorded placement",
        )
    })?;
    require_same_placement(conn, &member, deadline)
        .map_err(|cause| refused(member_id, cause.message()))?;
    let reply = channel
        .exchange(
            &run,
            FlushAction::Reserve {
                director_member_id: run.director_member_id,
                expires_at_mono_ns: InputExpiration::capture(
                    deadline.instant() + RESERVATION_GRACE,
                    &mut RealInputDeadlineClock,
                )?
                .monotonic_ns(),
            },
            deadline,
        )
        .map_err(|cause| {
            refused(
                member_id,
                format!("the member's runner is unreachable: {}", cause.message()),
            )
        })?;
    let (flush_input_id, literal, submit_key, settle_ms, flushed, message_ids, remaining) =
        match reply {
            FlushReply::Empty => {
                return Ok(FlushOutcome {
                    member_id,
                    pane_id,
                    flush_input_id: None,
                    flushed_input_ids: Vec::new(),
                    message_ids: Vec::new(),
                    remaining_input_ids: Vec::new(),
                });
            }
            FlushReply::Refused { cause } => return Err(refused(member_id, cause)),
            FlushReply::Reserved {
                flush_input_id,
                literal,
                submit_key,
                settle_ms,
                flushed_input_ids,
                message_ids,
                remaining_input_ids,
            } => (
                flush_input_id,
                literal,
                submit_key,
                settle_ms,
                flushed_input_ids,
                message_ids,
                remaining_input_ids,
            ),
            FlushReply::Settled { .. } | FlushReply::NotOutstanding { .. } => {
                return Err(CafleetError::App(
                    "runner control: a flush reservation received a settlement reply".into(),
                ));
            }
        };
    let keys = PressKeys::parse(&[submit_key])?;
    let mut started = false;
    let written = (|| {
        let _span = crate::time::DeferTermination::begin()?;
        require_same_placement(conn, &member, deadline)?;
        locked.pane_lock(&settings.database_url)?;
        deadline.remaining()?;
        mux.send_text(&pane_id, &literal, deadline)
            .map_err(|cause| {
                started = cause.submission_started();
                CafleetError::App(cause.to_string())
            })?;
        started = true;
        sleep(Duration::from_millis(settle_ms));
        deadline.remaining()?;
        mux.send_keys(&pane_id, &keys, deadline)
            .map_err(|cause| CafleetError::App(cause.to_string()))
    })();
    let outcome = match (&written, started) {
        (Ok(()), _) => FlushWriteOutcome::Acknowledged,
        (Err(_), false) => FlushWriteOutcome::NotStarted,
        (Err(_), true) => FlushWriteOutcome::Uncertain,
    };
    let settled = channel.exchange(
        &run,
        FlushAction::Settle {
            flush_input_id,
            outcome,
        },
        deadline,
    );
    drop(locked);
    match (written, settled) {
        (
            _,
            Ok(FlushReply::NotOutstanding {
                state, disposition, ..
            }),
        ) => Err(late_settlement(
            member_id,
            flush_input_id,
            state,
            disposition,
        )),
        (Err(cause), _) if outcome == FlushWriteOutcome::NotStarted => {
            Err(refused(member_id, cause.message()))
        }
        (Err(cause), _) => Err(uncertain(member_id, cause.message(), flush_input_id)),
        (Ok(()), Ok(FlushReply::Settled { .. })) => Ok(FlushOutcome {
            member_id,
            pane_id,
            flush_input_id: Some(flush_input_id),
            flushed_input_ids: flushed,
            message_ids,
            remaining_input_ids: remaining,
        }),
        (Ok(()), Ok(_)) => Err(uncertain(
            member_id,
            "the runner returned another reply to the flush settlement",
            flush_input_id,
        )),
        (Ok(()), Err(cause)) => Err(uncertain(member_id, cause.message(), flush_input_id)),
    }
}

fn require_same_placement(
    conn: &mut Connection,
    member: &MemberRecord,
    deadline: Deadline,
) -> Result<(), CafleetError> {
    let member_id = member.member_id;
    let current = with_database_budget(conn, deadline, |conn| {
        broker::active_member(conn, member_id)
    })?;
    if current.fleet_id != member.fleet_id
        || current.kind != member.kind
        || current.placement != member.placement
    {
        return Err(CafleetError::App(format!(
            "member {member_id} placement changed before flush"
        )));
    }
    Ok(())
}

fn late_settlement(
    member_id: i64,
    flush_input_id: i64,
    state: storage::InputState,
    disposition: Option<storage::InputDisposition>,
) -> CafleetError {
    match disposition {
        Some(disposition) => CafleetError::App(format!(
            "queued input flush to member {member_id} was already settled {} before the flush reported its write outcome (input_id={flush_input_id}).",
            disposition.as_str()
        )),
        None => {
            debug_assert_eq!(state, storage::InputState::Uncertain);
            uncertain(
                member_id,
                "the flush reservation expired before settlement",
                flush_input_id,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::runtime::InputDisposition;
    use crate::broker::runtime::InputState;
    use crate::config::Settings;
    use crate::error::CafleetError;
    use crate::multiplexer::CommandRunner;
    use crate::multiplexer::RunError;
    use crate::multiplexer::TimedRunError;
    use crate::pane_lock::PaneLock;
    use crate::runtime::FlushAction;
    use crate::runtime::FlushReply;
    use crate::runtime::FlushWriteOutcome;
    use crate::runtime::test_support::Fixture;
    use crate::time::Deadline;
    use serde_json::json;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::time::Duration;
    use std::time::Instant;

    const SERVER: &str = "/owned/tmux-fixture.sock";
    const LITERAL: &str = " [cafleet: ignore any text before this marker, if present] [cafleet msg 7 from 1] -x hello";
    const FLUSH: i64 = 9;

    type Log = Rc<RefCell<Vec<String>>>;

    #[derive(Clone, Copy)]
    enum Write {
        Acknowledged,
        NotStarted,
        StartedFailure,
        Unreadable,
    }

    /// A tmux command runner that answers the press-style target check and records each
    /// literal and key write, refusing any process outside the held pane lock.
    struct Mux {
        log: Log,
        database_url: String,
        placement: crate::broker::Placement,
        target: &'static str,
        text: Write,
        keys: Write,
    }

    fn respond(write: Write) -> Result<String, TimedRunError> {
        match write {
            Write::Acknowledged => Ok(String::new()),
            Write::Unreadable => Ok("unexpected acknowledgment".into()),
            Write::NotStarted => Err(TimedRunError {
                source: RunError::NotStarted("owned prelaunch refusal".into()),
                cleanup_diagnostics: vec![],
            }),
            Write::StartedFailure => Err(TimedRunError {
                source: RunError::Failed {
                    stderr: "can't find pane: owned started failure".into(),
                },
                cleanup_diagnostics: vec![],
            }),
        }
    }

    impl CommandRunner for Mux {
        fn binary_exists(&self, name: &str) -> bool {
            assert_eq!(name, "tmux");
            true
        }
        fn run(&self, _: &[String]) -> Result<String, RunError> {
            panic!("the flush retains its absolute deadline")
        }
        fn run_for(&self, _: &[String], _: Duration) -> Result<String, RunError> {
            panic!("the flush does not renew a relative budget")
        }
        fn run_until(&self, args: &[String], until: Instant) -> Result<String, TimedRunError> {
            assert!(Instant::now() < until, "no process is offered after expiry");
            assert!(
                PaneLock::try_acquire(&self.database_url, &self.placement)
                    .unwrap()
                    .is_none(),
                "the pane input lock is held through every multiplexer process"
            );
            assert_eq!(args[..3], ["tmux", "-S", SERVER], "{args:?}");
            match args[3].as_str() {
                "display-message" => {
                    self.log.borrow_mut().push("target".into());
                    Ok(format!("{}\n", self.target))
                }
                "send-keys" if args.iter().any(|arg| arg == "-l") => {
                    assert_eq!(args[4..], ["-t", "%2", "-l", LITERAL]);
                    self.log.borrow_mut().push("text".into());
                    respond(self.text)
                }
                "send-keys" => {
                    self.log
                        .borrow_mut()
                        .push(format!("keys:{}", args[4..].join(" ")));
                    respond(self.keys)
                }
                other => panic!("unexpected flush process: {other}"),
            }
        }
    }

    /// A control channel that records each action and answers with the scripted replies;
    /// `on_reserve` runs when the reservation is answered.
    struct Channel {
        log: Log,
        run_id: i64,
        replies: RefCell<VecDeque<FlushReply>>,
        on_reserve: Box<dyn Fn()>,
    }

    impl FlushChannel for Channel {
        fn exchange(
            &self,
            run: &crate::broker::runtime::RunRecord,
            action: FlushAction,
            deadline: Deadline,
        ) -> Result<FlushReply, CafleetError> {
            assert_eq!(run.run_id, self.run_id);
            deadline.remaining()?;
            match action {
                FlushAction::Reserve { .. } => {
                    self.log.borrow_mut().push("reserve".into());
                    (self.on_reserve)();
                }
                FlushAction::Settle {
                    flush_input_id,
                    outcome,
                } => {
                    let outcome = match outcome {
                        FlushWriteOutcome::Acknowledged => "acknowledged",
                        FlushWriteOutcome::NotStarted => "not_started",
                        FlushWriteOutcome::Uncertain => "uncertain",
                    };
                    self.log
                        .borrow_mut()
                        .push(format!("settle:{flush_input_id}:{outcome}"));
                }
            }
            Ok(self
                .replies
                .borrow_mut()
                .pop_front()
                .expect("a scripted reply for every exchange"))
        }
    }

    struct Harness {
        fixture: Fixture,
        settings: Settings,
        log: Log,
    }

    fn harness() -> Harness {
        let fixture = Fixture::new();
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='running' WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        let settings = Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()));
        Harness {
            fixture,
            settings,
            log: Log::default(),
        }
    }

    fn reservation(flushed: &[i64], messages: &[i64], remaining: &[i64]) -> FlushReply {
        FlushReply::Reserved {
            flush_input_id: FLUSH,
            literal: LITERAL.into(),
            submit_key: "Enter".into(),
            settle_ms: 500,
            flushed_input_ids: flushed.to_vec(),
            message_ids: messages.to_vec(),
            remaining_input_ids: remaining.to_vec(),
        }
    }

    fn settled() -> FlushReply {
        FlushReply::Settled {
            flush_input_id: FLUSH,
        }
    }

    struct Case {
        target: &'static str,
        text: Write,
        keys: Write,
        replies: Vec<FlushReply>,
        on_reserve: Box<dyn Fn()>,
        deadline: Duration,
    }

    impl Case {
        fn replying(replies: Vec<FlushReply>) -> Self {
            Self {
                target: "%2|@1|main",
                text: Write::Acknowledged,
                keys: Write::Acknowledged,
                replies,
                on_reserve: Box::new(|| {}),
                deadline: Duration::from_secs(10),
            }
        }
    }

    impl Harness {
        fn member_id(&self) -> i64 {
            self.fixture.run.member_id
        }

        fn member(&self) -> crate::broker::MemberRecord {
            crate::broker::active_member(&self.fixture.conn, self.member_id()).unwrap()
        }

        fn flush_as(
            &mut self,
            member: crate::broker::MemberRecord,
            case: Case,
        ) -> Result<FlushOutcome, CafleetError> {
            let mux = Rc::new(Mux {
                log: self.log.clone(),
                database_url: self.settings.database_url.clone(),
                placement: member.placement.clone(),
                target: case.target,
                text: case.text,
                keys: case.keys,
            });
            let mux = crate::multiplexer::multiplexer_for_server("tmux", SERVER, mux).unwrap();
            let channel = Channel {
                log: self.log.clone(),
                run_id: self.fixture.run.run_id,
                replies: RefCell::new(case.replies.into()),
                on_reserve: case.on_reserve,
            };
            let log = self.log.clone();
            let sleep = move |interval: Duration| {
                log.borrow_mut()
                    .push(format!("sleep:{}", interval.as_millis()))
            };
            flush_member_with(
                &mut self.fixture.conn,
                &self.settings,
                member,
                Deadline::until(Instant::now() + case.deadline),
                &*mux,
                &channel,
                &sleep,
            )
        }

        fn flush(&mut self, case: Case) -> Result<FlushOutcome, CafleetError> {
            let member = self.member();
            self.flush_as(member, case)
        }

        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }

        fn lock_released(&self) -> bool {
            PaneLock::try_acquire(&self.settings.database_url, &self.member().placement)
                .unwrap()
                .is_some()
        }
    }

    fn failure(result: Result<FlushOutcome, CafleetError>) -> CafleetError {
        match result {
            Ok(_) => panic!("this flush fails"),
            Err(cause) => cause,
        }
    }

    fn not_flushed(member_id: i64, cause: &str) -> String {
        format!("queued input was not flushed to member {member_id}: {cause}")
    }

    fn uncertain(member_id: i64, cause: &str) -> String {
        format!(
            "queued input flush to member {member_id} is uncertain: {cause} (input_id={FLUSH}). Inspect the pane, then reconcile input {FLUSH}."
        )
    }

    #[test]
    fn a_reserved_flush_types_the_literal_settles_then_submits_and_acknowledges() {
        let mut h = harness();
        let member_id = h.member_id();
        let outcome = h
            .flush(Case::replying(vec![
                reservation(&[4, 5], &[7, 8], &[]),
                settled(),
            ]))
            .unwrap();
        assert_eq!(
            h.log(),
            [
                "target",
                "reserve",
                "text",
                "sleep:500",
                "keys:-t %2 Enter",
                "settle:9:acknowledged"
            ]
        );
        assert_eq!(
            serde_json::to_string(&outcome.to_json()).unwrap(),
            serde_json::to_string(&json!({
                "member_id":member_id,"pane_id":"%2","flush_input_id":FLUSH,
                "flushed_input_ids":[4,5],"message_ids":[7,8],"remaining_input_ids":[],
                "delivery":{"status":"submitted","method":"forced","reason":null}
            }))
            .unwrap(),
            "JSON fields keep their documented order"
        );
        assert_eq!(
            outcome.text(),
            format!(
                "Flushed 2 queued previews to member {member_id} (input {FLUSH}): messages 7,8"
            )
        );
        assert!(h.lock_released());
    }

    #[test]
    fn a_partial_flush_reports_the_previews_still_queued() {
        let mut h = harness();
        let member_id = h.member_id();
        let outcome = h
            .flush(Case::replying(vec![
                reservation(&[4], &[7], &[5, 6]),
                settled(),
            ]))
            .unwrap();
        assert_eq!(outcome.to_json()["remaining_input_ids"], json!([5, 6]));
        assert_eq!(
            outcome.text().lines().collect::<Vec<_>>(),
            [
                format!(
                    "Flushed 1 queued previews to member {member_id} (input {FLUSH}): messages 7"
                ),
                "Still queued: 5,6".to_owned()
            ]
        );
    }

    #[test]
    fn an_empty_queue_writes_nothing_and_reports_no_previews() {
        let mut h = harness();
        let member_id = h.member_id();
        let outcome = h.flush(Case::replying(vec![FlushReply::Empty])).unwrap();
        assert_eq!(h.log(), ["target", "reserve"]);
        assert_eq!(
            serde_json::to_string(&outcome.to_json()).unwrap(),
            serde_json::to_string(&json!({
                "member_id":member_id,"pane_id":"%2","flush_input_id":null,
                "flushed_input_ids":[],"message_ids":[],"remaining_input_ids":[],
                "delivery":null
            }))
            .unwrap()
        );
        assert_eq!(
            outcome.text(),
            format!("No queued previews for member {member_id}")
        );
        assert!(h.lock_released());
    }

    #[test]
    fn a_runner_refusal_maps_to_the_not_flushed_error_without_writing() {
        let mut h = harness();
        let member_id = h.member_id();
        let cause = failure(h.flush(Case::replying(vec![FlushReply::Refused {
            cause: "the member's input is fenced".into(),
        }])));
        assert_eq!(cause.exit_code(), 1);
        assert_eq!(
            cause.message(),
            not_flushed(member_id, "the member's input is fenced")
        );
        assert_eq!(h.log(), ["target", "reserve"]);
        assert!(h.lock_released());
    }

    #[test]
    fn the_director_or_a_pending_placement_is_refused_before_any_process() {
        let mut h = harness();
        let director =
            crate::broker::active_member(&h.fixture.conn, h.fixture.run.director_member_id)
                .unwrap();
        let director_id = director.member_id;
        assert_eq!(
            failure(h.flush_as(director, Case::replying(vec![]))).message(),
            not_flushed(director_id, "cannot flush the Director's own pane")
        );
        let member_id = h.member_id();
        let mut pending = h.member();
        pending.placement.mux_pane_id = None;
        assert_eq!(
            failure(h.flush_as(pending, Case::replying(vec![]))).message(),
            not_flushed(
                member_id,
                &format!("member {member_id} has no pane yet (pending placement)")
            )
        );
        assert!(h.log().is_empty());
    }

    #[test]
    fn a_run_that_is_not_running_is_refused_before_any_process() {
        let mut h = harness();
        let member_id = h.member_id();
        h.fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='degraded' WHERE run_id=?1",
                [h.fixture.run.run_id],
            )
            .unwrap();
        assert_eq!(
            failure(h.flush(Case::replying(vec![]))).message(),
            not_flushed(member_id, "the member's run is degraded")
        );
        assert!(h.log().is_empty());
    }

    #[test]
    fn a_busy_pane_lock_refuses_the_flush_at_the_deadline() {
        let mut h = harness();
        let member_id = h.member_id();
        let _held = PaneLock::try_acquire(&h.settings.database_url, &h.member().placement)
            .unwrap()
            .unwrap();
        let mut case = Case::replying(vec![]);
        case.deadline = Duration::from_millis(300);
        assert_eq!(
            failure(h.flush(case)).message(),
            not_flushed(
                member_id,
                &format!("member {member_id}'s pane input remained busy")
            )
        );
        assert!(h.log().is_empty());
    }

    #[test]
    fn a_target_that_does_not_match_the_placement_is_refused_before_reserving() {
        let mut h = harness();
        let member_id = h.member_id();
        let mut case = Case::replying(vec![]);
        case.target = "%9|@1|main";
        assert_eq!(
            failure(h.flush(case)).message(),
            not_flushed(
                member_id,
                "press target context does not match the recorded placement"
            )
        );
        assert_eq!(h.log(), ["target"]);
        assert!(h.lock_released());
    }

    #[test]
    fn a_placement_change_after_reserving_settles_not_started_and_writes_nothing() {
        let mut h = harness();
        let member_id = h.member_id();
        let database_url = h.settings.database_url.clone();
        let mut case = Case::replying(vec![reservation(&[4], &[7], &[]), settled()]);
        case.on_reserve = Box::new(move || {
            let conn = crate::db::connect(&database_url).unwrap();
            assert_eq!(
                conn.execute(
                    "UPDATE member_placements SET coding_agent='claude' WHERE member_id=?1",
                    [member_id],
                )
                .unwrap(),
                1
            );
        });
        assert_eq!(
            failure(h.flush(case)).message(),
            not_flushed(
                member_id,
                &format!("member {member_id} placement changed before flush")
            )
        );
        assert_eq!(h.log(), ["target", "reserve", "settle:9:not_started"]);
        assert!(h.lock_released());
    }

    #[test]
    fn a_literal_write_that_never_started_settles_not_started() {
        let mut h = harness();
        let member_id = h.member_id();
        let mut case = Case::replying(vec![reservation(&[4], &[7], &[]), settled()]);
        case.text = Write::NotStarted;
        let cause = failure(h.flush(case));
        assert!(
            cause.message().starts_with(&not_flushed(member_id, "")),
            "{cause}"
        );
        assert_eq!(
            h.log(),
            ["target", "reserve", "text", "settle:9:not_started"]
        );
        assert!(h.lock_released());
    }

    #[test]
    fn a_started_failure_or_unreadable_acknowledgment_settles_uncertain() {
        for (text, keys, writes) in [
            (Write::StartedFailure, Write::Acknowledged, vec!["text"]),
            (Write::Unreadable, Write::Acknowledged, vec!["text"]),
            (
                Write::Acknowledged,
                Write::StartedFailure,
                vec!["text", "sleep:500", "keys:-t %2 Enter"],
            ),
        ] {
            let mut h = harness();
            let member_id = h.member_id();
            let mut case = Case::replying(vec![reservation(&[4], &[7], &[]), settled()]);
            case.text = text;
            case.keys = keys;
            let cause = failure(h.flush(case));
            let message = cause.message();
            assert!(
                message.starts_with(&format!(
                    "queued input flush to member {member_id} is uncertain: "
                )),
                "{message}"
            );
            assert!(
                message.ends_with(&format!(
                    " (input_id={FLUSH}). Inspect the pane, then reconcile input {FLUSH}."
                )),
                "{message}"
            );
            let mut expected_log = vec!["target".to_owned(), "reserve".to_owned()];
            expected_log.extend(writes.iter().map(|write| (*write).to_owned()));
            expected_log.push("settle:9:uncertain".into());
            assert_eq!(h.log(), expected_log);
            assert!(h.lock_released());
        }
    }

    #[test]
    fn a_settle_that_finds_the_flush_no_longer_outstanding_exits_with_its_durable_state() {
        for (state, disposition, expected) in [
            (
                InputState::Uncertain,
                None,
                "uncertain: the flush reservation expired before settlement",
            ),
            (
                InputState::Settled,
                Some(InputDisposition::Submitted),
                "submitted",
            ),
            (
                InputState::Settled,
                Some(InputDisposition::NotSubmitted),
                "not_submitted",
            ),
        ] {
            let mut h = harness();
            let member_id = h.member_id();
            let cause = failure(h.flush(Case::replying(vec![
                reservation(&[4], &[7], &[]),
                FlushReply::NotOutstanding {
                    flush_input_id: FLUSH,
                    state,
                    disposition,
                },
            ])));
            assert_eq!(cause.exit_code(), 1);
            let message = if state == InputState::Uncertain {
                uncertain(member_id, "the flush reservation expired before settlement")
            } else {
                format!(
                    "queued input flush to member {member_id} was already settled {expected} before the flush reported its write outcome (input_id={FLUSH})."
                )
            };
            assert_eq!(cause.message(), message, "{expected}");
            assert!(h.lock_released());
        }
    }
}
