use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cafleet::error::CafleetError;
use cafleet::input::{Deadline, with_database_budget};
use rusqlite::{Connection, ErrorCode};
use tempfile::TempDir;

const PRIOR_TIMEOUT: Duration = Duration::from_millis(137);

struct Fixture {
    _directory: TempDir,
    path: PathBuf,
    conn: Connection,
}

impl Fixture {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("budget.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42);")
            .unwrap();
        conn.busy_timeout(PRIOR_TIMEOUT).unwrap();
        Self {
            _directory: directory,
            path,
            conn,
        }
    }

    fn assert_timeout_restored(&self) {
        let timeout: u32 = self
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(u128::from(timeout), PRIOR_TIMEOUT.as_millis());
    }
}

struct ExclusiveLock {
    release: Option<Sender<Duration>>,
    worker: Option<JoinHandle<()>>,
}

impl ExclusiveLock {
    fn new(path: &Path) -> Self {
        let path = path.to_path_buf();
        let (ready_sender, ready) = channel();
        let (release, command) = channel();
        let worker = thread::spawn(move || {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
            ready_sender.send(()).unwrap();
            let delay = command
                .recv_timeout(Duration::from_secs(9))
                .expect("exclusive-lock owner received its bounded release");
            thread::sleep(delay);
            conn.execute_batch("ROLLBACK").unwrap();
        });
        let owner = Self {
            release: Some(release),
            worker: Some(worker),
        };
        ready
            .recv_timeout(Duration::from_secs(2))
            .expect("exclusive SQLite lock was acquired before querying");
        owner
    }

    fn release_after(&mut self, delay: Duration) {
        assert!(delay <= Duration::from_secs(2));
        self.release.take().unwrap().send(delay).unwrap();
    }
}

impl Drop for ExclusiveLock {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(Duration::ZERO);
        }
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn read_probe(conn: &Connection) -> Result<i64, CafleetError> {
    conn.query_row("SELECT value FROM probe", [], |row| row.get(0))
        .map_err(|error| CafleetError::App(error.to_string()))
}

fn blocked_read(conn: &Connection) -> Result<i64, CafleetError> {
    let error = conn
        .query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert_eq!(error.sqlite_error_code(), Some(ErrorCode::DatabaseBusy));
    Err(CafleetError::App(error.to_string()))
}

#[derive(Clone, Copy)]
enum ScopeExit {
    Success,
    Error,
    Unwind,
}

fn exercise_scope_exit(conn: &mut Connection, exit: ScopeExit) {
    match exit {
        ScopeExit::Success => {
            assert_eq!(
                with_database_budget(conn, Deadline::new(), |conn| read_probe(conn)).unwrap(),
                42
            );
        }
        ScopeExit::Error => {
            let error = with_database_budget(conn, Deadline::new(), |conn| {
                conn.query_row("SELECT missing_column FROM probe", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| CafleetError::App(error.to_string()))
            })
            .unwrap_err();
            assert!(error.message().contains("no such column: missing_column"));
        }
        ScopeExit::Unwind => {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_database_budget(conn, Deadline::new(), |_| -> Result<i64, CafleetError> {
                    panic!("intentional database-scope unwind");
                })
            }));
            let payload = result.expect_err("the nested operation unwinds");
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"intentional database-scope unwind")
            );
        }
    }
}

#[derive(Clone, Copy)]
enum ScopeSequence {
    Statements,
    AfterNested(ScopeExit),
    NestedWait,
}

fn assert_absolute_wait(sequence: ScopeSequence) {
    let mut fixture = Fixture::new();
    let mut first = ExclusiveLock::new(&fixture.path);
    let path = fixture.path.clone();
    let start = Instant::now();
    let deadline = Deadline::new();
    first.release_after(Duration::from_secs(2));
    let error = with_database_budget(&mut fixture.conn, deadline, |conn| {
        assert_eq!(read_probe(conn)?, 42);
        drop(first);
        if let ScopeSequence::AfterNested(exit) = sequence {
            exercise_scope_exit(conn, exit);
        }
        let _second = ExclusiveLock::new(&path);
        if matches!(sequence, ScopeSequence::NestedWait) {
            with_database_budget(conn, Deadline::new(), |conn| blocked_read(conn))
        } else {
            blocked_read(conn)
        }
    })
    .unwrap_err();
    let elapsed = start.elapsed();
    assert_eq!(error.message(), "database is locked");
    assert!(
        deadline.remaining().is_err(),
        "the exclusive lock remained held through the deadline"
    );
    fixture.assert_timeout_restored();
    assert!(elapsed < Duration::from_millis(6500), "{elapsed:?}");
}

#[test]
fn two_statement_lock_waits_share_the_original_absolute_deadline() {
    assert_absolute_wait(ScopeSequence::Statements);
}

#[test]
fn prior_timeout_is_restored_after_success_error_and_unwind() {
    let mut fixture = Fixture::new();
    for exit in [ScopeExit::Success, ScopeExit::Error, ScopeExit::Unwind] {
        exercise_scope_exit(&mut fixture.conn, exit);
        fixture.assert_timeout_restored();
    }
}

#[test]
fn outer_deadline_handler_is_restored_after_nested_success() {
    assert_absolute_wait(ScopeSequence::AfterNested(ScopeExit::Success));
}

#[test]
fn outer_deadline_handler_is_restored_after_nested_error() {
    assert_absolute_wait(ScopeSequence::AfterNested(ScopeExit::Error));
}

#[test]
fn outer_deadline_handler_is_restored_after_nested_unwind() {
    assert_absolute_wait(ScopeSequence::AfterNested(ScopeExit::Unwind));
}

#[test]
fn a_nested_wait_retains_the_earlier_outer_deadline() {
    assert_absolute_wait(ScopeSequence::NestedWait);
}
