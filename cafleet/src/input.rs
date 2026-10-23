use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::broker::records::{MemberRecord, Placement};
use crate::config::Settings;
use crate::error::CafleetError;

#[derive(Clone, Copy)]
pub struct Deadline(Instant);

impl Deadline {
    pub fn new() -> Self {
        Self(Instant::now() + Duration::from_secs(5))
    }

    pub fn remaining(self) -> Result<Duration, CafleetError> {
        self.0
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| {
                CafleetError::App("pane input attempt exceeded its five-second deadline".into())
            })
    }

    pub fn retry(self) -> bool {
        match self.remaining() {
            Ok(left) => {
                std::thread::sleep(left.min(Duration::from_millis(50)));
                self.remaining().is_ok()
            }
            Err(_) => false,
        }
    }
}

impl Default for Deadline {
    fn default() -> Self {
        Self::new()
    }
}

// rusqlite's safe busy handler accepts a function pointer; these scoped values
// supply its deadline during synchronous operations on the borrowed connection.
thread_local! {
    static DATABASE_WAITS: std::cell::RefCell<Vec<(usize, Deadline)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn remaining_database_wait() -> Option<Duration> {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        assert!(
            !waits.is_empty(),
            "a database deadline handler has an active scope"
        );
        waits
            .iter()
            .try_fold(Duration::MAX, |remaining, (_, deadline)| {
                deadline.remaining().map(|left| remaining.min(left))
            })
            .ok()
    })
}

fn wait_for_database_deadline(_: i32) -> bool {
    let Some(remaining) = remaining_database_wait() else {
        return false;
    };
    std::thread::sleep(remaining.min(Duration::from_millis(10)));
    remaining_database_wait().is_some()
}

struct DatabaseBudget<'a> {
    conn: &'a mut Connection,
    identity: usize,
    previous_timeout: Duration,
    active: bool,
}

impl<'a> DatabaseBudget<'a> {
    fn new(conn: &'a mut Connection, deadline: Deadline) -> Result<Self, CafleetError> {
        deadline.remaining()?;
        let previous: u32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .map_err(crate::broker::members::db_err)?;
        let identity = std::ptr::from_ref(conn).addr();
        conn.busy_handler(Some(wait_for_database_deadline))
            .map_err(crate::broker::members::db_err)?;
        DATABASE_WAITS.with(|waits| waits.borrow_mut().push((identity, deadline)));
        Ok(Self {
            conn,
            identity,
            previous_timeout: Duration::from_millis(u64::from(previous)),
            active: true,
        })
    }

    fn restore(&mut self) -> Result<(), CafleetError> {
        if !self.active {
            return Ok(());
        }
        let nested = DATABASE_WAITS.with(|waits| {
            let waits = waits.borrow();
            assert_eq!(
                waits
                    .last()
                    .expect("an active database budget is registered")
                    .0,
                self.identity
            );
            waits
                .iter()
                .rev()
                .skip(1)
                .any(|(identity, _)| *identity == self.identity)
        });
        if nested {
            self.conn.busy_handler(Some(wait_for_database_deadline))
        } else {
            self.conn.busy_timeout(self.previous_timeout)
        }
        .map_err(crate::broker::members::db_err)?;
        DATABASE_WAITS.with(|waits| {
            waits
                .borrow_mut()
                .pop()
                .expect("the active database scope is registered");
        });
        self.active = false;
        Ok(())
    }
}

impl Drop for DatabaseBudget<'_> {
    fn drop(&mut self) {
        self.restore()
            .expect("an open borrowed connection accepts busy-handler restoration");
    }
}

pub fn with_database_budget<T>(
    conn: &mut Connection,
    deadline: Deadline,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    let mut budget = DatabaseBudget::new(conn, deadline)?;
    let result = operation(budget.conn);
    let restored = budget.restore();
    match (result, restored) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(primary), Err(cleanup)) => Err(CafleetError::App(format!(
            "{}; restoring database wait failed: {}",
            primary.message(),
            cleanup.message()
        ))),
    }
}

pub struct PaneLock {
    _file: File,
}

pub fn lock_path(database_url: &str, placement: &Placement) -> Result<PathBuf, CafleetError> {
    let pane = placement
        .mux_pane_id
        .as_deref()
        .ok_or_else(|| CafleetError::App("pane input requires a concrete placement".into()))?;
    let database =
        std::fs::canonicalize(crate::db::database_path(database_url)?).map_err(|error| {
            CafleetError::App(format!(
                "cannot resolve database input-lock identity: {error}"
            ))
        })?;
    let mut directory = database.into_os_string();
    directory.push(".pane-locks");
    let key = format!("{}\0{}\0{pane}", placement.backend, placement.mux_session);
    let key: String = Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(PathBuf::from(directory).join(format!("{key}.lock")))
}

impl PaneLock {
    pub fn try_acquire(
        database_url: &str,
        placement: &Placement,
    ) -> Result<Option<Self>, CafleetError> {
        let path = lock_path(database_url, placement)?;
        let directory = path
            .parent()
            .expect("a pane lock has its database-adjacent directory");
        std::fs::create_dir_all(directory).map_err(|error| {
            CafleetError::App(format!(
                "cannot create pane input-lock directory {}: {error}",
                directory.display()
            ))
        })?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                CafleetError::App(format!(
                    "cannot open pane input lock {}: {error}",
                    path.display()
                ))
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(CafleetError::App(format!(
                "cannot lock pane input {}: {error}",
                path.display()
            ))),
        }
    }
}

pub struct LockedMember {
    pub member: MemberRecord,
    _guard: PaneLock,
}

pub fn lock_member(
    conn: &Connection,
    settings: &Settings,
    member: &MemberRecord,
) -> Result<Option<LockedMember>, CafleetError> {
    let placement = member.placement.as_ref().ok_or_else(|| {
        CafleetError::App(format!("member {} has no placement row", member.member_id))
    })?;
    let Some(guard) =
        PaneLock::try_acquire(&settings.database_url, placement).map_err(|error| {
            CafleetError::App(format!(
                "member {} pane input: {}",
                member.member_id,
                error.message()
            ))
        })?
    else {
        return Ok(None);
    };
    let current = crate::broker::get_member(conn, member.member_id, member.fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {} not found", member.member_id)))?;
    if current.placement != member.placement {
        return Err(CafleetError::App(format!(
            "member {} placement changed while acquiring pane input",
            member.member_id
        )));
    }
    Ok(Some(LockedMember {
        member: current,
        _guard: guard,
    }))
}

#[derive(PartialEq, Eq)]
struct DeletionPlacement {
    member_id: i64,
    placement: Placement,
    path: PathBuf,
}

fn deletion_placements(
    conn: &Connection,
    settings: &Settings,
    fleet_id: i64,
) -> Result<Vec<DeletionPlacement>, CafleetError> {
    let mut statement = conn.prepare(
        "SELECT m.member_id, p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, p.created_at
         FROM members m JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL"
    ).map_err(crate::broker::members::db_err)?;
    let rows = statement
        .query_map([fleet_id], |row| {
            Ok((row.get::<_, i64>(0)?, Placement::from_row(row, 1)?))
        })
        .map_err(crate::broker::members::db_err)?;
    let mut placements = Vec::new();
    for row in rows {
        let (member_id, placement) = row.map_err(crate::broker::members::db_err)?;
        let path = lock_path(&settings.database_url, &placement).map_err(|error| {
            CafleetError::App(format!(
                "member {member_id} pane input: {}",
                error.message()
            ))
        })?;
        placements.push(DeletionPlacement {
            member_id,
            placement,
            path,
        });
    }
    placements
        .sort_by(|left, right| (&left.path, left.member_id).cmp(&(&right.path, right.member_id)));
    Ok(placements)
}

pub fn delete_fleet(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    deadline: Deadline,
) -> Result<serde_json::Value, CafleetError> {
    with_database_budget(conn, deadline, |conn| {
        crate::db::require_current(conn)?;
        crate::broker::fleets::fetch_fleet(conn, fleet_id)?
            .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
        Ok(())
    })?;
    loop {
        let snapshot = with_database_budget(conn, deadline, |conn| {
            deletion_placements(conn, settings, fleet_id)
        })?;
        let mut guards = Vec::new();
        let mut previous = None;
        for target in &snapshot {
            if previous == Some(&target.path) {
                continue;
            }
            let guard = loop {
                match PaneLock::try_acquire(&settings.database_url, &target.placement).map_err(
                    |error| {
                        CafleetError::App(format!(
                            "member {} pane input: {}",
                            target.member_id,
                            error.message()
                        ))
                    },
                )? {
                    Some(guard) => break guard,
                    None if deadline.retry() => {}
                    None => {
                        return Err(CafleetError::App(format!(
                            "fleet {fleet_id} pane input remained busy for 5 seconds at member {}; fleet was not deleted.",
                            target.member_id
                        )));
                    }
                }
            };
            guards.push(guard);
            previous = Some(&target.path);
        }
        let result = with_database_budget(conn, deadline, |conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(crate::broker::members::db_err)?;
            if deletion_placements(&tx, settings, fleet_id)? != snapshot {
                return Ok(None);
            }
            let result = crate::broker::fleets::delete_fleet_rows(&tx, fleet_id)?;
            deadline.remaining()?;
            tx.commit().map_err(crate::broker::members::db_err)?;
            Ok(Some(result))
        })?;
        if let Some(result) = result {
            return Ok(result);
        }
        drop(guards);
        deadline.remaining()?;
    }
}
