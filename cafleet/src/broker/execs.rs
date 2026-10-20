//! Member execs (SPEC §6.2 *Member execs*): one row per `cafleet member
//! exec`, whose state is derived from its timestamps — queued, dispatched,
//! running, finished, closed.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::members::db_err;
use super::monitor::process_alive;
use super::records::{ExecMember, MemberExec};
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc, parse_lenient};

pub const EXEC_START_GRACE_SECONDS: i64 = 30;

const EXEC_COLUMNS: &str = "exec_id, member_id, command, created_at, dispatched_at, started_at, \
     pid, finished_at, exit_code, resumed_at";

fn map_exec(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemberExec> {
    Ok(MemberExec {
        exec_id: row.get(0)?,
        member_id: row.get(1)?,
        command: row.get(2)?,
        created_at: row.get(3)?,
        dispatched_at: row.get(4)?,
        started_at: row.get(5)?,
        pid: row.get(6)?,
        finished_at: row.get(7)?,
        exit_code: row.get(8)?,
        resumed_at: row.get(9)?,
    })
}

fn first_exec(
    conn: &Connection,
    member_id: i64,
    state: &str,
) -> Result<Option<MemberExec>, CafleetError> {
    conn.query_row(
        &format!(
            "SELECT {EXEC_COLUMNS} FROM member_execs \
             WHERE member_id=?1 AND {state} ORDER BY exec_id LIMIT 1"
        ),
        [member_id],
        map_exec,
    )
    .optional()
    .map_err(db_err)
}

/// Queue `command` for `member_id`; the stored body is verbatim.
pub fn queue_exec(
    conn: &mut Connection,
    member_id: i64,
    command: &str,
) -> Result<MemberExec, CafleetError> {
    conn.execute(
        "INSERT INTO member_execs (member_id, command, created_at) VALUES (?1, ?2, ?3)",
        params![member_id, command, format_utc(now_utc())],
    )
    .map_err(db_err)?;
    let exec_id = conn.last_insert_rowid();
    get_exec(conn, exec_id)?.ok_or_else(|| CafleetError::InvalidStoredValue {
        field: "member_execs.exec_id".into(),
        value: exec_id.to_string(),
    })
}

pub fn get_exec(conn: &Connection, exec_id: i64) -> Result<Option<MemberExec>, CafleetError> {
    conn.query_row(
        &format!("SELECT {EXEC_COLUMNS} FROM member_execs WHERE exec_id=?1"),
        [exec_id],
        map_exec,
    )
    .optional()
    .map_err(db_err)
}

/// The member an exec's notices name, read whatever the member's status.
pub fn exec_member(conn: &Connection, member_id: i64) -> Result<ExecMember, CafleetError> {
    conn.query_row(
        "SELECT member_id, fleet_id, name FROM members WHERE member_id=?1",
        [member_id],
        |row| {
            Ok(ExecMember {
                member_id: row.get(0)?,
                fleet_id: row.get(1)?,
                name: row.get(2)?,
            })
        },
    )
    .map_err(db_err)
}

/// Whether the pane has a dispatched or running exec.
pub fn exec_in_flight(conn: &Connection, member_id: i64) -> Result<bool, CafleetError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM member_execs \
             WHERE member_id=?1 AND dispatched_at IS NOT NULL AND finished_at IS NULL)",
        [member_id],
        |row| row.get(0),
    )
    .map_err(db_err)
}

pub fn oldest_finished_exec(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<MemberExec>, CafleetError> {
    first_exec(
        conn,
        member_id,
        "finished_at IS NOT NULL AND resumed_at IS NULL",
    )
}

pub fn oldest_queued_exec(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<MemberExec>, CafleetError> {
    first_exec(
        conn,
        member_id,
        "dispatched_at IS NULL AND finished_at IS NULL",
    )
}

/// The `finished_at` of the pane's latest finished exec.
pub fn latest_exec_finished_at(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<String>, CafleetError> {
    conn.query_row(
        "SELECT MAX(finished_at) FROM member_execs WHERE member_id=?1",
        [member_id],
        |row| row.get(0),
    )
    .map_err(db_err)
}

/// Stamp a queued exec as dispatched; `false` when it is no longer queued.
pub fn stamp_exec_dispatched(
    conn: &mut Connection,
    exec_id: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE member_execs SET dispatched_at=?1 \
             WHERE exec_id=?2 AND dispatched_at IS NULL AND finished_at IS NULL",
            params![when, exec_id],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

/// Return an exec to queued after its dispatch keystroke failed.
pub fn clear_exec_dispatched(conn: &mut Connection, exec_id: i64) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE member_execs SET dispatched_at=NULL WHERE exec_id=?1 AND started_at IS NULL",
        [exec_id],
    )
    .map_err(db_err)?;
    Ok(())
}

/// The `member exec-run` start claim: `true` iff this process now owns a
/// dispatched exec that nothing has started or finished.
pub fn start_exec(
    conn: &mut Connection,
    exec_id: i64,
    pid: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE member_execs SET started_at=?1, pid=?2 \
             WHERE exec_id=?3 AND dispatched_at IS NOT NULL AND started_at IS NULL \
               AND finished_at IS NULL",
            params![when, pid, exec_id],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

pub fn finish_exec(
    conn: &mut Connection,
    exec_id: i64,
    exit_code: i64,
    when: &str,
) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE member_execs SET finished_at=?1, exit_code=?2 WHERE exec_id=?3",
        params![when, exit_code, exec_id],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Close a finished exec; `false` when it is already closed.
pub fn stamp_exec_resumed(
    conn: &mut Connection,
    exec_id: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    let changed = conn
        .execute(
            "UPDATE member_execs SET resumed_at=?1 \
             WHERE exec_id=?2 AND finished_at IS NOT NULL AND resumed_at IS NULL",
            params![when, exec_id],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

/// Reopen an exec after its resume keystroke failed.
pub fn clear_exec_resumed(conn: &mut Connection, exec_id: i64) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE member_execs SET resumed_at=NULL WHERE exec_id=?1",
        [exec_id],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Every exec of the fleet that will never report an exit status: dispatched
/// but not started within the start grace, or running under a process that
/// is no longer alive.
pub fn lost_execs(
    conn: &Connection,
    fleet_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<MemberExec>, CafleetError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {EXEC_COLUMNS} FROM member_execs \
             WHERE dispatched_at IS NOT NULL AND finished_at IS NULL \
               AND member_id IN (SELECT member_id FROM members WHERE fleet_id=?1) \
             ORDER BY exec_id"
        ))
        .map_err(db_err)?;
    let in_flight = stmt
        .query_map([fleet_id], map_exec)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    let mut lost = Vec::new();
    for exec in in_flight {
        let is_lost = match exec.pid {
            Some(pid) => !process_alive(pid),
            None => {
                let dispatched_at = exec
                    .dispatched_at
                    .as_deref()
                    .expect("the selection requires dispatched_at");
                now - parse_lenient(dispatched_at)? >= Duration::seconds(EXEC_START_GRACE_SECONDS)
            }
        };
        if is_lost {
            lost.push(exec);
        }
    }
    Ok(lost)
}

/// Close a lost exec with no exit status.
pub fn close_lost_exec(
    conn: &mut Connection,
    exec_id: i64,
    when: &str,
) -> Result<(), CafleetError> {
    conn.execute(
        "UPDATE member_execs SET finished_at=?1, resumed_at=?1 \
         WHERE exec_id=?2 AND finished_at IS NULL",
        params![when, exec_id],
    )
    .map_err(db_err)?;
    Ok(())
}
