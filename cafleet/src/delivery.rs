use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, PaneLock, with_database_budget};
use crate::multiplexer::{AnyMultiplexer, Multiplexer, MultiplexerError};
use crate::output::truncate_text;
use crate::pane_state::{PaneState, classify};
use crate::receive::{AttemptOutcome, Delivery, NotificationError};
use crate::time::format_utc;
use rusqlite::Connection;

pub const DELIVERY_CAPTURE_LINES: i64 = 80;

pub(crate) fn attempt_preview(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
) -> Result<AttemptOutcome, CafleetError> {
    let Some(fleet_id) = with_database_budget(conn, deadline, |conn| {
        broker::active_member_fleet(conn, member_id)
    })?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    if expected_fleet.is_some_and(|expected| expected != fleet_id) {
        return Ok(AttemptOutcome::NoPending);
    }
    let member = with_database_budget(conn, deadline, |conn| {
        broker::get_member(conn, member_id, fleet_id)
    })?;
    let Some(member) = member else {
        return Ok(AttemptOutcome::NoPending);
    };
    let Some(placement) = member
        .placement
        .as_ref()
        .filter(|p| p.mux_pane_id.is_some())
    else {
        return Ok(AttemptOutcome::Held);
    };
    if member.kind == broker::records::MemberKind::Director {
        return Ok(AttemptOutcome::NoPending);
    }
    let Some(initial) = with_database_budget(conn, deadline, |conn| {
        broker::oldest_pending_preview(conn, member_id)
    })?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    let failed = |message_id, delivery, error| {
        AttemptOutcome::Failed(NotificationError {
            member_id,
            message_id,
            delivery,
            error,
        })
    };
    let _guard = match PaneLock::try_acquire(&settings.database_url, placement) {
        Ok(Some(guard)) => guard,
        Ok(None) => return Ok(AttemptOutcome::Held),
        Err(error) => {
            return Ok(failed(
                initial.message_id,
                Delivery::NotSent,
                format!("member {member_id} pane input: {}", error.message()),
            ));
        }
    };
    let current = with_database_budget(conn, deadline, |conn| {
        broker::get_member(conn, member_id, fleet_id)
    })?;
    let Some(current) = current else {
        return Ok(AttemptOutcome::NoPending);
    };
    if current.placement != member.placement {
        return Ok(AttemptOutcome::Held);
    }
    let Some(message) = with_database_budget(conn, deadline, |conn| {
        broker::oldest_pending_preview(conn, member_id)
    })?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    if message.from_member_id == member_id {
        return Ok(AttemptOutcome::Held);
    }
    let mux = match resolve().and_then(|mux| {
        mux.ensure_available()?;
        Ok(mux.bounded(deadline))
    }) {
        Ok(mux) => mux,
        Err(error) => {
            return Ok(failed(
                message.message_id,
                Delivery::NotSent,
                error.to_string(),
            ));
        }
    };
    let pane = placement
        .mux_pane_id
        .as_deref()
        .expect("concrete preview placement");
    let content = match mux.capture_pane(pane, DELIVERY_CAPTURE_LINES) {
        Ok(content) => content,
        Err(error) => {
            return Ok(failed(
                message.message_id,
                Delivery::NotSent,
                error.to_string(),
            ));
        }
    };
    let state = classify(&placement.coding_agent, &content);
    if let Some(reason) = state.hold_reason() {
        return Ok(if matches!(state, PaneState::Draft | PaneState::Unknown) {
            failed(
                message.message_id,
                Delivery::NotSent,
                format!(
                    "member {member_id} is {reason}; preview {} was not sent.",
                    message.message_id
                ),
            )
        } else {
            AttemptOutcome::Held
        });
    }
    let hold = with_database_budget(conn, deadline, |conn| {
        let Some(current) = broker::get_member(conn, member_id, fleet_id)? else {
            return Ok(Some(AttemptOutcome::NoPending));
        };
        if !broker::messaging::is_preview_pending(conn, message.message_id)? {
            return Ok(Some(AttemptOutcome::Settled {
                message_id: message.message_id,
            }));
        }
        if current.placement != member.placement {
            return Ok(Some(AttemptOutcome::Held));
        }
        let Some(pending) = broker::oldest_pending_preview(conn, member_id)? else {
            return Ok(Some(AttemptOutcome::NoPending));
        };
        if pending.message_id != message.message_id {
            return Ok(Some(AttemptOutcome::Held));
        }
        Ok(None)
    })?;
    if let Some(hold) = hold {
        return Ok(hold);
    }
    let text =
        truncate_text(Some(&message.text), settings.max_text_len).expect("present message text");
    let uncertain = |cause| {
        failed(
            message.message_id,
            Delivery::Uncertain,
            format!(
                "preview {} delivery to member {member_id} is uncertain: {cause}. Inspect the pane before retrying.",
                message.message_id
            ),
        )
    };
    if let Err(error) = mux.send_inline_preview(
        pane,
        message.message_id,
        message.from_member_id,
        &message.created_at,
        &text,
    ) {
        return Ok(if error.submission_started() {
            uncertain(error.to_string())
        } else {
            failed(message.message_id, Delivery::NotSent, error.to_string())
        });
    }
    match with_database_budget(conn, deadline, |conn| {
        broker::stamp_notified(
            conn,
            message.message_id,
            &format_utc(crate::time::now_utc()),
        )
    }) {
        Ok(true) => Ok(AttemptOutcome::Sent {
            message_id: message.message_id,
        }),
        Ok(false) => Ok(AttemptOutcome::Settled {
            message_id: message.message_id,
        }),
        Err(error) => Ok(uncertain(error.message().into_owned())),
    }
}
