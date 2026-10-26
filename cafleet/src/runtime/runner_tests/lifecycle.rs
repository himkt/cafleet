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

#[derive(Debug, PartialEq)]
struct DeadlinePanic(u32);

fn progress_checked_sum(conn: &Connection) -> Result<i64, CafleetError> {
    conn.query_row(
        "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter",
        [],
        |row| row.get(0),
    )
    .map_err(|cause| CafleetError::App(cause.to_string()))
}

#[test]
fn runner_database_deadline_review_unwind_preserves_the_panic_and_restores_connection_reuse() {
    let mut fixture = Fixture::new();
    fixture
        .conn
        .busy_timeout(Duration::from_millis(137))
        .unwrap();
    let until = Instant::now() + Duration::from_millis(40);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_database_deadline(&mut fixture.conn, until, |_| -> Result<(), CafleetError> {
            std::panic::panic_any(DeadlinePanic(194));
        })
    }));
    let payload = match unwound {
        Err(payload) => payload,
        Ok(_) => panic!("the original operation panic must propagate"),
    };
    assert_eq!(
        payload.downcast_ref::<DeadlinePanic>(),
        Some(&DeadlinePanic(194))
    );
    std::thread::sleep(until.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
    assert_eq!(
        fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        137
    );
}

#[test]
fn runner_database_deadline_review_operation_error_stops_its_watchdog_and_restores_reuse() {
    let mut fixture = Fixture::new();
    fixture
        .conn
        .busy_timeout(Duration::from_millis(173))
        .unwrap();
    let until = Instant::now() + Duration::from_millis(100);
    let started = Instant::now();
    let error = with_database_deadline(&mut fixture.conn, until, |_| -> Result<(), CafleetError> {
        Err(CafleetError::Usage(
            "fixture deadline operation error".into(),
        ))
    })
    .unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert_eq!(error.message(), "fixture deadline operation error");
    assert!(started.elapsed() < Duration::from_millis(750));
    std::thread::sleep(until.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
    assert_eq!(
        fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        173
    );
}

#[test]
fn runner_database_deadline_review_inner_unwind_restores_the_live_outer_cancellation_scope() {
    let mut fixture = Fixture::new();
    let started = Instant::now();
    let outer_until = started + Duration::from_millis(400);
    let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
        let inner_until = Instant::now() + Duration::from_millis(40);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_database_deadline(conn, inner_until, |_| -> Result<(), CafleetError> {
                std::panic::panic_any(DeadlinePanic(195));
            })
        }));
        let payload = match unwound {
            Err(payload) => payload,
            Ok(_) => panic!("the inner operation panic must propagate"),
        };
        assert_eq!(
            payload.downcast_ref::<DeadlinePanic>(),
            Some(&DeadlinePanic(195))
        );
        std::thread::sleep(
            inner_until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
        );
        assert_eq!(progress_checked_sum(conn)?, 50005000);
        conn.query_row(
            "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|cause| CafleetError::App(cause.to_string()))
    });
    assert!(result.is_err());
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1250));
    assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
}

#[test]
fn runner_database_deadline_review_nested_later_deadline_keeps_the_earliest_outer_budget() {
    let mut fixture = Fixture::new();
    let started = Instant::now();
    let outer_until = started + Duration::from_millis(100);
    let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
        with_database_deadline(conn, started + Duration::from_secs(1), |conn| {
            conn.query_row(
                "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|cause| CafleetError::App(cause.to_string()))
        })
    });
    assert!(result.is_err());
    assert!(started.elapsed() >= Duration::from_millis(70));
    assert!(started.elapsed() < Duration::from_millis(750));
    assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
}

fn assert_retired_inner_scope_preserves_outer(operation_fails: bool) {
    let mut fixture = Fixture::new();
    let started = Instant::now();
    let outer_until = started + Duration::from_millis(400);
    let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
        let inner_until = Instant::now() + Duration::from_millis(40);
        let inner = with_database_deadline(conn, inner_until, |_| {
            if operation_fails {
                Err(CafleetError::Usage("fixture inner operation error".into()))
            } else {
                Ok(7)
            }
        });
        if operation_fails {
            let error = inner.unwrap_err();
            assert_eq!(error.exit_code(), 2);
            assert_eq!(error.message(), "fixture inner operation error");
        } else {
            assert_eq!(inner.unwrap(), 7);
        }
        std::thread::sleep(
            inner_until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
        );
        assert_eq!(progress_checked_sum(conn)?, 50005000);
        conn.query_row(
            "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|cause| CafleetError::App(cause.to_string()))
    });
    assert!(result.is_err());
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1250));
    assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
}

#[test]
fn runner_database_deadline_review_inner_success_restores_the_live_outer_scope() {
    assert_retired_inner_scope_preserves_outer(false);
}

#[test]
fn runner_database_deadline_review_inner_error_restores_the_live_outer_scope() {
    assert_retired_inner_scope_preserves_outer(true);
}
