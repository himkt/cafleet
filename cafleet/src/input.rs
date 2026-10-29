use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::broker::records::{MemberRecord, Placement};
use crate::config::Settings;
use crate::error::CafleetError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline(Instant);

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
    pub(crate) fn new() -> Self {
        Self::until(Instant::now() + Duration::from_secs(5))
    }

    pub(crate) fn until(instant: Instant) -> Self {
        Self(instant)
    }

    pub(crate) fn remaining(self) -> Result<Duration, CafleetError> {
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

    pub(crate) fn retry(self) -> bool {
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
        let previous: u32 = conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        conn.busy_handler(Some(wait_for_database_deadline))?;
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
                .map_err(crate::error::CafleetError::from)
        } else {
            self.conn
                .busy_timeout(self.previous_timeout)
                .map_err(crate::error::CafleetError::from)
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

pub(crate) fn with_database_budget<T>(
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
    CafleetError::merge_cleanup(result, restored)
}

pub(crate) struct PaneLock {
    file: File,
    path: PathBuf,
}

#[cfg(test)]
pub(crate) const RELEASED_LOCK_GRACE: Duration = Duration::from_secs(2);

pub(crate) fn lock_path(
    database_url: &str,
    placement: &Placement,
) -> Result<PathBuf, CafleetError> {
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
    let key = crate::digest::sha256_hex(key.as_bytes());
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
            .file
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
    pub(crate) fn try_acquire(
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
            Ok(()) => Ok(Some(Self { file, path })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(CafleetError::App(format!(
                "cannot lock pane input {}: {error}",
                path.display()
            ))),
        }
    }

    /// Retry a held lock every 50 ms until `lock_deadline`; `None` means it stayed busy.
    pub(crate) fn acquire_within(
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

    /// Acquire a lock the caller has just seen released. A child forked by a concurrent
    /// test shares the released descriptor's flock until its exec closes it, so retry briefly.
    #[cfg(test)]
    pub(crate) fn acquire_released(
        database_url: &str,
        placement: &Placement,
    ) -> Result<Option<Self>, CafleetError> {
        Self::acquire_within(
            database_url,
            placement,
            Deadline::until(Instant::now() + RELEASED_LOCK_GRACE),
        )
    }
}

pub(crate) struct LockedMember {
    pub(crate) member: MemberRecord,
    guard: PaneLock,
}

impl LockedMember {
    pub(crate) fn pane_lock(&self, database_url: &str) -> Result<&PaneLock, CafleetError> {
        let placement = self
            .member
            .placement
            .as_ref()
            .expect("locked member has a placement");
        self.guard.validate_binding(database_url, placement)?;
        Ok(&self.guard)
    }
}

pub(crate) struct LockedDeletion<S, T> {
    pub(crate) snapshot: S,
    pub(crate) value: T,
    _span: crate::signals::DeferTermination,
    guards: Vec<PaneLock>,
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
        pane_lock_for(&self.guards, database_url, placement)
    }

    pub(crate) fn snapshot_and_pane_lock(
        &mut self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<(&mut S, &PaneLock), CafleetError> {
        let guard = pane_lock_for(&self.guards, database_url, placement)?;
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
                guards,
            });
        }
        drop(span);
        drop(guards);
        deadline.remaining()?;
    }
}

pub(crate) fn lock_member(
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
        guard,
    }))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::time::Duration;

    use rusqlite::Connection;

    use super::*;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use crate::multiplexer::INPUT_RESERVATION;
    pub(crate) struct LockFixture {
        pub(crate) _dir: tempfile::TempDir,
        pub(crate) conn: Connection,
        pub(crate) settings: Settings,
        pub(crate) member: MemberRecord,
    }

    impl LockFixture {
        pub(crate) fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = create_fleet_fixture(&mut conn, "locks");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let member = crate::broker::get_member(&conn, worker, fleet)
                .unwrap()
                .unwrap();
            let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()), None);
            Self {
                _dir: dir,
                conn,
                settings,
                member,
            }
        }

        pub(crate) fn placement(&self) -> &Placement {
            self.member.placement.as_ref().unwrap()
        }

        pub(crate) fn hold_for(&self, hold: Option<Duration>) -> std::thread::JoinHandle<()> {
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

        pub(crate) fn writer_deadline() -> Deadline {
            Deadline::new().reserving(INPUT_RESERVATION)
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::runtime::runner::test_support::*;

    mod pane_guard {
        use super::*;
        use crate::input::{PaneLock, pane_lock_for};
        #[test]
        fn runner_recovery_pane_guard_validates_the_actual_database_placement_and_named_inode() {
            use std::os::unix::fs::PermissionsExt;
            let fixture = Fixture::new();
            let configuration = crate::config::Settings::for_database(
                std::path::Path::new(fixture.conn.path().unwrap()),
                None,
            );
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
            let held = PaneLock::try_acquire(&configuration.database_url, &placement)
                .unwrap()
                .unwrap();
            held.validate_binding(&configuration.database_url, &placement)
                .unwrap();
            for field in ["backend", "session", "pane"] {
                let mut changed = placement.clone();
                match field {
                    "backend" => changed.backend = "herdr".into(),
                    "session" => changed.mux_session.push_str("-foreign"),
                    "pane" => changed.mux_pane_id = Some("%foreign".into()),
                    _ => unreachable!(),
                }
                assert!(
                    held.validate_binding(&configuration.database_url, &changed)
                        .is_err()
                );
            }
            let sibling_path = fixture._dir.path().join("sibling.db");
            let mut sibling = Connection::open(&sibling_path).unwrap();
            crate::db::migrate_to_head(&mut sibling).unwrap();
            let sibling_url = format!("sqlite:///{}", sibling_path.display());
            assert!(held.validate_binding(&sibling_url, &placement).is_err());
            let path = crate::input::lock_path(&configuration.database_url, &placement).unwrap();
            let original = path.with_extension("retained");
            std::fs::rename(&path, &original).unwrap();
            std::fs::write(&path, b"replacement lock identity").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                held.validate_binding(&configuration.database_url, &placement)
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"replacement lock identity");
        }

        #[test]
        fn runner_recovery_pane_guard_selection_reuses_a_deduplicated_physical_guard() {
            let fixture = Fixture::new();
            let database_url = format!("sqlite:///{}", fixture.conn.path().unwrap());
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
            let guards = vec![
                PaneLock::try_acquire(&database_url, &placement)
                    .unwrap()
                    .unwrap(),
            ];
            let selected = pane_lock_for(&guards, &database_url, &placement).unwrap();
            assert!(std::ptr::eq(selected, &guards[0]));
            let mut shared = placement.clone();
            shared.mux_window_id = "@shared".into();
            assert!(std::ptr::eq(
                pane_lock_for(&guards, &database_url, &shared).unwrap(),
                &guards[0]
            ));
            assert!(
                PaneLock::try_acquire(&database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            assert!(pane_lock_for(&[], &database_url, &placement).is_err());
        }
    }
    mod database_scope_tests {
        use super::super::*;

        #[test]
        fn expired_completion_commits_uncontended_work() {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE probe(value INTEGER);")
                .unwrap();
            let expired = Deadline::until(Instant::now() - Duration::from_millis(1));
            assert!(with_database_budget(&mut conn, expired, |_| Ok(())).is_err());
            complete_database_budget(&mut conn, expired, |conn| {
                conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO probe VALUES(42); COMMIT;")
                    .map_err(crate::error::CafleetError::from)
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
                    let tx =
                        conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                    tx.execute("UPDATE probe SET value=42", [])?;
                    release.send(()).unwrap();
                    tx.commit().map_err(crate::error::CafleetError::from)
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
                    .map_err(crate::error::CafleetError::from)
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
                    with_database_budget(conn, Deadline::new(), |_| Err::<(), _>(
                        CafleetError::App("expected".into())
                    ))
                    .is_err()
                );
                assert_timeout(conn, 137);
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = with_database_budget(
                        conn,
                        Deadline::new(),
                        |_| -> Result<(), CafleetError> { panic!("scope unwind") },
                    );
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
                            .map_err(crate::error::CafleetError::from)
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
                    conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))?,
                    42
                );
                release.join().unwrap();
                let writer = Connection::open(&path).unwrap();
                writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
                conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .map_err(crate::error::CafleetError::from)
            })
            .unwrap_err();
            assert!(matches!(error, CafleetError::DatabaseBusy(_)));
            assert!(started.elapsed() >= Duration::from_millis(180));
            assert!(started.elapsed() < Duration::from_millis(700));
        }
    }
    mod pane_lock_wait_tests {
        use super::super::test_support::LockFixture;
        use super::super::*;
        use crate::multiplexer::INPUT_RESERVATION;

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
                PaneLock::acquire_released(&fixture.settings.database_url, fixture.placement())
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
            let _guard = PaneLock::acquire_released(&alias_url, &duplicate)
                .unwrap()
                .unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().ino(), metadata.ino());
            assert_eq!(std::fs::read(&path).unwrap(), b"operator marker");
        }
    }
}
