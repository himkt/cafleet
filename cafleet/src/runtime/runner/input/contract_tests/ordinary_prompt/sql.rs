use super::*;
use crate::input::{Deadline, with_database_budget};
use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct SqlObservation {
    begins: AtomicUsize,
    run_read: AtomicBool,
    delay: Option<Instant>,
    entered: AtomicBool,
}
fn observed_sql<T>(
    conn: &mut Connection,
    state: Arc<SqlObservation>,
    body: impl FnOnce(&mut Connection) -> T,
) -> T {
    let authorization = state.clone();
    conn.authorizer(Some(move |context: AuthContext<'_>| {
        match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Begin,
            } => {
                authorization.begins.fetch_add(1, Ordering::SeqCst);
            }
            AuthAction::Read {
                table_name: "member_runs",
                ..
            } => {
                authorization.run_read.store(true, Ordering::SeqCst);
            }
            _ => {}
        }
        Authorization::Allow
    }))
    .unwrap();
    conn.progress_handler(
        1,
        Some(move || {
            if state.run_read.load(Ordering::SeqCst)
                && !state.entered.swap(true, Ordering::SeqCst)
                && let Some(until) = state.delay
            {
                std::thread::sleep(
                    until.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                );
            }
            false
        }),
    );
    let result = catch_unwind(AssertUnwindSafe(|| body(conn)));
    conn.progress_handler(0, None::<fn() -> bool>);
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    match result {
        Ok(result) => result,
        Err(panic) => resume_unwind(panic),
    }
}
fn sql_state(delay: Option<Instant>) -> Arc<SqlObservation> {
    Arc::new(SqlObservation {
        begins: AtomicUsize::new(0),
        run_read: AtomicBool::new(false),
        delay,
        entered: AtomicBool::new(false),
    })
}
fn timeout(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn step5_prompt_final_sql_is_nonwaiting_on_the_supplied_connection_and_fresh_drain_progresses() {
    with_host(|h| {
        let id = h.arm("prompt:contended");
        h.drain();
        h.fixture
            .conn
            .busy_timeout(Duration::from_millis(600))
            .unwrap();
        let blocker =
            crate::db::connect(&format!("sqlite:///{}", h.fixture.conn.path().unwrap())).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let state = sql_state(None);
        let run = h.fixture.run.clone();
        let until = h.until;
        let original_address = &h.fixture.conn as *const Connection as usize;
        let coordinator = &mut h.coordinator;
        let transport = &mut h.transport;
        let pty = &mut h.pty;
        let observation = &h.controller;
        let started = Instant::now();
        let pending = observed_sql(&mut h.fixture.conn, state.clone(), |conn| {
            with_database_budget(conn, Deadline::until(until), |conn| {
                assert_eq!(conn as *const Connection as usize, original_address);
                let result = coordinator
                    .advance(conn, &run, transport, pty, observation, until)
                    .map_err(|f| f.source)?;
                assert!(conn.is_autocommit());
                Ok(result)
            })
        })
        .unwrap();
        let elapsed = started.elapsed();
        assert!(pending.is_none());
        assert!(
            elapsed < Duration::from_millis(200),
            "final validation waits neither its600ms connection timeout nor the original request budget: {elapsed:?}"
        );
        assert!(
            state.begins.load(Ordering::SeqCst) > 0,
            "the supplied connection attempts its actual final transaction"
        );
        assert!(h.fixture.conn.is_autocommit());
        assert_eq!(timeout(&h.fixture.conn), 600);
        assert!(h.pty.program.is_empty());
        assert!(matches!(
            storage::get_input(&h.fixture.conn, id).unwrap().state,
            storage::InputState::InProgress
        ));
        assert_eq!(h.hooks.count("before"), 1);

        h.human.bytes.extend(b"human");
        h.pty.child.bytes.extend(SCREEN);
        let previous_seq = h.transport.snapshot().screen_seq;
        h.drain();
        assert_eq!(h.human.consumed, 5);
        assert_eq!(h.coordinator.manual_generation, 5);
        assert_eq!(h.pty.manual, b"human");
        assert!(h.transport.snapshot().screen_seq > previous_seq);
        assert!(h.advance().is_none());
        assert!(h.pty.program.is_empty());
        blocker.execute_batch("ROLLBACK").unwrap();
        assert!(
            h.advance().is_none(),
            "contention consumes the preceding drain"
        );
        h.drain();
        let result = submitted(h.advance().expect("fresh real drain after SQLite release"));
        assert!(matches!(result.status, SubmissionStatus::Accepted));
        assert_eq!(result.input_id, Some(id));
        assert_eq!(h.pty.program, vec![encoded()]);
        assert_eq!(h.hooks.count("before"), 1);
        assert_eq!(timeout(&h.fixture.conn), 600);
    });
}

#[test]
fn step5_prompt_inherited_expiry_refuses_before_sql_and_after_uncontended_sql_on_the_actual_connection()
 {
    for during in [false, true] {
        with_host(|h| {
            let id = h.arm("prompt:sql-expiry");
            h.drain();
            h.fixture
                .conn
                .busy_timeout(Duration::from_millis(123))
                .unwrap();
            let outer = Instant::now() + Duration::from_millis(50);
            let state = sql_state(during.then_some(outer));
            let run = h.fixture.run.clone();
            let until = h.until;
            let coordinator = &mut h.coordinator;
            let transport = &mut h.transport;
            let observation = &h.controller;
            let original_address = &h.fixture.conn as *const Connection as usize;
            let result = observed_sql(&mut h.fixture.conn, state.clone(), |conn| {
                with_database_budget(conn, Deadline::until(outer), |conn| {
                    assert_eq!(conn as *const Connection as usize, original_address);
                    if !during {
                        std::thread::sleep(
                            outer.saturating_duration_since(Instant::now())
                                + Duration::from_millis(1),
                        );
                    }
                    match coordinator.acquire_write_guard(conn, &run, transport, observation, until)
                    {
                        Err(failure) => {
                            assert_eq!(failure.run_id, Some(run.run_id));
                            assert_eq!(failure.input_id, Some(id));
                            Err(failure.source)
                        }
                        Ok(Some(guard)) => {
                            drop(guard);
                            Ok(true)
                        }
                        Ok(None) => Ok(false),
                    }
                })
            });
            assert!(matches!(result, Err(CafleetError::DeadlineExceeded)));
            assert_eq!(
                state.entered.load(Ordering::SeqCst),
                during,
                "actual member-run SQL entry is control-separated"
            );
            assert!(h.fixture.conn.is_autocommit());
            assert_eq!(timeout(&h.fixture.conn), 123);
            assert!(h.pty.offers.is_empty());
            assert!(matches!(
                storage::get_input(&h.fixture.conn, id).unwrap().state,
                storage::InputState::InProgress
            ));
        });
    }
}

#[test]
fn step5_prompt_guard_keeps_the_earliest_inherited_expiration_after_sql_scope_restoration() {
    with_host(|h| {
        let id = h.arm("prompt:guard-sql-bound");
        h.drain();
        let before = storage::get_input(&h.fixture.conn, id).unwrap();
        h.fixture
            .conn
            .busy_timeout(Duration::from_millis(321))
            .unwrap();
        let outer = Instant::now() + Duration::from_millis(80);
        let run = h.fixture.run.clone();
        let until = h.until;
        let coordinator = &mut h.coordinator;
        let transport = &mut h.transport;
        let observation = &h.controller;
        let mut guard = with_database_budget(&mut h.fixture.conn, Deadline::until(outer), |conn| {
            coordinator
                .acquire_write_guard(conn, &run, transport, observation, until)
                .map_err(|failure| failure.source)?
                .ok_or_else(|| {
                    CafleetError::App("healthy Armed operation yields a real guard".into())
                })
        })
        .unwrap();
        assert_eq!(timeout(&h.fixture.conn), 321);
        std::thread::sleep(
            outer.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        assert!(Instant::now() < until);
        let result = h
            .transport
            .write_programmatic(&mut h.pty, &encoded(), &mut guard, until);
        assert!(matches!(result, Err(CafleetError::DeadlineExceeded)));
        assert!(h.pty.offers.is_empty());
        drop(guard);
        let point = h.coordinator.retained_checkpoint(id).unwrap();
        assert!(matches!(point.stage, SubmissionStage::ZeroBytes));
        assert_eq!(point.written_bytes, 0);
        assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), before);
        assert!(h.fixture.conn.is_autocommit());
        assert_eq!(timeout(&h.fixture.conn), 321);
    });
}
