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
        (Err(cause), Ok(FlushReply::Settled { .. }))
            if outcome == FlushWriteOutcome::NotStarted =>
        {
            Err(refused(member_id, cause.message()))
        }
        (Err(cause), Err(settle)) if outcome == FlushWriteOutcome::NotStarted => Err(uncertain(
            member_id,
            format!(
                "{}; the runner did not confirm that the flush wrote nothing: {}",
                cause.message(),
                settle.message()
            ),
            flush_input_id,
        )),
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
