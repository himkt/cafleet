use chrono::Duration;
use rusqlite::{Connection, TransactionBehavior};

use crate::broker;
use crate::broker::members::db_err;
use crate::broker::records::{MemberExec, MessageRecord, PaneTarget};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{strip_ansi, truncate_text};
use crate::pane_state::{PaneState, classify};
use crate::time::{Clock, format_utc, parse_lenient};

pub const DELIVERY_CAPTURE_LINES: i64 = 40;
pub const EXEC_RESUME_GRACE_SECONDS: i64 = 10;

pub trait PaneIo {
    fn capture_pane(&self, pane_id: &str, lines: i64) -> Result<String, String>;
    fn send_inline_preview(
        &self,
        pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), String>;
    fn send_automatic_prompt(&self, pane_id: &str, text: &str) -> Result<(), String>;
    fn send_automatic_trigger(&self, pane_id: &str, payload: &str) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneOutcome {
    Fired { item: Item },
    Held(HoldReason),
    Idle,
}

impl PaneOutcome {
    pub fn fired_preview(&self, message_id: i64) -> bool {
        matches!(self, PaneOutcome::Fired { item: Item::Preview { message_id: fired } } if *fired == message_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Preview { message_id: i64 },
    ExecDispatch { exec_id: i64 },
    ExecResume { exec_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    ExecRunning,
    ResumeGrace,
    AwaitingUser,
    Working,
    Draft,
    Unclassified,
    CaptureFailed(String),
    Busy,
    KeystrokeFailed(String),
}

pub enum PingOutcome {
    Sent,
    Held(HoldReason),
}

pub fn observe_pane(io: &dyn PaneIo, pane_id: &str, coding_agent: &str) -> Result<(), HoldReason> {
    let content = io
        .capture_pane(pane_id, DELIVERY_CAPTURE_LINES)
        .map_err(HoldReason::CaptureFailed)?;
    match classify(coding_agent, &strip_ansi(&content)) {
        PaneState::Finished => Ok(()),
        PaneState::AwaitingUser => Err(HoldReason::AwaitingUser),
        PaneState::Working => Err(HoldReason::Working),
        PaneState::Draft => Err(HoldReason::Draft),
        PaneState::Unclassified => Err(HoldReason::Unclassified),
    }
}

pub fn claim_and_recheck(
    conn: &mut Connection,
    io: &dyn PaneIo,
    clock: &dyn Clock,
    target: &PaneTarget,
) -> Result<Result<broker::PaneClaim, HoldReason>, CafleetError> {
    let Some(claim) = broker::claim_automatic_pane(conn, target.member_id, clock)? else {
        return Ok(Err(HoldReason::Busy));
    };
    Ok(observe_pane(io, &target.pane_id, &target.coding_agent).map(|()| claim))
}

fn ping_hold(reason: HoldReason) -> Result<PingOutcome, CafleetError> {
    match reason {
        HoldReason::CaptureFailed(error) => {
            Err(CafleetError::App(format!("capture failed: {error}")))
        }
        reason => Ok(PingOutcome::Held(reason)),
    }
}

pub fn ping_pane(
    conn: &mut Connection,
    io: &dyn PaneIo,
    clock: &dyn Clock,
    target: &PaneTarget,
) -> Result<PingOutcome, CafleetError> {
    let payload = format!(
        "cafleet message poll {} — then resume your work if something was still running.",
        target.member_id,
    );
    if broker::exec_in_flight(conn, target.member_id)? {
        return Ok(PingOutcome::Held(HoldReason::ExecRunning));
    }
    if let Err(reason) = observe_pane(io, &target.pane_id, &target.coding_agent) {
        return ping_hold(reason);
    }
    let claim = match claim_and_recheck(conn, io, clock, target)? {
        Ok(claim) => claim,
        Err(reason) => return ping_hold(reason),
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    if !broker::validate_automatic_claim(&tx, &claim, clock)? {
        return Ok(PingOutcome::Held(HoldReason::Busy));
    }
    tx.commit().map_err(db_err)?;
    if !io.send_automatic_trigger(&target.pane_id, &payload) {
        return Err(CafleetError::App(format!(
            "send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane {}.",
            target.pane_id,
        )));
    }
    Ok(PingOutcome::Sent)
}

enum Owed {
    Resume(MemberExec),
    Dispatch(MemberExec),
    Preview(MessageRecord),
}

impl Owed {
    fn select(conn: &Connection, member_id: i64) -> Result<Option<Owed>, CafleetError> {
        if let Some(exec) = broker::oldest_finished_exec(conn, member_id)? {
            return Ok(Some(Owed::Resume(exec)));
        }
        if let Some(exec) = broker::oldest_queued_exec(conn, member_id)? {
            return Ok(Some(Owed::Dispatch(exec)));
        }
        Ok(broker::oldest_pending_preview(conn, member_id)?.map(Owed::Preview))
    }

    fn item(&self) -> Item {
        match self {
            Owed::Resume(exec) => Item::ExecResume {
                exec_id: exec.exec_id,
            },
            Owed::Dispatch(exec) => Item::ExecDispatch {
                exec_id: exec.exec_id,
            },
            Owed::Preview(message) => Item::Preview {
                message_id: message.message_id,
            },
        }
    }

    fn stamp(&self, conn: &Connection, when: &str) -> Result<bool, CafleetError> {
        match self {
            Owed::Resume(exec) => broker::stamp_exec_resumed(conn, exec.exec_id, when),
            Owed::Dispatch(exec) => broker::stamp_exec_dispatched(conn, exec.exec_id, when),
            Owed::Preview(message) => broker::stamp_notified(conn, message.message_id, when),
        }
    }

    fn clear_stamp(&self, conn: &mut Connection) -> Result<(), CafleetError> {
        match self {
            Owed::Resume(exec) => broker::clear_exec_resumed(conn, exec.exec_id),
            Owed::Dispatch(exec) => broker::clear_exec_dispatched(conn, exec.exec_id),
            Owed::Preview(message) => broker::clear_notified(conn, message.message_id),
        }
    }

    fn prepare(&self, settings: &Settings) -> PreparedDelivery<'_> {
        match self {
            Owed::Resume(exec) => PreparedDelivery::Prompt(format!(
                "[cafleet] exec {} finished with exit {}. Continue your work using its output above.",
                exec.exec_id,
                exec.exit_code
                    .expect("an exec that awaits its resume recorded an exit code"),
            )),
            Owed::Dispatch(exec) => {
                PreparedDelivery::Prompt(format!("! cafleet member exec-run {}", exec.exec_id))
            }
            Owed::Preview(message) => PreparedDelivery::Preview {
                message,
                text: truncate_text(Some(&message.text), settings.max_text_len)
                    .expect("a present text truncates to a present text"),
            },
        }
    }
}

enum PreparedDelivery<'a> {
    Prompt(String),
    Preview {
        message: &'a MessageRecord,
        text: String,
    },
}

impl PreparedDelivery<'_> {
    fn send(&self, io: &dyn PaneIo, pane_id: &str) -> Result<(), String> {
        match self {
            Self::Prompt(text) => io.send_automatic_prompt(pane_id, text),
            Self::Preview { message, text } => io.send_inline_preview(
                pane_id,
                message.message_id,
                message.from_member_id,
                &message.created_at,
                text,
            ),
        }
    }
}

pub fn deliver_pane(
    conn: &mut Connection,
    io: &dyn PaneIo,
    settings: &Settings,
    member_id: i64,
    clock: &dyn Clock,
) -> Result<PaneOutcome, CafleetError> {
    let Some(target) = broker::pane_target(conn, member_id)? else {
        return Ok(PaneOutcome::Idle);
    };
    if broker::exec_in_flight(conn, member_id)? {
        return Ok(PaneOutcome::Held(HoldReason::ExecRunning));
    }
    let Some(owed) = Owed::select(conn, member_id)? else {
        return Ok(PaneOutcome::Idle);
    };
    let prepared = owed.prepare(settings);
    let observation = observe_pane(io, &target.pane_id, &target.coding_agent);
    if let Err(HoldReason::CaptureFailed(error)) = &observation {
        return Ok(PaneOutcome::Held(HoldReason::CaptureFailed(error.clone())));
    }
    if let Owed::Resume(exec) = &owed {
        if observation == Err(HoldReason::Working) {
            broker::stamp_exec_resumed(conn, exec.exec_id, &format_utc(clock.utc_now()))?;
            return Ok(PaneOutcome::Held(HoldReason::Working));
        }
        let end = exec
            .finished_at
            .as_deref()
            .expect("a finished exec carries finished_at");
        if clock.utc_now() - parse_lenient(end)? < Duration::seconds(EXEC_RESUME_GRACE_SECONDS) {
            return Ok(PaneOutcome::Held(HoldReason::ResumeGrace));
        }
    }
    if let Err(reason) = observation {
        return Ok(PaneOutcome::Held(reason));
    }
    let claim = match claim_and_recheck(conn, io, clock, &target)? {
        Ok(claim) => claim,
        Err(reason) => return Ok(PaneOutcome::Held(reason)),
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    if !broker::validate_automatic_claim(&tx, &claim, clock)?
        || !owed.stamp(&tx, &format_utc(clock.utc_now()))?
    {
        return Ok(PaneOutcome::Held(HoldReason::Busy));
    }
    tx.commit().map_err(db_err)?;
    match prepared.send(io, &target.pane_id) {
        Ok(()) => Ok(PaneOutcome::Fired { item: owed.item() }),
        Err(error) => {
            owed.clear_stamp(conn)?;
            Ok(PaneOutcome::Held(HoldReason::KeystrokeFailed(error)))
        }
    }
}
