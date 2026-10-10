//! Held delivery (SPEC §6.5): at most one keystroke into one member's pane,
//! gated on the state the pane's capture classifies to.

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use crate::broker;
use crate::broker::records::{MemberExec, MessageRecord, PaneTarget};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{strip_ansi, truncate_text};
use crate::pane_state::{PaneState, classify};
use crate::time::{format_utc, parse_lenient};

pub const DELIVERY_CAPTURE_LINES: i64 = 40;
pub const EXEC_RESUME_GRACE_SECONDS: i64 = 10;

const RESUME_CLAUSE: &str = "[cafleet] Resume your work if something was still running.";
const BROKER_NOTE: &str = "[cafleet] This delivery's Escape dismissed a pending prompt in your pane; \
     that rejection did not come from the user. Re-issue the tool call if you still need it.";

/// What a delivery needs from the pane-hosting backend. `Err` carries the raw
/// backend error string.
pub trait PaneIo {
    fn capture_pane(&self, pane_id: &str, lines: i64) -> Result<String, String>;
    fn send_inline_preview(
        &self,
        pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
        note: Option<&str>,
    ) -> Result<(), String>;
    fn send_prompt(&self, pane_id: &str, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneOutcome {
    Fired { item: Item, forced: bool },
    Held(HoldReason),
    Idle,
}

impl PaneOutcome {
    /// Whether this outcome keystroked the preview of `message_id`.
    pub fn fired_preview(&self, message_id: i64) -> bool {
        matches!(
            self,
            PaneOutcome::Fired { item: Item::Preview { message_id: fired }, .. }
                if *fired == message_id
        )
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
    Unclassified,
    CaptureFailed(String),
    Busy,
    KeystrokeFailed(String),
}

enum Gate {
    Ordinary,
    Forced,
    Hold(HoldReason),
}

/// The gate table: a pane at rest always receives; any other state holds
/// until the hold age reaches the timeout, and a timeout of `0` never forces.
fn gate(state: PaneState, hold_age: Duration, hold_timeout: i64) -> Gate {
    let reason = match state {
        PaneState::Finished => return Gate::Ordinary,
        PaneState::AwaitingUser => HoldReason::AwaitingUser,
        PaneState::Working => HoldReason::Working,
        PaneState::Unclassified => HoldReason::Unclassified,
    };
    if hold_timeout > 0 && hold_age >= Duration::seconds(hold_timeout) {
        Gate::Forced
    } else {
        Gate::Hold(reason)
    }
}

/// The line a forced preview adds for the state the broker observed.
fn forced_note(state: PaneState) -> Option<&'static str> {
    match state {
        PaneState::AwaitingUser => Some(BROKER_NOTE),
        PaneState::Working => Some(RESUME_CLAUSE),
        PaneState::Finished | PaneState::Unclassified => None,
    }
}

/// The one item a pane is owed next.
enum Owed {
    Resume(MemberExec),
    Dispatch(MemberExec),
    Preview(MessageRecord),
}

impl Owed {
    /// A finished exec not yet resumed, else the oldest queued exec, else the
    /// oldest pending preview.
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

    /// When the item became owed: an exec's end for a resume, the row's
    /// creation otherwise.
    fn owed_since(&self) -> &str {
        match self {
            Owed::Resume(exec) => finished_at(exec),
            Owed::Dispatch(exec) => &exec.created_at,
            Owed::Preview(message) => &message.created_at,
        }
    }

    /// Stamp the item as keystroked; `false` when it is no longer owed,
    /// because it was ACKed or another process stamped it first.
    fn stamp(&self, conn: &mut Connection, when: &str) -> Result<bool, CafleetError> {
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

    fn send(
        &self,
        io: &dyn PaneIo,
        settings: &Settings,
        pane_id: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        match self {
            Owed::Resume(exec) => io.send_prompt(
                pane_id,
                &format!(
                    "[cafleet] exec {} finished with exit {}. \
                     Continue your work using its output above.",
                    exec.exec_id,
                    exec.exit_code
                        .expect("an exec that awaits its resume recorded an exit code"),
                ),
            ),
            Owed::Dispatch(exec) => io.send_prompt(
                pane_id,
                &format!("! cafleet member exec-run {}", exec.exec_id),
            ),
            Owed::Preview(message) => io.send_inline_preview(
                pane_id,
                message.message_id,
                message.from_member_id,
                &message.created_at,
                &truncate_text(Some(&message.text), settings.max_text_len)
                    .expect("a present text truncates to a present text"),
                note,
            ),
        }
    }
}

fn finished_at(exec: &MemberExec) -> &str {
    exec.finished_at
        .as_deref()
        .expect("a finished exec carries finished_at")
}

/// `now − max(item timestamp, finished_at of the pane's latest exec,
/// forced_at)`: the two pane-level terms restart the clock for everything
/// queued behind an exec or a forced delivery.
fn hold_age(
    conn: &Connection,
    target: &PaneTarget,
    owed: &Owed,
    now: DateTime<Utc>,
) -> Result<Duration, CafleetError> {
    let mut baseline = parse_lenient(owed.owed_since())?;
    let latest_exec_end = broker::latest_exec_finished_at(conn, target.member_id)?;
    for restart in [&latest_exec_end, &target.forced_at].into_iter().flatten() {
        baseline = baseline.max(parse_lenient(restart)?);
    }
    Ok(now - baseline)
}

/// Perform at most one keystroke into `member_id`'s pane.
pub fn deliver_pane(
    conn: &mut Connection,
    io: &dyn PaneIo,
    settings: &Settings,
    member_id: i64,
    now: DateTime<Utc>,
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

    let content = match io.capture_pane(&target.pane_id, DELIVERY_CAPTURE_LINES) {
        Ok(content) => content,
        Err(error) => return Ok(PaneOutcome::Held(HoldReason::CaptureFailed(error))),
    };
    let state = classify(&target.coding_agent, &strip_ansi(&content));

    if let Owed::Resume(exec) = &owed {
        if state == PaneState::Working {
            // The harness answered the command's output with a turn of its
            // own, so the exec closes without a keystroke.
            broker::stamp_exec_resumed(conn, exec.exec_id, &format_utc(now))?;
            return Ok(PaneOutcome::Held(HoldReason::Working));
        }
        if now - parse_lenient(finished_at(exec))? < Duration::seconds(EXEC_RESUME_GRACE_SECONDS) {
            return Ok(PaneOutcome::Held(HoldReason::ResumeGrace));
        }
    }

    let age = hold_age(conn, &target, &owed, now)?;
    let forced = match gate(state, age, settings.delivery_hold_timeout) {
        Gate::Ordinary => false,
        Gate::Forced => true,
        Gate::Hold(reason) => return Ok(PaneOutcome::Held(reason)),
    };

    let stamp = format_utc(now);
    // A refused claim means another cafleet process is typing into the pane;
    // a refused stamp means the item stopped being owed after it was selected.
    if !broker::claim_pane(conn, member_id, now)? || !owed.stamp(conn, &stamp)? {
        return Ok(PaneOutcome::Held(HoldReason::Busy));
    }
    if forced {
        broker::stamp_forced(conn, member_id, &stamp)?;
    }
    let note = if forced { forced_note(state) } else { None };
    match owed.send(io, settings, &target.pane_id, note) {
        Ok(()) => Ok(PaneOutcome::Fired {
            item: owed.item(),
            forced,
        }),
        Err(error) => {
            owed.clear_stamp(conn)?;
            Ok(PaneOutcome::Held(HoldReason::KeystrokeFailed(error)))
        }
    }
}
