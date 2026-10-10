//! Pane keystroke state (SPEC §6.2): the pane claim, the forced-delivery and
//! silence-notice stamps on `member_placements`, and the owed-work selections
//! the delivery callers read.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::members::db_err;
use super::records::{PaneTarget, SilentMember};
use crate::error::CafleetError;
use crate::time::format_utc;

pub const KEYSTROKE_SPACING_SECONDS: i64 = 3;

const PANE_TARGET_SELECT: &str = "SELECT m.member_id, m.name, p.mux_pane_id, p.coding_agent, \
            p.forced_at \
     FROM members m JOIN member_placements p ON p.member_id=m.member_id \
     WHERE m.status='active' AND p.mux_pane_id IS NOT NULL";

/// A pending preview or an exec that is not yet closed.
const HAS_OWED_WORK: &str = "(EXISTS(SELECT 1 FROM messages g \
     WHERE g.owner_member_id=m.member_id AND g.type='unicast' \
       AND g.status_state='input_required' AND g.notified_at IS NULL) \
     OR EXISTS(SELECT 1 FROM member_execs e \
     WHERE e.member_id=m.member_id AND e.resumed_at IS NULL))";

fn map_pane_target(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaneTarget> {
    Ok(PaneTarget {
        member_id: row.get(0)?,
        name: row.get(1)?,
        pane_id: row.get(2)?,
        coding_agent: row.get(3)?,
        forced_at: row.get(4)?,
    })
}

/// The pane of an active member; `None` when the member is not active or
/// owns no pane.
pub fn pane_target(conn: &Connection, member_id: i64) -> Result<Option<PaneTarget>, CafleetError> {
    conn.query_row(
        &format!("{PANE_TARGET_SELECT} AND m.member_id=?1"),
        [member_id],
        map_pane_target,
    )
    .optional()
    .map_err(db_err)
}

/// Every active pane-owning member of the fleet that has owed work, the
/// Director included, ordered by `member_id`.
pub fn owed_pane_targets(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<PaneTarget>, CafleetError> {
    let mut stmt = conn
        .prepare(&format!(
            "{PANE_TARGET_SELECT} AND m.fleet_id=?1 AND {HAS_OWED_WORK} \
             ORDER BY m.member_id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], map_pane_target)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn fleet_has_owed_work(conn: &Connection, fleet_id: i64) -> Result<bool, CafleetError> {
    conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM members m \
                 JOIN member_placements p ON p.member_id=m.member_id \
                 WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL \
                   AND {HAS_OWED_WORK})"
        ),
        [fleet_id],
        |row| row.get(0),
    )
    .map_err(db_err)
}

/// Claim the member's pane for one keystroke. `false` means another cafleet
/// process claimed it within the spacing window and is typing into it.
pub fn claim_pane(
    conn: &mut Connection,
    member_id: i64,
    now: DateTime<Utc>,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE member_placements SET keystroke_at=?1 \
             WHERE member_id=?2 AND (keystroke_at IS NULL OR keystroke_at <= ?3)",
            params![
                format_utc(now),
                member_id,
                format_utc(now - Duration::seconds(KEYSTROKE_SPACING_SECONDS)),
            ],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

pub fn stamp_forced(conn: &mut Connection, member_id: i64, when: &str) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE member_placements SET forced_at=?1 WHERE member_id=?2",
        params![when, member_id],
    )
    .map_err(db_err)?;
    Ok(())
}

/// The ready-watchdog candidates: every active non-Director member of the
/// fleet that owns a pane, has never sent a message, and has not been
/// reported. The caller applies the grace period to `spawned_at`.
pub fn unreported_silent_members(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<SilentMember>, CafleetError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.member_id, m.name, p.created_at \
             FROM members m JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL \
               AND p.silence_notice_at IS NULL \
               AND NOT EXISTS(SELECT 1 FROM fleets f \
                              WHERE f.fleet_id=m.fleet_id \
                                AND f.director_member_id=m.member_id) \
               AND NOT EXISTS(SELECT 1 FROM messages g WHERE g.from_member_id=m.member_id) \
             ORDER BY m.member_id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([fleet_id], |row| {
            Ok(SilentMember {
                member_id: row.get(0)?,
                name: row.get(1)?,
                spawned_at: row.get(2)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn stamp_silence_notice(
    conn: &mut Connection,
    member_id: i64,
    when: &str,
) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE member_placements SET silence_notice_at=?1 WHERE member_id=?2",
        params![when, member_id],
    )
    .map_err(db_err)?;
    Ok(())
}
