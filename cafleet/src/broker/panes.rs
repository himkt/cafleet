//! Pane keystroke state (SPEC §6.2): pane claims and silence-notice stamps on
//! `member_placements`, and the owed-work selections the delivery callers read.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::members::db_err;
use super::records::{PaneTarget, SilentMember};
use crate::error::CafleetError;
use crate::time::{Clock, format_utc};

pub const KEYSTROKE_SPACING_SECONDS: i64 = 3;

pub struct PaneClaim {
    member_id: i64,
    keystroke_at: String,
    claimed_utc: DateTime<Utc>,
    acquired_at: std::time::Instant,
}

pub fn claim_automatic_pane(
    conn: &mut Connection,
    member_id: i64,
    clock: &dyn Clock,
) -> Result<Option<PaneClaim>, CafleetError> {
    let claimed_utc = clock.utc_now();
    let keystroke_at = format_utc(claimed_utc);
    let threshold = format_utc(claimed_utc - Duration::seconds(KEYSTROKE_SPACING_SECONDS));
    let acquired_at = clock.monotonic_now();
    let changed = conn
        .execute(
            "UPDATE member_placements SET keystroke_at=?1 \
         WHERE member_id=?2 AND (keystroke_at IS NULL OR keystroke_at <= ?3)",
            params![keystroke_at, member_id, threshold],
        )
        .map_err(db_err)?;
    Ok((changed == 1).then_some(PaneClaim {
        member_id,
        keystroke_at,
        claimed_utc,
        acquired_at,
    }))
}

pub fn validate_automatic_claim(
    tx: &Transaction<'_>,
    claim: &PaneClaim,
    clock: &dyn Clock,
) -> Result<bool, CafleetError> {
    let stored: Option<Option<String>> = tx
        .query_row(
            "SELECT keystroke_at FROM member_placements WHERE member_id=?1",
            [claim.member_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if stored.flatten().as_deref() != Some(claim.keystroke_at.as_str()) {
        return Ok(false);
    }
    let now = clock.utc_now();
    let elapsed = clock
        .monotonic_now()
        .checked_duration_since(claim.acquired_at);
    Ok(now >= claim.claimed_utc
        && now < claim.claimed_utc + Duration::seconds(KEYSTROKE_SPACING_SECONDS)
        && elapsed.is_some_and(|duration| {
            duration
                < std::time::Duration::from_secs(
                    u64::try_from(KEYSTROKE_SPACING_SECONDS)
                        .expect("keystroke spacing is nonnegative"),
                )
        }))
}

const PANE_TARGET_SELECT: &str = "SELECT m.member_id, m.name, p.mux_pane_id, p.coding_agent \
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
