use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, with_database_budget};
use crate::multiplexer::Multiplexer;
use crate::output::truncate_text;
use crate::pane_state::{PaneState, classify};
use crate::time::format_utc;
use rusqlite::Connection;

pub const DELIVERY_CAPTURE_LINES: i64 = 80;

pub fn deliver_pane(
    conn: &mut Connection,
    io: &dyn Multiplexer,
    settings: &Settings,
    member_id: i64,
    deadline: Deadline,
) -> Result<Option<i64>, CafleetError> {
    let Some(fleet_id) = with_database_budget(conn, deadline, |conn| {
        broker::active_member_fleet(conn, member_id)
    })?
    else {
        return Ok(None);
    };
    let member = with_database_budget(conn, deadline, |conn| {
        broker::get_member(conn, member_id, fleet_id)
    })?
    .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    if member.kind == broker::records::MemberKind::Director
        || member
            .placement
            .as_ref()
            .and_then(|p| p.mux_pane_id.as_ref())
            .is_none()
    {
        return Ok(None);
    }
    let Some(locked) = with_database_budget(conn, deadline, |conn| {
        crate::input::lock_member(conn, settings, &member)
    })?
    else {
        return Ok(None);
    };
    let Some(message) = with_database_budget(conn, deadline, |conn| {
        broker::oldest_pending_preview(conn, member_id)
    })?
    else {
        return Ok(None);
    };
    if message.from_member_id == member_id {
        return Ok(None);
    }
    let placement = locked
        .member
        .placement
        .as_ref()
        .expect("locked concrete placement");
    let pane = placement
        .mux_pane_id
        .as_deref()
        .expect("locked concrete pane");
    let content = io
        .capture_pane(pane, DELIVERY_CAPTURE_LINES)
        .map_err(|error| CafleetError::App(error.to_string()))?;
    let state = classify(&placement.coding_agent, &content);
    if let Some(reason) = state.hold_reason() {
        return if matches!(state, PaneState::Draft | PaneState::Unknown) {
            Err(CafleetError::App(format!(
                "member {member_id} is {reason}; preview {} was not sent.",
                message.message_id
            )))
        } else {
            Ok(None)
        };
    }
    let eligible = with_database_budget(conn, deadline, |conn| {
        let current = broker::get_member(conn, member_id, fleet_id)?;
        Ok(current
            .as_ref()
            .is_some_and(|current| current.placement == locked.member.placement)
            && broker::oldest_pending_preview(conn, member_id)?
                .is_some_and(|pending| pending.message_id == message.message_id))
    })?;
    if !eligible {
        return Ok(None);
    }
    let text =
        truncate_text(Some(&message.text), settings.max_text_len).expect("present message text");
    let uncertain = |cause: String| {
        CafleetError::App(format!(
            "preview {} delivery to member {member_id} is uncertain: {cause}. Inspect the pane before retrying.",
            message.message_id
        ))
    };
    io.send_inline_preview(
        pane,
        message.message_id,
        message.from_member_id,
        &message.created_at,
        &text,
        None,
    )
    .map_err(|error| {
        if error.submission_started() {
            uncertain(error.to_string())
        } else {
            CafleetError::App(error.to_string())
        }
    })?;
    let stamped = with_database_budget(conn, deadline, |conn| {
        broker::stamp_notified(
            conn,
            message.message_id,
            &format_utc(crate::time::now_utc()),
        )
    })
    .map_err(|error| uncertain(error.message().into_owned()))?;
    Ok(stamped.then_some(message.message_id))
}
