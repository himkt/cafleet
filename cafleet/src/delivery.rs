//! Held delivery (SPEC §6.5): at most one keystroke into one member's pane,
//! gated on the state the pane's capture classifies to.

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use crate::broker;
use crate::broker::records::{MessageRecord, PaneTarget};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{strip_ansi, truncate_text};
use crate::pane_state::{PaneState, classify};
use crate::time::{format_utc, parse_lenient};

pub const DELIVERY_CAPTURE_LINES: i64 = 40;

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

fn hold_age(
    target: &PaneTarget,
    item_timestamp: &str,
    now: DateTime<Utc>,
) -> Result<Duration, CafleetError> {
    let mut baseline = parse_lenient(item_timestamp)?;
    if let Some(forced_at) = &target.forced_at {
        baseline = baseline.max(parse_lenient(forced_at)?);
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
    let Some(message) = broker::oldest_pending_preview(conn, member_id)? else {
        return Ok(PaneOutcome::Idle);
    };

    let content = match io.capture_pane(&target.pane_id, DELIVERY_CAPTURE_LINES) {
        Ok(content) => content,
        Err(error) => return Ok(PaneOutcome::Held(HoldReason::CaptureFailed(error))),
    };
    let state = classify(&target.coding_agent, &strip_ansi(&content));
    let age = hold_age(&target, &message.created_at, now)?;
    let forced = match gate(state, age, settings.delivery_hold_timeout) {
        Gate::Ordinary => false,
        Gate::Forced => true,
        Gate::Hold(reason) => return Ok(PaneOutcome::Held(reason)),
    };

    fire_preview(conn, io, settings, &target, &message, state, forced, now)
}

#[allow(clippy::too_many_arguments)]
fn fire_preview(
    conn: &mut Connection,
    io: &dyn PaneIo,
    settings: &Settings,
    target: &PaneTarget,
    message: &MessageRecord,
    state: PaneState,
    forced: bool,
    now: DateTime<Utc>,
) -> Result<PaneOutcome, CafleetError> {
    let stamp = format_utc(now);
    // A refused claim means another cafleet process is typing into the pane;
    // a refused stamp means the row was ACKed after it was selected.
    if !broker::claim_pane(conn, target.member_id, now)?
        || !broker::stamp_notified(conn, message.message_id, &stamp)?
    {
        return Ok(PaneOutcome::Held(HoldReason::Busy));
    }
    if forced {
        broker::stamp_forced(conn, target.member_id, &stamp)?;
    }

    let text = truncate_text(Some(&message.text), settings.max_text_len)
        .expect("a present text truncates to a present text");
    let note = if forced { forced_note(state) } else { None };
    match io.send_inline_preview(
        &target.pane_id,
        message.message_id,
        message.from_member_id,
        &message.created_at,
        &text,
        note,
    ) {
        Ok(()) => Ok(PaneOutcome::Fired {
            item: Item::Preview {
                message_id: message.message_id,
            },
            forced,
        }),
        Err(error) => {
            broker::clear_notified(conn, message.message_id)?;
            Ok(PaneOutcome::Held(HoldReason::KeystrokeFailed(error)))
        }
    }
}
