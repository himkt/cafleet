use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::broker::records::{DeletedFleet, DeletedMember, MemberRecord, Placement};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::{AnyMultiplexer, PostCloseLayout};

#[derive(Debug, Clone, Copy)]
pub struct Deadline(Instant);

pub(crate) fn valid_input(text: &str) -> bool {
    !text.trim().is_empty() && !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

impl Deadline {
    pub(crate) fn instant(self) -> Instant {
        self.0
    }
    pub(crate) fn inherited_or_new() -> Self {
        DATABASE_WAITS
            .with(|waits| {
                waits
                    .borrow()
                    .frames
                    .iter()
                    .map(|frame| frame.deadline)
                    .min_by_key(|deadline| deadline.0)
            })
            .unwrap_or_default()
    }
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

    /// The deadline `reserve` earlier; already expired when less than `reserve` remains.
    pub(crate) fn reserving(self, reserve: Duration) -> Self {
        Self(
            self.0
                .checked_sub(reserve)
                .expect("a monotonic deadline lies far past the clock origin"),
        )
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
    deadline: Deadline,
    completion: bool,
}

#[derive(Default)]
struct DatabaseWaits {
    identity: Option<usize>,
    frames: Vec<DatabaseWait>,
}

thread_local! {
    static DATABASE_WAITS: std::cell::RefCell<DatabaseWaits> = const {
        std::cell::RefCell::new(DatabaseWaits { identity: None, frames: Vec::new() })
    };
}

pub(crate) fn inherited_database_deadline(conn: &Connection) -> Option<Deadline> {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        if waits.frames.is_empty() {
            assert!(
                waits.identity.is_none(),
                "an empty database scope has no owner"
            );
            return None;
        }
        assert_eq!(
            waits.identity,
            Some(std::ptr::from_ref(conn).addr()),
            "inherited database scopes use the same connection"
        );
        waits
            .frames
            .iter()
            .map(|frame| frame.deadline)
            .min_by_key(|deadline| deadline.instant())
    })
}

fn remaining_database_wait() -> Option<Duration> {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        assert!(
            waits.identity.is_some() && !waits.frames.is_empty(),
            "a busy handler has an owning connection scope"
        );
        let start = waits
            .frames
            .iter()
            .rposition(|wait| wait.completion)
            .unwrap_or(0);
        waits.frames[start..]
            .iter()
            .try_fold(Duration::MAX, |remaining, wait| {
                wait.deadline.remaining().map(|left| remaining.min(left))
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
    fn new(
        conn: &'a mut Connection,
        deadline: Deadline,
        completion: bool,
    ) -> Result<Self, CafleetError> {
        let identity = std::ptr::from_ref(conn).addr();
        DATABASE_WAITS.with(|waits| {
            if let Some(owner) = waits.borrow().identity {
                assert_eq!(
                    owner, identity,
                    "nested database scopes use the same connection"
                );
            }
        });
        let previous: u32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .map_err(crate::broker::members::db_err)?;
        conn.busy_handler(Some(wait_for_database_deadline))
            .map_err(crate::broker::members::db_err)?;
        DATABASE_WAITS.with(|waits| {
            let mut waits = waits.borrow_mut();
            waits.identity = Some(identity);
            waits.frames.push(DatabaseWait {
                deadline,
                completion,
            })
        });
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
            assert_eq!(waits.identity, Some(self.identity));
            assert!(
                !waits.frames.is_empty(),
                "an active database budget is registered"
            );
            waits.frames.len() > 1
        });
        let restored = if nested {
            self.conn
                .busy_handler(Some(wait_for_database_deadline))
                .map_err(crate::broker::members::db_err)
        } else {
            self.conn
                .busy_timeout(self.previous_timeout)
                .map_err(crate::broker::members::db_err)
        };
        DATABASE_WAITS.with(|waits| {
            let mut waits = waits.borrow_mut();
            waits
                .frames
                .pop()
                .expect("the active database scope is registered");
            if waits.frames.is_empty() {
                waits.identity = None;
            }
        });
        self.active = false;
        restored
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
    scoped_database_budget(conn, deadline, false, operation)
}

pub(crate) fn finalize_deletion<T>(
    conn: &mut Connection,
    deadline: Deadline,
    wrapped: bool,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    if wrapped {
        with_database_budget(conn, deadline, operation)
    } else {
        complete_database_budget(conn, deadline, operation)
    }
}

// Pane side effects are irreversible; mandatory storage finalization gets a
// bounded grace period independent of the input deadline.
pub(crate) fn complete_database_budget<T>(
    conn: &mut Connection,
    deadline: Deadline,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    scoped_database_budget(conn, completion_deadline(deadline), true, operation)
}

fn completion_deadline(deadline: Deadline) -> Deadline {
    Deadline::until(deadline.0.max(Instant::now() + Duration::from_secs(1)))
}

fn scoped_database_budget<T>(
    conn: &mut Connection,
    deadline: Deadline,
    completion: bool,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    let mut budget = DatabaseBudget::new(conn, deadline, completion)?;
    let result = operation(budget.conn);
    let restored = budget.restore().map_err(|error| {
        CafleetError::App(format!(
            "restoring database wait failed: {}",
            error.message()
        ))
    });
    match (result, restored) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(primary), Err(cleanup)) => Err(primary.with_cleanup(cleanup.message())),
    }
}

#[cfg(test)]
mod database_scope_tests {
    use super::*;

    #[test]
    fn expired_completion_commits_uncontended_work() {
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
    fn completion_waits_for_a_reader_to_release_commit_after_input_expiry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reader.db");
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(1)")
            .unwrap();
        let reader = Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT * FROM probe").unwrap();
        let (release, released) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            released.recv().unwrap();
            std::thread::sleep(Duration::from_millis(100));
            reader.execute_batch("COMMIT").unwrap();
        });
        let deadline = Deadline::until(Instant::now() + Duration::from_millis(20));
        with_database_budget(&mut conn, deadline, |conn| {
            std::thread::sleep(Duration::from_millis(30));
            complete_database_budget(conn, deadline, |conn| {
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(crate::broker::members::db_err)?;
                tx.execute("UPDATE probe SET value=42", [])
                    .map_err(crate::broker::members::db_err)?;
                release.send(()).unwrap();
                tx.commit().map_err(crate::broker::members::db_err)
            })?;
            assert!(with_database_budget(conn, deadline, |_| Ok(())).is_err());
            Ok(())
        })
        .unwrap();
        thread.join().unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
    }

    #[test]
    fn expired_completion_reports_contention_after_bounded_grace() {
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
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    fn assert_timeout(conn: &Connection, expected: u32) {
        assert_eq!(
            conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            expected
        );
    }

    #[test]
    fn scope_exit_restores_timeouts_and_allows_sequential_connections_and_threads() {
        let mut first = Connection::open_in_memory().unwrap();
        let mut second = Connection::open_in_memory().unwrap();
        for conn in [&mut first, &mut second] {
            conn.busy_timeout(Duration::from_millis(137)).unwrap();
            with_database_budget(conn, Deadline::new(), |_| Ok(())).unwrap();
            assert_timeout(conn, 137);
            assert!(
                with_database_budget(conn, Deadline::new(), |_| Err::<(), _>(CafleetError::App(
                    "expected".into()
                )))
                .is_err()
            );
            assert_timeout(conn, 137);
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ =
                    with_database_budget(conn, Deadline::new(), |_| -> Result<(), CafleetError> {
                        panic!("scope unwind")
                    });
            }));
            assert!(panic.is_err());
            assert_timeout(conn, 137);
        }
        with_database_budget(&mut first, Deadline::new(), |_| {
            std::thread::spawn(|| {
                let mut conn = Connection::open_in_memory().unwrap();
                with_database_budget(&mut conn, Deadline::new(), |_| Ok(())).unwrap();
            })
            .join()
            .unwrap();
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn cross_connection_nesting_fails_before_changing_the_inner_timeout() {
        let mut outer = Connection::open_in_memory().unwrap();
        let mut inner = Connection::open_in_memory().unwrap();
        inner.busy_timeout(Duration::from_millis(137)).unwrap();
        with_database_budget(&mut outer, Deadline::new(), |_| {
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_database_budget(&mut inner, Deadline::new(), |_| Ok(()))
            }));
            assert!(panic.is_err());
            assert_timeout(&inner, 137);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn nested_exit_restores_the_earliest_outer_deadline_on_every_exit() {
        for exit in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested.db");
            let writer = Connection::open(&path).unwrap();
            writer.execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE").unwrap();
            let mut conn = Connection::open(&path).unwrap();
            conn.busy_timeout(Duration::from_millis(137)).unwrap();
            let started = Instant::now();
            let outer = Deadline::until(started + Duration::from_millis(160));
            let error = with_database_budget(&mut conn, outer, |conn| {
                match exit {
                    0 => with_database_budget(conn, Deadline::new(), |_| Ok(())).unwrap(),
                    1 => assert!(
                        with_database_budget(conn, Deadline::new(), |_| Err::<(), _>(
                            CafleetError::App("expected".into())
                        ))
                        .is_err()
                    ),
                    2 => assert!(
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let _ = with_database_budget(
                                conn,
                                Deadline::new(),
                                |_| -> Result<(), CafleetError> { panic!("nested unwind") },
                            );
                        }))
                        .is_err()
                    ),
                    _ => unreachable!(),
                }
                with_database_budget(conn, Deadline::new(), |conn| {
                    conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                        .map_err(crate::broker::members::db_err)
                })
            })
            .unwrap_err();
            assert!(matches!(error, CafleetError::DatabaseBusy(_)));
            assert!(started.elapsed() >= Duration::from_millis(140));
            assert!(started.elapsed() < Duration::from_millis(700));
            assert_timeout(&conn, 137);
        }
    }

    #[test]
    fn consecutive_sql_waits_share_one_absolute_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aggregate.db");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE",
            )
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            writer.execute_batch("ROLLBACK").unwrap();
        });
        let mut conn = Connection::open(&path).unwrap();
        let started = Instant::now();
        let deadline = Deadline::until(started + Duration::from_millis(200));
        let error = with_database_budget(&mut conn, deadline, |conn| {
            assert_eq!(
                conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .map_err(crate::broker::members::db_err)?,
                42
            );
            release.join().unwrap();
            let writer = Connection::open(&path).unwrap();
            writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
            conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                .map_err(crate::broker::members::db_err)
        })
        .unwrap_err();
        assert!(matches!(error, CafleetError::DatabaseBusy(_)));
        assert!(started.elapsed() >= Duration::from_millis(180));
        assert!(started.elapsed() < Duration::from_millis(700));
    }
}

pub struct PaneLock {
    _file: File,
    path: PathBuf,
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
    pub(crate) fn validate_binding(
        &self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<(), CafleetError> {
        use std::os::unix::fs::MetadataExt;
        let path = lock_path(database_url, placement)?;
        if self.path != path {
            return Err(CafleetError::App("held pane lock binding mismatch".into()));
        }
        let open = self
            ._file
            .metadata()
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        let named = std::fs::symlink_metadata(&path)
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        if !named.is_file()
            || named.file_type().is_symlink()
            || (open.dev(), open.ino()) != (named.dev(), named.ino())
        {
            return Err(CafleetError::App(
                "held pane lock pathname identity mismatch".into(),
            ));
        }
        Ok(())
    }
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
            Ok(()) => Ok(Some(Self { _file: file, path })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(CafleetError::App(format!(
                "cannot lock pane input {}: {error}",
                path.display()
            ))),
        }
    }

    /// Retry a held lock every 50 ms until `lock_deadline`; `None` means it stayed busy.
    pub fn acquire_within(
        database_url: &str,
        placement: &Placement,
        lock_deadline: Deadline,
    ) -> Result<Option<Self>, CafleetError> {
        loop {
            if let Some(guard) = Self::try_acquire(database_url, placement)? {
                return Ok(Some(guard));
            }
            let Ok(left) = lock_deadline.remaining() else {
                return Ok(None);
            };
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
    }
}

pub struct LockedMember {
    pub member: MemberRecord,
    _guard: PaneLock,
}

impl LockedMember {
    pub(crate) fn pane_lock(&self, database_url: &str) -> Result<&PaneLock, CafleetError> {
        let placement = self
            .member
            .placement
            .as_ref()
            .expect("locked member has a placement");
        self._guard.validate_binding(database_url, placement)?;
        Ok(&self._guard)
    }
}

pub(crate) struct LockedDeletion<S, T> {
    pub snapshot: S,
    pub value: T,
    _span: crate::signals::DeferTermination,
    _guards: Vec<PaneLock>,
}

pub(crate) fn pane_lock_for<'a>(
    guards: &'a [PaneLock],
    database_url: &str,
    placement: &Placement,
) -> Result<&'a PaneLock, CafleetError> {
    let path = lock_path(database_url, placement)?;
    let guard = guards
        .iter()
        .find(|guard| guard.path == path)
        .ok_or_else(|| CafleetError::App("matching held pane lock required".into()))?;
    guard.validate_binding(database_url, placement)?;
    Ok(guard)
}

impl<S, T> LockedDeletion<S, T> {
    pub(crate) fn pane_lock(
        &self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<&PaneLock, CafleetError> {
        pane_lock_for(&self._guards, database_url, placement)
    }

    pub(crate) fn snapshot_and_pane_lock(
        &mut self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<(&mut S, &PaneLock), CafleetError> {
        let guard = pane_lock_for(&self._guards, database_url, placement)?;
        Ok((&mut self.snapshot, guard))
    }
}

pub(crate) fn lock_deletion<S, T>(
    conn: &mut Connection,
    settings: &Settings,
    deadline: Deadline,
    mut load: impl FnMut(&mut Connection) -> Result<(S, Vec<(i64, Placement)>), CafleetError>,
    mut revalidate: impl FnMut(&mut Connection, &mut S, &[PaneLock]) -> Result<Option<T>, CafleetError>,
    busy: impl Fn(i64) -> CafleetError,
) -> Result<LockedDeletion<S, T>, CafleetError> {
    loop {
        let (mut snapshot, targets) = with_database_budget(conn, deadline, &mut load)?;
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
        let span = crate::signals::DeferTermination::begin()?;
        let value = with_database_budget(conn, deadline, |conn| {
            revalidate(conn, &mut snapshot, &guards)
        })?;
        if let Some(value) = value {
            return Ok(LockedDeletion {
                snapshot,
                value,
                _span: span,
                _guards: guards,
            });
        }
        drop(span);
        drop(guards);
        deadline.remaining()?;
    }
}

pub fn lock_member(
    conn: &mut Connection,
    settings: &Settings,
    member: &MemberRecord,
    deadline: Deadline,
) -> Result<Option<LockedMember>, CafleetError> {
    lock_member_with_reservation(
        conn,
        settings,
        member,
        deadline,
        crate::multiplexer::INPUT_RESERVATION,
    )
}

pub(crate) fn lock_member_with_reservation(
    conn: &mut Connection,
    settings: &Settings,
    member: &MemberRecord,
    deadline: Deadline,
    reservation: Duration,
) -> Result<Option<LockedMember>, CafleetError> {
    let placement = member.placement.as_ref().ok_or_else(|| {
        CafleetError::App(format!("member {} has no placement row", member.member_id))
    })?;
    let Some(guard) = PaneLock::acquire_within(
        &settings.database_url,
        placement,
        deadline.reserving(reservation),
    )
    .map_err(|error| {
        CafleetError::App(format!(
            "member {} pane input: {}",
            member.member_id,
            error.message()
        ))
    })?
    else {
        return Ok(None);
    };
    let current = with_database_budget(conn, deadline, |conn| {
        crate::broker::get_member(conn, member.member_id, member.fleet_id)
    })?
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

#[cfg(test)]
mod pane_lock_wait_tests {
    use super::*;
    use crate::broker::tests as common;
    use crate::multiplexer::INPUT_RESERVATION;

    struct LockFixture {
        _dir: tempfile::TempDir,
        conn: Connection,
        settings: Settings,
        member: MemberRecord,
    }

    impl LockFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = common::migrated_conn(&dir);
            let (fleet, _) = common::create_fleet(&mut conn, "locks");
            let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
            let member = crate::broker::get_member(&conn, worker, fleet)
                .unwrap()
                .unwrap();
            let url = format!("sqlite:///{}", conn.path().unwrap());
            let settings =
                Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone()))
                    .unwrap();
            Self {
                _dir: dir,
                conn,
                settings,
                member,
            }
        }

        fn placement(&self) -> &Placement {
            self.member.placement.as_ref().unwrap()
        }

        fn hold_for(&self, hold: Option<Duration>) -> std::thread::JoinHandle<()> {
            let guard = PaneLock::try_acquire(&self.settings.database_url, self.placement())
                .unwrap()
                .expect("the fixture lock is free");
            std::thread::spawn(move || {
                match hold {
                    Some(hold) => std::thread::sleep(hold),
                    None => std::thread::sleep(Duration::from_secs(3)),
                }
                drop(guard);
            })
        }

        fn writer_deadline() -> Deadline {
            Deadline::new().reserving(INPUT_RESERVATION)
        }
    }

    #[test]
    fn reserving_moves_the_deadline_earlier_by_the_reserve() {
        let deadline = Deadline::until(Instant::now() + Duration::from_secs(5));
        let left = deadline.reserving(INPUT_RESERVATION).remaining().unwrap();
        assert!(left <= Duration::from_millis(1500), "{left:?}");
        assert!(left > Duration::from_millis(1400), "{left:?}");
    }

    #[test]
    fn reserving_more_than_remains_is_already_expired() {
        let deadline = Deadline::until(Instant::now() + Duration::from_secs(3));
        assert!(deadline.reserving(INPUT_RESERVATION).remaining().is_err());
    }

    #[test]
    fn an_uncontended_lock_is_acquired_at_once() {
        let fixture = LockFixture::new();
        let started = Instant::now();
        let guard = PaneLock::acquire_within(
            &fixture.settings.database_url,
            fixture.placement(),
            LockFixture::writer_deadline(),
        )
        .unwrap();
        assert!(guard.is_some());
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn a_lock_released_within_the_wait_is_acquired() {
        let fixture = LockFixture::new();
        let holder = fixture.hold_for(Some(Duration::from_millis(300)));
        let started = Instant::now();
        let guard = PaneLock::acquire_within(
            &fixture.settings.database_url,
            fixture.placement(),
            LockFixture::writer_deadline(),
        )
        .unwrap();
        assert!(guard.is_some());
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert!(started.elapsed() < Duration::from_millis(1000));
        holder.join().unwrap();
    }

    #[test]
    fn a_lock_released_just_before_the_reserved_deadline_is_acquired() {
        let fixture = LockFixture::new();
        let lock_deadline = LockFixture::writer_deadline();
        let holder = fixture.hold_for(Some(Duration::from_millis(1300)));
        let guard = PaneLock::acquire_within(
            &fixture.settings.database_url,
            fixture.placement(),
            lock_deadline,
        )
        .unwrap();
        assert!(guard.is_some(), "the 50 ms retry observes the release");
        holder.join().unwrap();
    }

    #[test]
    fn a_lock_still_busy_at_the_reserved_deadline_returns_none() {
        let fixture = LockFixture::new();
        let holder = fixture.hold_for(None);
        let started = Instant::now();
        let guard = PaneLock::acquire_within(
            &fixture.settings.database_url,
            fixture.placement(),
            LockFixture::writer_deadline(),
        )
        .unwrap();
        assert!(guard.is_none());
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(1400), "{waited:?}");
        assert!(waited < Duration::from_millis(2000), "{waited:?}");
        holder.join().unwrap();
    }

    #[test]
    fn lock_member_waits_for_the_file_lock_then_revalidates_the_placement() {
        let mut fixture = LockFixture::new();
        let holder = fixture.hold_for(Some(Duration::from_millis(300)));
        let member = fixture.member.clone();
        let locked = lock_member(
            &mut fixture.conn,
            &fixture.settings,
            &member,
            Deadline::new(),
        )
        .unwrap()
        .expect("the lock frees within the wait");
        assert_eq!(locked.member, member);
        holder.join().unwrap();
    }

    #[test]
    fn lock_member_reports_a_busy_pane_after_the_bounded_wait() {
        let mut fixture = LockFixture::new();
        let holder = fixture.hold_for(None);
        let member = fixture.member.clone();
        let started = Instant::now();
        let locked = lock_member(
            &mut fixture.conn,
            &fixture.settings,
            &member,
            Deadline::new(),
        )
        .unwrap();
        assert!(locked.is_none());
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(1400), "{waited:?}");
        assert!(waited < Duration::from_millis(2000), "{waited:?}");
        holder.join().unwrap();
    }

    #[test]
    fn lock_member_rejects_a_placement_changed_while_acquiring() {
        let mut fixture = LockFixture::new();
        let member = fixture.member.clone();
        let guard = PaneLock::try_acquire(&fixture.settings.database_url, fixture.placement())
            .unwrap()
            .unwrap();
        let path = fixture.conn.path().unwrap().to_owned();
        let member_id = member.member_id;
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            let conn = Connection::open(path).unwrap();
            conn.execute(
                "UPDATE member_placements SET mux_window_id='@9' WHERE member_id=?1",
                [member_id],
            )
            .unwrap();
            drop(guard);
        });
        let error = match lock_member(
            &mut fixture.conn,
            &fixture.settings,
            &member,
            Deadline::new(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("a changed placement must be rejected"),
        };
        assert_eq!(
            error.message(),
            format!(
                "member {} placement changed while acquiring pane input",
                member.member_id
            )
        );
        holder.join().unwrap();
        assert!(
            PaneLock::try_acquire(&fixture.settings.database_url, fixture.placement())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn physical_lock_identity_uses_canonical_database_backend_session_and_pane() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let fixture = LockFixture::new();
        let url = &fixture.settings.database_url;
        let placement = fixture.placement();
        let alias = fixture._dir.path().join("alias.db");
        symlink(crate::db::database_path(url).unwrap(), &alias).unwrap();
        let alias_url = format!("sqlite:///{}", alias.display());
        let path = lock_path(url, placement).unwrap();
        assert_eq!(lock_path(&alias_url, placement).unwrap(), path);
        let mut duplicate = placement.clone();
        duplicate.mux_window_id = "@99".into();
        duplicate.coding_agent = "codex".into();
        assert_eq!(lock_path(url, &duplicate).unwrap(), path);
        for alternative in [
            Placement {
                backend: "herdr".into(),
                ..placement.clone()
            },
            Placement {
                mux_session: "other".into(),
                ..placement.clone()
            },
            Placement {
                mux_pane_id: Some("%99".into()),
                ..placement.clone()
            },
        ] {
            assert_ne!(lock_path(url, &alternative).unwrap(), path);
            assert!(PaneLock::try_acquire(url, &alternative).unwrap().is_some());
        }
        let guard = PaneLock::try_acquire(url, placement).unwrap().unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        std::fs::write(&path, "operator marker").unwrap();
        assert!(
            PaneLock::try_acquire(&alias_url, &duplicate)
                .unwrap()
                .is_none()
        );
        drop(guard);
        let _guard = PaneLock::try_acquire(&alias_url, &duplicate)
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), metadata.ino());
        assert_eq!(std::fs::read(&path).unwrap(), b"operator marker");
    }

    #[test]
    fn fleet_deletion_contention_preserves_rows_and_releases_preceding_guards() {
        let mut fixture = LockFixture::new();
        let placements =
            deletion_placements(&fixture.conn, &fixture.settings, fixture.member.fleet_id).unwrap();
        let held = placements.last().unwrap();
        let guard = PaneLock::try_acquire(&fixture.settings.database_url, &held.placement)
            .unwrap()
            .unwrap();
        let before = crate::broker::list_members(&fixture.conn, fixture.member.fleet_id).unwrap();
        let started = Instant::now();
        assert!(
            delete_fleet(
                &mut fixture.conn,
                &fixture.settings,
                fixture.member.fleet_id,
                Deadline::until(started + Duration::from_millis(160))
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(700));
        assert_eq!(
            crate::broker::list_members(&fixture.conn, fixture.member.fleet_id).unwrap(),
            before
        );
        for placement in &placements[..placements.len() - 1] {
            assert!(
                PaneLock::try_acquire(&fixture.settings.database_url, &placement.placement)
                    .unwrap()
                    .is_some()
            );
        }
        drop(guard);
        assert_eq!(
            common::guarded_delete_fleet(&mut fixture.conn, fixture.member.fleet_id)
                .unwrap()
                .deregistered_count,
            before.len()
        );
    }

    #[test]
    fn deletion_deduplicates_physical_panes_and_revalidates_after_all_locks() {
        let mut fixture = LockFixture::new();
        let placement = fixture.placement().clone();
        let member = fixture.member.member_id;
        let locked = lock_deletion(
            &mut fixture.conn,
            &fixture.settings,
            Deadline::new(),
            |_| {
                Ok((
                    (),
                    vec![(member, placement.clone()), (member + 1, placement.clone())],
                ))
            },
            |_, _, _| {
                assert!(
                    PaneLock::try_acquire(&fixture.settings.database_url, &placement)
                        .unwrap()
                        .is_none()
                );
                Ok(Some(()))
            },
            |_| CafleetError::App("duplicate pane remained locked".into()),
        )
        .unwrap();
        assert!(
            PaneLock::try_acquire(&fixture.settings.database_url, &placement)
                .unwrap()
                .is_none()
        );
        drop(locked);
        assert!(
            PaneLock::try_acquire(&fixture.settings.database_url, &placement)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn deletion_acquires_physical_keys_in_sorted_order() {
        let fixture = LockFixture::new();
        let targets =
            deletion_placements(&fixture.conn, &fixture.settings, fixture.member.fleet_id).unwrap();
        assert_eq!(targets.len(), 2);
        let high_guard =
            PaneLock::try_acquire(&fixture.settings.database_url, &targets[1].placement)
                .unwrap()
                .unwrap();
        let low = targets[0].placement.clone();
        let settings = fixture.settings.clone();
        let path = fixture.conn.path().unwrap().to_owned();
        let (loaded, wait_loaded) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut conn = Connection::open(path).unwrap();
            let locked = lock_deletion(
                &mut conn,
                &settings,
                Deadline::until(Instant::now() + Duration::from_secs(2)),
                |_| {
                    loaded.send(()).unwrap();
                    Ok((
                        (),
                        targets
                            .iter()
                            .rev()
                            .map(|target| (target.member_id, target.placement.clone()))
                            .collect(),
                    ))
                },
                |_, _, _| Ok(Some(())),
                |_| CafleetError::App("sorted lock wait expired".into()),
            )
            .unwrap();
            drop(locked);
        });
        wait_loaded.recv_timeout(Duration::from_secs(1)).unwrap();
        let limit = Instant::now() + Duration::from_millis(700);
        loop {
            if PaneLock::try_acquire(&fixture.settings.database_url, &low)
                .unwrap()
                .is_none()
            {
                break;
            }
            assert!(
                Instant::now() < limit,
                "the lower key is owned while the higher key waits"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(high_guard);
        thread.join().unwrap();
    }

    #[test]
    fn fleet_deletion_reloads_new_placements_within_the_original_deadline() {
        let mut fixture = LockFixture::new();
        let placements =
            deletion_placements(&fixture.conn, &fixture.settings, fixture.member.fleet_id).unwrap();
        let first = &placements[0].placement;
        let last = PaneLock::try_acquire(&fixture.settings.database_url, &placements[1].placement)
            .unwrap()
            .unwrap();
        let path = fixture.conn.path().unwrap().to_owned();
        let settings = fixture.settings.clone();
        let fleet = fixture.member.fleet_id;
        let started = Instant::now();
        let deadline = Deadline::until(started + Duration::from_millis(600));
        let thread = std::thread::spawn(move || {
            let mut conn = Connection::open(path).unwrap();
            delete_fleet(&mut conn, &settings, fleet, deadline)
        });
        loop {
            if PaneLock::try_acquire(&fixture.settings.database_url, first)
                .unwrap()
                .is_none()
            {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_millis(400),
                "deletion owns the first key before waiting on the last"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let added = common::register(&mut fixture.conn, fleet, "added-during-wait", Some("%99"));
        let added_placement = crate::broker::get_member(&fixture.conn, added, fleet)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        let added_guard = PaneLock::try_acquire(&fixture.settings.database_url, &added_placement)
            .unwrap()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(last);
        let error = thread.join().unwrap().unwrap_err();
        assert!(
            error.message().contains(&format!("at member {added};")),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_millis(1100));
        assert_eq!(
            crate::broker::list_members(&fixture.conn, fleet)
                .unwrap()
                .len(),
            3
        );
        assert!(
            crate::broker::get_fleet(&fixture.conn, fleet)
                .unwrap()
                .unwrap()
                .deleted_at
                .is_none()
        );
        drop(added_guard);
        for placement in placements {
            assert!(
                PaneLock::try_acquire(&fixture.settings.database_url, &placement.placement)
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct DeletionPlacement {
    member_id: i64,
    placement: Placement,
    path: PathBuf,
    run_identity: Option<serde_json::Value>,
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
        let run_identity = crate::broker::runtime::member_runtime(conn, member_id)?
            .map(|runtime| deletion_run_identity(&runtime.run));
        placements.push(DeletionPlacement {
            member_id,
            placement,
            path,
            run_identity,
        });
    }
    placements
        .sort_by(|left, right| (&left.path, left.member_id).cmp(&(&right.path, right.member_id)));
    Ok(placements)
}

pub(crate) const PENDING_PANE_STATUS: &str = "(pending — no pane)";
pub(crate) const NO_PLACEMENT_STATUS: &str = "(no placement)";

pub(crate) trait DeletionHost {
    #[expect(
        clippy::too_many_arguments,
        reason = "deletion forwards the validated caller identity and actual retained guard"
    )]
    fn recover(
        &mut self,
        conn: &mut Connection,
        settings: &Settings,
        member_id: i64,
        expected_run: &crate::broker::runtime::RunRecord,
        expected_placement: &Placement,
        held: &PaneLock,
        deadline: Deadline,
    ) -> Result<crate::runtime::runner::RecoveryCompletion, CafleetError> {
        crate::runtime::runner::recover_member_under_pane_lock(
            conn,
            settings,
            member_id,
            expected_run,
            expected_placement,
            deadline.instant(),
            held,
        )
    }
    fn shutdown(
        &mut self,
        conn: &mut Connection,
        run: &crate::broker::runtime::RunRecord,
        deadline: Deadline,
    ) -> Result<crate::runtime::runner::ShutdownOutcome, CafleetError>;
    fn close_pane(
        &mut self,
        placement: &Placement,
        deadline: Deadline,
    ) -> Result<PostCloseLayout, CafleetError>;
    fn restore_layout(&mut self, layout: PostCloseLayout) -> Result<(), CafleetError>;
}

pub(crate) struct MultiplexerDeletionHost<'a>(pub(crate) &'a AnyMultiplexer);

impl DeletionHost for MultiplexerDeletionHost<'_> {
    fn shutdown(
        &mut self,
        conn: &mut Connection,
        run: &crate::broker::runtime::RunRecord,
        deadline: Deadline,
    ) -> Result<crate::runtime::runner::ShutdownOutcome, CafleetError> {
        crate::runtime::runner::request_shutdown(conn, run, deadline.instant())
    }

    fn close_pane(
        &mut self,
        placement: &Placement,
        _: Deadline,
    ) -> Result<PostCloseLayout, CafleetError> {
        let pane = placement
            .mux_pane_id
            .as_deref()
            .expect("deletion target has a concrete pane");
        self.0
            .close_pane(pane, true)
            .map_err(|cause| CafleetError::App(cause.to_string()))
    }

    fn restore_layout(&mut self, layout: PostCloseLayout) -> Result<(), CafleetError> {
        self.0
            .restore_after_close(layout)
            .map_err(|cause| CafleetError::App(cause.to_string()))
    }
}

struct NativeFleetDeletionHost<'a> {
    settings: &'a Settings,
    mux: Option<AnyMultiplexer>,
}

impl NativeFleetDeletionHost<'_> {
    fn multiplexer(
        &mut self,
        deadline: Deadline,
    ) -> Result<MultiplexerDeletionHost<'_>, CafleetError> {
        if self.mux.is_none() {
            let mux = crate::runtime::resolve_mux(self.settings)
                .map_err(|cause| CafleetError::App(cause.to_string()))?
                .bounded(deadline);
            mux.ensure_available()
                .map_err(|cause| CafleetError::App(cause.to_string()))?;
            self.mux = Some(mux);
        }
        Ok(MultiplexerDeletionHost(
            self.mux
                .as_ref()
                .expect("the multiplexer was resolved above"),
        ))
    }
}

impl DeletionHost for NativeFleetDeletionHost<'_> {
    fn shutdown(
        &mut self,
        conn: &mut Connection,
        run: &crate::broker::runtime::RunRecord,
        deadline: Deadline,
    ) -> Result<crate::runtime::runner::ShutdownOutcome, CafleetError> {
        crate::runtime::runner::request_shutdown(conn, run, deadline.instant())
    }

    fn close_pane(
        &mut self,
        placement: &Placement,
        deadline: Deadline,
    ) -> Result<PostCloseLayout, CafleetError> {
        self.multiplexer(deadline)?.close_pane(placement, deadline)
    }

    fn restore_layout(&mut self, layout: PostCloseLayout) -> Result<(), CafleetError> {
        let mux = self
            .mux
            .as_ref()
            .expect("layout restoration follows a pane close");
        MultiplexerDeletionHost(mux).restore_layout(layout)
    }
}

fn deletion_run_identity(run: &crate::broker::runtime::RunRecord) -> serde_json::Value {
    crate::runtime::runner::stable_run_identity(run)
}

pub(crate) fn active_member(
    conn: &Connection,
    member_id: i64,
) -> Result<MemberRecord, CafleetError> {
    let missing = || CafleetError::App(format!("Member {member_id} not found"));
    let fleet_id = crate::broker::active_member_fleet(conn, member_id)?.ok_or_else(missing)?;
    crate::broker::get_member(conn, member_id, fleet_id)?.ok_or_else(missing)
}

pub(crate) fn member_deletion_matches(expected: &MemberRecord, current: &MemberRecord) -> bool {
    expected.member_id == current.member_id
        && expected.fleet_id == current.fleet_id
        && expected.kind == current.kind
        && expected.placement == current.placement
        && expected
            .runtime
            .as_ref()
            .map(|runtime| deletion_run_identity(&runtime.run))
            == current
                .runtime
                .as_ref()
                .map(|runtime| deletion_run_identity(&runtime.run))
}

fn revalidate_member(
    conn: &Connection,
    expected: &MemberRecord,
    stage: &str,
) -> Result<(), CafleetError> {
    if member_deletion_matches(expected, &active_member(conn, expected.member_id)?) {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "member placement or runtime ownership changed {stage}"
        )))
    }
}

/// `scope` revalidates the caller's wider deletion state, such as a fleet's
/// placement set, before each irreversible step up to the pane close.
pub(crate) fn end_member(
    conn: &mut Connection,
    settings: &Settings,
    snapshot: &mut MemberRecord,
    held: &PaneLock,
    host: &mut impl DeletionHost,
    deadline: Deadline,
    scope: &mut dyn FnMut(&Connection) -> Result<(), CafleetError>,
) -> Result<String, CafleetError> {
    use crate::broker::runtime::CleanupState;
    let member_id = snapshot.member_id;
    let placement = snapshot
        .placement
        .clone()
        .expect("an ended member holds its concrete placement");
    let pane_id = placement
        .mux_pane_id
        .clone()
        .expect("an ended member has a concrete pane");
    with_database_budget(conn, deadline, |conn| {
        revalidate_member(conn, snapshot, "before cleanup")?;
        scope(conn)?;
        if let Some(runtime) = crate::broker::runtime::member_runtime(conn, member_id)?
            && (!runtime.run.status.terminal()
                || runtime.run.cleanup_state == CleanupState::Incomplete)
        {
            let outcome = host.shutdown(conn, &runtime.run, deadline)?;
            if outcome.cleanup_state != CleanupState::Complete {
                return Err(CafleetError::App(format!(
                    "member {member_id} child cleanup remains unconfirmed: {}; inspect with cafleet doctor",
                    outcome.diagnostics.join("; ")
                )));
            }
        }
        if let Some(expected) = snapshot.runtime.as_ref().map(|runtime| runtime.run.clone()) {
            scope(conn)?;
            let current = crate::broker::runtime::get_run(conn, expected.run_id)?;
            if deletion_run_identity(&current) != deletion_run_identity(&expected) {
                return Err(CafleetError::App(
                    "member runtime ownership changed during cleanup".into(),
                ));
            }
            deadline.remaining()?;
            let completion = host.recover(
                conn, settings, member_id, &current, &placement, held, deadline,
            )?;
            crate::runtime::runner::validate_recovery_completion(&expected, &completion)?;
            if !matches!(
                completion.report.outcome,
                crate::runtime::runner::RecoveryOutcome::Recovered
            ) {
                return Err(CafleetError::App(
                    "member recovery remains incomplete; inspect with cafleet doctor".into(),
                ));
            }
            snapshot.runtime.as_mut().expect("captured runtime").run = completion.run;
        }
        revalidate_member(conn, snapshot, "before pane close")?;
        scope(conn)
    })?;
    deadline.remaining()?;
    let layout = host.close_pane(&placement, deadline).map_err(|error| {
        CafleetError::App(format!(
            "kill_pane failed for pane {pane_id}: {}. Verify with 'cafleet doctor'.",
            error.message()
        ))
    })?;
    finalize_deletion(conn, deadline, snapshot.runtime.is_some(), |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::broker::members::db_err)?;
        revalidate_member(&tx, snapshot, "after pane close")?;
        let changed = crate::broker::members::deregister_member_rows(&tx, member_id)?;
        assert!(
            changed,
            "the input-locked member stays active until deregistration"
        );
        tx.commit().map_err(crate::broker::members::db_err)
    })
    .map_err(|error| {
        CafleetError::App(format!(
            "pane {pane_id} was closed, but deregister failed: {}",
            error.message()
        ))
    })?;
    if let Err(error) = host.restore_layout(layout) {
        eprintln!(
            "Member {member_id} was deregistered; optional layout restoration failed: {}",
            error.message()
        );
    }
    Ok(format!("{pane_id} (killed)"))
}

pub fn delete_fleet(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    deadline: Deadline,
) -> Result<DeletedFleet, CafleetError> {
    delete_fleet_with_host(
        conn,
        settings,
        fleet_id,
        deadline,
        &mut NativeFleetDeletionHost {
            settings,
            mux: None,
        },
    )
}

pub(crate) fn delete_fleet_with_host(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    deadline: Deadline,
    host: &mut impl DeletionHost,
) -> Result<DeletedFleet, CafleetError> {
    with_database_budget(conn, deadline, |conn| {
        crate::broker::fleets::fetch_fleet(conn, fleet_id)?
            .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
        Ok(())
    })?;
    let changed_error = || {
        CafleetError::App(format!(
            "fleet {fleet_id} placement or runtime ownership changed during cleanup; fleet was not deleted."
        ))
    };
    let mut locked = lock_deletion(
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
        |conn, snapshot, _guards| {
            if deletion_placements(conn, settings, fleet_id)? != *snapshot {
                return Ok(None);
            }
            let mut members = Vec::new();
            for target in snapshot.iter() {
                let member = active_member(conn, target.member_id)?;
                if member.kind != crate::broker::records::MemberKind::Director {
                    members.push(member);
                }
            }
            Ok(Some(members))
        },
        |member| {
            CafleetError::App(format!(
                "fleet {fleet_id} pane input remained busy for 5 seconds at member {member}; fleet was not deleted."
            ))
        },
    )?;
    let mut remaining = locked.snapshot.clone();
    let mut members = std::mem::take(&mut locked.value);
    let mut ended = Vec::new();
    for member in &mut members {
        let member_id = member.member_id;
        let placement = member
            .placement
            .clone()
            .expect("a locked deletion target has a concrete placement");
        let held = locked.pane_lock(&settings.database_url, &placement)?;
        let mut scope_changed = false;
        let mut scope = |conn: &Connection| {
            if same_placements(&deletion_placements(conn, settings, fleet_id)?, &remaining) {
                Ok(())
            } else {
                scope_changed = true;
                Err(changed_error())
            }
        };
        match end_member(conn, settings, member, held, host, deadline, &mut scope) {
            Ok(pane_status) => {
                ended.push(DeletedMember {
                    member_id,
                    pane_status,
                });
                remaining.retain(|target| target.member_id != member_id);
            }
            Err(error) if scope_changed => return Err(error),
            Err(error) => return Err(member_not_ended(fleet_id, member_id, &error, &ended)),
        }
    }
    let (deregistered_count, unplaced) = with_database_budget(conn, deadline, |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::broker::members::db_err)?;
        if !same_placements(&deletion_placements(&tx, settings, fleet_id)?, &remaining) {
            return Err(changed_error());
        }
        let unplaced = unplaced_member_outcomes(&tx, fleet_id)?;
        let deregistered_count = crate::broker::fleets::delete_fleet_rows(&tx, fleet_id)?;
        deadline.remaining()?;
        tx.commit().map_err(crate::broker::members::db_err)?;
        Ok((deregistered_count, unplaced))
    })?;
    drop(locked);
    let deregistered_count = deregistered_count + ended.len();
    ended.extend(unplaced);
    ended.sort_by_key(|member| member.member_id);
    Ok(DeletedFleet {
        fleet_id,
        deregistered_count,
        members: ended,
    })
}

fn same_placements(current: &[DeletionPlacement], expected: &[DeletionPlacement]) -> bool {
    current.len() == expected.len()
        && current.iter().zip(expected).all(|(current, expected)| {
            current.member_id == expected.member_id && current.placement == expected.placement
        })
}

fn member_not_ended(
    fleet_id: i64,
    member_id: i64,
    cause: &CafleetError,
    ended: &[DeletedMember],
) -> CafleetError {
    let ended = if ended.is_empty() {
        "none".to_owned()
    } else {
        ended
            .iter()
            .map(|member| member.member_id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    CafleetError::App(format!(
        "fleet {fleet_id} was not deleted: member {member_id}: {}. Members already deleted: {ended}.",
        cause.message().trim_end_matches('.')
    ))
}

fn unplaced_member_outcomes(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<DeletedMember>, CafleetError> {
    let mut statement = conn
        .prepare(
            "SELECT m.member_id, p.member_id IS NOT NULL
             FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id
             WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NULL
               AND m.member_id IS NOT (SELECT director_member_id FROM fleets WHERE fleet_id=?1)",
        )
        .map_err(crate::broker::members::db_err)?;
    let rows = statement
        .query_map([fleet_id], |row| {
            Ok(DeletedMember {
                member_id: row.get(0)?,
                pane_status: if row.get(1)? {
                    PENDING_PANE_STATUS
                } else {
                    NO_PLACEMENT_STATUS
                }
                .to_owned(),
            })
        })
        .map_err(crate::broker::members::db_err)?;
    rows.collect::<Result<_, _>>()
        .map_err(crate::broker::members::db_err)
}
