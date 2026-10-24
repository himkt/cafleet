use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::broker::records::{MemberRecord, Placement};
use crate::config::Settings;
use crate::error::CafleetError;

#[derive(Debug, Clone, Copy)]
pub struct Deadline(Instant);

impl Deadline {
    pub fn new() -> Self {
        Self::until(Instant::now() + Duration::from_secs(5))
    }

    pub(crate) fn until(instant: Instant) -> Self {
        Self(instant)
    }

    pub fn remaining(self) -> Result<Duration, CafleetError> {
        self.remaining_at(Instant::now())
    }

    pub(crate) fn remaining_at(self, now: Instant) -> Result<Duration, CafleetError> {
        self.0
            .checked_duration_since(now)
            .filter(|left| !left.is_zero())
            .ok_or(CafleetError::DeadlineExceeded)
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

struct DatabaseWait {
    identity: usize,
    deadline: Deadline,
    handler: usize,
}

thread_local! {
    static DATABASE_WAITS: std::cell::RefCell<Vec<DatabaseWait>> = const { std::cell::RefCell::new(Vec::new()) };
}

const DATABASE_HANDLERS: &[fn(i32) -> bool] = &[
    wait_for_database_deadline::<0>,
    wait_for_database_deadline::<1>,
    wait_for_database_deadline::<2>,
    wait_for_database_deadline::<3>,
    wait_for_database_deadline::<4>,
    wait_for_database_deadline::<5>,
    wait_for_database_deadline::<6>,
    wait_for_database_deadline::<7>,
    wait_for_database_deadline::<8>,
    wait_for_database_deadline::<9>,
    wait_for_database_deadline::<10>,
    wait_for_database_deadline::<11>,
    wait_for_database_deadline::<12>,
    wait_for_database_deadline::<13>,
    wait_for_database_deadline::<14>,
    wait_for_database_deadline::<15>,
    wait_for_database_deadline::<16>,
    wait_for_database_deadline::<17>,
    wait_for_database_deadline::<18>,
    wait_for_database_deadline::<19>,
    wait_for_database_deadline::<20>,
    wait_for_database_deadline::<21>,
    wait_for_database_deadline::<22>,
    wait_for_database_deadline::<23>,
    wait_for_database_deadline::<24>,
    wait_for_database_deadline::<25>,
    wait_for_database_deadline::<26>,
    wait_for_database_deadline::<27>,
    wait_for_database_deadline::<28>,
    wait_for_database_deadline::<29>,
    wait_for_database_deadline::<30>,
    wait_for_database_deadline::<31>,
];

fn remaining_database_wait(handler: usize) -> Option<Duration> {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        assert!(
            waits.iter().any(|wait| wait.handler == handler),
            "a busy handler has an owning connection scope"
        );
        waits
            .iter()
            .filter(|wait| wait.handler == handler)
            .try_fold(Duration::MAX, |remaining, wait| {
                wait.deadline.remaining().map(|left| remaining.min(left))
            })
            .ok()
    })
}

fn wait_for_database_deadline<const HANDLER: usize>(_: i32) -> bool {
    let Some(remaining) = remaining_database_wait(HANDLER) else {
        return false;
    };
    std::thread::sleep(remaining.min(Duration::from_millis(10)));
    remaining_database_wait(HANDLER).is_some()
}

struct DatabaseBudget<'a> {
    conn: &'a mut Connection,
    identity: usize,
    previous_timeout: Duration,
    handler: usize,
    active: bool,
}

impl<'a> DatabaseBudget<'a> {
    fn new(conn: &'a mut Connection, deadline: Deadline) -> Result<Self, CafleetError> {
        let previous: u32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .map_err(crate::broker::members::db_err)?;
        let identity = std::ptr::from_ref(conn).addr();
        // Distinct safe function pointers give rusqlite a connection-specific callback.
        let handler = DATABASE_WAITS
            .with(|waits| {
                let waits = waits.borrow();
                waits
                    .iter()
                    .find(|wait| wait.identity == identity)
                    .map(|wait| wait.handler)
                    .or_else(|| {
                        (0..DATABASE_HANDLERS.len())
                            .find(|handler| waits.iter().all(|wait| wait.handler != *handler))
                    })
            })
            .ok_or_else(|| {
                CafleetError::App("all scoped database deadline handlers are in use".into())
            })?;
        conn.busy_handler(Some(DATABASE_HANDLERS[handler]))
            .map_err(crate::broker::members::db_err)?;
        DATABASE_WAITS.with(|waits| {
            waits.borrow_mut().push(DatabaseWait {
                identity,
                deadline,
                handler,
            })
        });
        Ok(Self {
            conn,
            identity,
            previous_timeout: Duration::from_millis(u64::from(previous)),
            handler,
            active: true,
        })
    }

    fn restore(&mut self) -> Result<(), CafleetError> {
        if !self.active {
            return Ok(());
        }
        let previous = DATABASE_WAITS.with(|waits| {
            let waits = waits.borrow();
            assert_eq!(
                waits
                    .last()
                    .expect("an active database budget is registered")
                    .identity,
                self.identity
            );
            assert_eq!(waits.last().unwrap().handler, self.handler);
            waits
                .iter()
                .rev()
                .skip(1)
                .find(|wait| wait.identity == self.identity)
                .map(|wait| wait.handler)
        });
        if let Some(handler) = previous {
            self.conn
                .busy_handler(Some(DATABASE_HANDLERS[handler]))
                .map_err(crate::broker::members::db_err)
        } else {
            self.conn
                .busy_timeout(self.previous_timeout)
                .map_err(crate::broker::members::db_err)
        }?;
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
    deadline.remaining()?;
    complete_database_budget(conn, deadline, operation)
}

// Successful pane side effects still need uncontended finalization after expiry;
// the original deadline continues to bound every SQLite lock wait.
pub(crate) fn complete_database_budget<T>(
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

#[cfg(test)]
mod database_scope_tests {
    use super::*;

    #[test]
    fn expired_completion_commits_uncontended_work_without_starting_a_new_budget() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE probe(value INTEGER);")
            .unwrap();
        let expired = Deadline::until(Instant::now() - Duration::from_millis(1));
        assert!(with_database_budget(&mut conn, expired, |_| Ok(())).is_err());
        complete_database_budget(&mut conn, expired, |conn| {
            conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO probe VALUES(42); COMMIT;")
                .map_err(crate::broker::members::db_err)
        })
        .unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
    }

    #[test]
    fn expired_completion_reports_real_contention_without_waiting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("completion.db");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch("CREATE TABLE probe(value INTEGER); BEGIN IMMEDIATE;")
            .unwrap();
        let mut conn = Connection::open(&path).unwrap();
        let started = Instant::now();
        let expired = Deadline::until(started - Duration::from_millis(1));
        let error = complete_database_budget(&mut conn, expired, |conn| {
            conn.execute("INSERT INTO probe VALUES(42)", [])
                .map_err(crate::broker::members::db_err)
        })
        .unwrap_err();
        assert!(error.message().contains("database is locked"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_nested_connection_uses_its_own_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("separate.db");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE;",
            )
            .unwrap();
        let mut inner = Connection::open(&path).unwrap();
        let mut outer = Connection::open_in_memory().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            writer.execute_batch("ROLLBACK").unwrap();
        });
        let result = with_database_budget(
            &mut outer,
            Deadline::until(Instant::now() + Duration::from_millis(200)),
            |_| {
                with_database_budget(
                    &mut inner,
                    Deadline::until(Instant::now() + Duration::from_secs(2)),
                    |conn| {
                        conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                            .map_err(crate::broker::members::db_err)
                    },
                )
            },
        );
        release.join().unwrap();
        assert_eq!(result.unwrap(), 42);
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

pub(crate) struct LockedDeletion<S, T> {
    pub snapshot: S,
    pub value: T,
    _guards: Vec<PaneLock>,
}

pub(crate) fn lock_deletion<S, T>(
    conn: &mut Connection,
    settings: &Settings,
    deadline: Deadline,
    mut load: impl FnMut(&mut Connection) -> Result<(S, Vec<(i64, Placement)>), CafleetError>,
    mut revalidate: impl FnMut(&mut Connection, &S) -> Result<Option<T>, CafleetError>,
    busy: impl Fn(i64) -> CafleetError,
) -> Result<LockedDeletion<S, T>, CafleetError> {
    loop {
        let (snapshot, targets) = with_database_budget(conn, deadline, &mut load)?;
        let mut targets = targets
            .into_iter()
            .map(|(member_id, placement)| {
                let path = lock_path(&settings.database_url, &placement).map_err(|error| {
                    CafleetError::App(format!(
                        "member {member_id} pane input: {}",
                        error.message()
                    ))
                })?;
                Ok((path, member_id, placement))
            })
            .collect::<Result<Vec<_>, CafleetError>>()?;
        targets.sort_by(|left, right| (&left.0, left.1).cmp(&(&right.0, right.1)));
        targets.dedup_by(|left, right| left.0 == right.0);
        let mut guards = Vec::new();
        for (_, member_id, placement) in targets {
            let guard = loop {
                deadline.remaining()?;
                match PaneLock::try_acquire(&settings.database_url, &placement).map_err(
                    |error| {
                        CafleetError::App(format!(
                            "member {member_id} pane input: {}",
                            error.message()
                        ))
                    },
                )? {
                    Some(guard) => break guard,
                    None if deadline.retry() => {}
                    None => return Err(busy(member_id)),
                }
            };
            guards.push(guard);
        }
        let value = with_database_budget(conn, deadline, |conn| revalidate(conn, &snapshot))?;
        if let Some(value) = value {
            return Ok(LockedDeletion {
                snapshot,
                value,
                _guards: guards,
            });
        }
        drop(guards);
        deadline.remaining()?;
    }
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
    let locked = lock_deletion(
        conn,
        settings,
        deadline,
        |conn| {
            let snapshot = deletion_placements(conn, settings, fleet_id)?;
            let targets = snapshot
                .iter()
                .map(|target| (target.member_id, target.placement.clone()))
                .collect();
            Ok((snapshot, targets))
        },
        |conn, snapshot| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(crate::broker::members::db_err)?;
            if deletion_placements(&tx, settings, fleet_id)? != *snapshot {
                return Ok(None);
            }
            let result = crate::broker::fleets::delete_fleet_rows(&tx, fleet_id)?;
            deadline.remaining()?;
            tx.commit().map_err(crate::broker::members::db_err)?;
            Ok(Some(result))
        },
        |member| {
            CafleetError::App(format!(
                "fleet {fleet_id} pane input remained busy for 5 seconds at member {member}; fleet was not deleted."
            ))
        },
    )?;
    Ok(locked.value)
}
