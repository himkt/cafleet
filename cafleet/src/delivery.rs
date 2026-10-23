use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{strip_ansi, truncate_text};
use crate::pane_state::{PaneState, classify};
use crate::time::format_utc;
use chrono::{DateTime, Utc};
use rusqlite::Connection;

pub const DELIVERY_CAPTURE_LINES: i64 = 80;

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

pub fn deliver_pane(
    conn: &mut Connection,
    io: &dyn PaneIo,
    settings: &Settings,
    member_id: i64,
    now: DateTime<Utc>,
) -> Result<Option<i64>, CafleetError> {
    let Some(fleet_id) = broker::active_member_fleet(conn, member_id)? else {
        return Ok(None);
    };
    let member = broker::get_member(conn, member_id, fleet_id)?
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
    let Some(locked) = crate::input::lock_member(conn, settings, &member)? else {
        return Ok(None);
    };
    let Some(message) = broker::oldest_pending_preview(conn, member_id)? else {
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
        .map_err(CafleetError::App)?;
    if classify(&placement.coding_agent, &strip_ansi(&content)) != PaneState::Finished {
        return Ok(None);
    }
    let text =
        truncate_text(Some(&message.text), settings.max_text_len).expect("present message text");
    io.send_inline_preview(
        pane,
        message.message_id,
        message.from_member_id,
        &message.created_at,
        &text,
        None,
    )
    .map_err(CafleetError::App)?;
    Ok(
        broker::stamp_notified(conn, message.message_id, &format_utc(now))?
            .then_some(message.message_id),
    )
}
