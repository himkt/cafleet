use std::time::{Duration, Instant};

use super::*;
use crate::error::CafleetError;

#[test]
fn runner_shutdown_clock_anchors_all_work_to_one_original_instant() {
    let start = Instant::now();
    let clock = ShutdownClock::new(start);
    assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
    assert_eq!(clock.display_deadline(), start + Duration::from_secs(3));
    assert_eq!(clock.storage_deadline(), start + Duration::from_secs(4));
    for elapsed in [0, 1, 2, 3, 4, 9] {
        let now = start + Duration::from_secs(elapsed);
        assert_eq!(
            clock.remaining_storage_at(now),
            Duration::from_secs(4).saturating_sub(Duration::from_secs(elapsed))
        );
        assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
    }
}

#[test]
fn runner_exit_status_keeps_native_code_signal_and_cleanup_outcome_distinct() {
    for (native, complete, expected) in [
        (Some(ChildExit::Code(0)), true, 0),
        (Some(ChildExit::Code(7)), true, 7),
        (Some(ChildExit::Signal(15)), true, 143),
        (Some(ChildExit::Code(0)), false, 1),
        (Some(ChildExit::Code(7)), false, 7),
        (Some(ChildExit::Signal(15)), false, 143),
        (None, false, 1),
    ] {
        assert_eq!(runner_exit_status(native, complete).unwrap(), expected);
    }
    assert!(runner_exit_status(None, true).is_err());
}

#[test]
fn runner_database_deadline_bounds_a_locked_write_and_restores_the_busy_timeout() {
    let mut fixture = Fixture::new();
    fixture
        .conn
        .execute_batch("CREATE TABLE deadline_fixture(value INTEGER)")
        .unwrap();
    fixture
        .conn
        .busy_timeout(Duration::from_millis(137))
        .unwrap();
    let other = crate::db::connect(&format!("sqlite:///{}", fixture.conn.path().unwrap())).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = Instant::now();
    let result = with_database_deadline(
        &mut fixture.conn,
        start + Duration::from_millis(80),
        |conn| {
            conn.execute("INSERT INTO deadline_fixture VALUES(1)", [])
                .map(|_| ())
                .map_err(|error| CafleetError::App(error.to_string()))
        },
    );
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_millis(750));
    assert_eq!(
        fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        137
    );
    other.execute_batch("ROLLBACK").unwrap();
    fixture
        .conn
        .execute("INSERT INTO deadline_fixture VALUES(2)", [])
        .unwrap();
}

#[test]
fn runner_database_deadline_cancels_a_long_statement_and_removes_its_progress_handler() {
    let mut fixture = Fixture::new();
    let start = Instant::now();
    let result = with_database_deadline(
        &mut fixture.conn,
        start + Duration::from_millis(80),
        |conn| {
            conn.query_row("WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",[],|row|row.get::<_,i64>(0)).map_err(|error|CafleetError::App(error.to_string()))
        },
    );
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_millis(750));
    assert_eq!(fixture.conn.query_row("WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter",[],|row|row.get::<_,i64>(0)).unwrap(),50005000);
    assert_eq!(
        fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5000
    );
}

#[test]
fn runner_database_deadline_preserves_an_already_expired_shared_budget() {
    let mut fixture = Fixture::new();
    let mut invoked = false;
    let result = with_database_deadline(
        &mut fixture.conn,
        Instant::now() - Duration::from_secs(1),
        |_| {
            invoked = true;
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(!invoked);
}
