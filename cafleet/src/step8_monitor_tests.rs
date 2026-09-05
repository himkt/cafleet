//! Step 8 approved monitor driver contracts; connect from lib.rs in Phase B.
use crate::{
    broker::{self, test_support},
    error::CafleetError,
    monitor::{
        MonitorEvent, MonitorLoopHooks, MonitorMux, MonitorSignal, MonitorSignalHandle, TickResult,
        run_monitor_loop_with_hooks,
    },
    multiplexer::{MultiplexerError, WakeEntry},
};
use chrono::{DateTime, Utc};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    io::{self, Write},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const WHEN: &str = "2026-09-06T01:02:03.000007+00:00";
const PID: i64 = 424242;
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(WHEN)
        .unwrap()
        .with_timezone(&Utc)
}
struct NoWake;
impl MonitorMux for NoWake {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
        panic!("wake interval is zero")
    }
    fn send_wake_entries(
        &self,
        _: &str,
        _: i64,
        _: &[WakeEntry<'_>],
        _: &WakeEntry<'_>,
    ) -> Result<bool, MultiplexerError> {
        panic!("wake interval is zero")
    }
}
struct Handle {
    signal: MonitorSignal,
    log: Rc<RefCell<Vec<String>>>,
}
impl MonitorSignalHandle for Handle {
    fn unregister(self: Box<Self>) -> bool {
        self.log
            .borrow_mut()
            .push(format!("unregister:{:?}", self.signal));
        self.signal == MonitorSignal::Terminate
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    None,
    FirstSignal,
    SecondSignal,
    Write,
    Flush,
    Tick,
    Replace,
}
struct Writer {
    failure: Failure,
    bytes: Vec<u8>,
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failure == Failure::Write {
            return Err(io::Error::other("startup write sentinel"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.failure == Failure::Flush {
            Err(io::Error::other("startup flush sentinel"))
        } else {
            Ok(())
        }
    }
}
struct Case {
    _dir: tempfile::TempDir,
    conn: rusqlite::Connection,
    fleet: i64,
    log: Rc<RefCell<Vec<String>>>,
    output: Vec<u8>,
    result: Result<(), CafleetError>,
}
impl Case {
    fn assert_cleanup(&self, handles: usize, clear_failed: bool, replacement: bool) {
        let log = self.log.borrow();
        assert_eq!(
            log.iter().filter(|s| s.starts_with("unregister:")).count(),
            handles,
            "{log:?}"
        );
        assert_eq!(
            log.iter()
                .filter(|s| s.starts_with("unregistered:"))
                .count(),
            handles,
            "{log:?}"
        );
        assert_eq!(
            log.iter().filter(|s| s.starts_with("clear:")).count(),
            1,
            "{log:?}"
        );
        assert_eq!(
            log.iter().filter(|s| s.starts_with("registered:")).count(),
            handles
        );
        let clear = log.iter().position(|s| s.starts_with("clear:")).unwrap();
        assert!(
            log.iter()
                .enumerate()
                .filter(|(_, s)| s.starts_with("unregister:"))
                .all(|(i, _)| i < clear)
        );
        for signal in ["Terminate", "Interrupt"].into_iter().take(handles) {
            assert_eq!(
                log.iter()
                    .filter(|s| **s == format!("unregister:{signal}"))
                    .count(),
                1
            );
            assert!(log.contains(&format!("unregistered:{signal}:{}", signal == "Terminate")));
        }
        let runtime = broker::read_monitor_runtime_record(&self.conn, self.fleet)
            .unwrap()
            .unwrap();
        assert_eq!(
            runtime.pid,
            if replacement {
                Some(PID + 1)
            } else if clear_failed {
                Some(PID)
            } else {
                None
            }
        );
        assert_eq!(
            runtime.last_wake_at.as_deref(),
            Some("retained wake ledger")
        );
        assert_eq!(
            runtime.wake_requested_at.as_deref(),
            Some("retained request")
        );
        if !clear_failed && !replacement {
            assert!(runtime.started_at.is_none() && runtime.last_tick_at.is_none());
        }
        if replacement {
            assert_eq!(runtime.started_at.as_deref(), Some("replacement start"));
        }
    }
}
fn run(failure: Failure, clear_failed: bool) -> Case {
    let dir = tempfile::Builder::new()
        .prefix(".step8-monitor-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let mut conn = test_support::migrated_conn(&dir);
    let (fleet, _) = test_support::create_fleet(&mut conn, "lease");
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let register = |signal, flag: Arc<AtomicBool>| -> io::Result<Box<dyn MonitorSignalHandle>> {
        assert!(Arc::ptr_eq(&flag, &stop));
        log.borrow_mut().push(format!("register:{signal:?}"));
        if (failure == Failure::FirstSignal && signal == MonitorSignal::Terminate)
            || (failure == Failure::SecondSignal && signal == MonitorSignal::Interrupt)
        {
            return Err(io::Error::other("signal registration sentinel"));
        }
        if signal == MonitorSignal::Interrupt
            && !matches!(failure, Failure::Tick | Failure::Replace)
        {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(Box::new(Handle {
            signal,
            log: log.clone(),
        }))
    };
    let observe = |event| {
        match event {
            MonitorEvent::Claimed { conn } => {
                assert!(conn.is_autocommit());
                assert_eq!(
                    broker::read_monitor_runtime_record(conn, fleet)
                        .unwrap()
                        .unwrap()
                        .pid,
                    Some(PID)
                );
                conn.execute("UPDATE monitor_runtime SET last_wake_at='retained wake ledger',wake_requested_at='retained request' WHERE fleet_id=?1",[fleet]).unwrap();
                // Avoid forcing a wake during the two tick paths; restore the request
                // after the completed tick so cleanup still proves its preservation.
                if matches!(failure, Failure::Tick | Failure::Replace) {
                    conn.execute(
                        "UPDATE monitor_runtime SET wake_requested_at=NULL WHERE fleet_id=?1",
                        [fleet],
                    )
                    .unwrap();
                }
                if clear_failed {
                    conn.execute_batch("CREATE TRIGGER reject_clear BEFORE UPDATE OF pid ON monitor_runtime WHEN NEW.pid IS NULL BEGIN SELECT RAISE(ABORT,'clear sentinel'); END;").unwrap();
                }
                if failure == Failure::Tick {
                    conn.execute_batch("CREATE TRIGGER reject_tick BEFORE UPDATE OF last_tick_at ON monitor_runtime WHEN NEW.pid IS NOT NULL BEGIN SELECT RAISE(ABORT,'tick sentinel'); END;").unwrap();
                }
                log.borrow_mut().push("claimed".into());
            }
            MonitorEvent::SignalRegistered { signal } => {
                log.borrow_mut().push(format!("registered:{signal:?}"))
            }
            MonitorEvent::SignalUnregistered { signal, removed } => log
                .borrow_mut()
                .push(format!("unregistered:{signal:?}:{removed}")),
            MonitorEvent::StartupWriteFinished { result } => {
                log.borrow_mut().push(format!("write:{}", result.is_ok()))
            }
            MonitorEvent::StartupFlushFinished { result } => {
                log.borrow_mut().push(format!("flush:{}", result.is_ok()))
            }
            MonitorEvent::TickFinished { conn, result } => {
                let label = match result {
                    Ok(TickResult::Continue) => "continue",
                    Ok(TickResult::Stop) => "stop",
                    Err(_) => "error",
                };
                log.borrow_mut().push(format!("tick:{label}"));
                if failure == Failure::Replace && matches!(result, Ok(TickResult::Continue)) {
                    conn.execute("UPDATE monitor_runtime SET pid=?1,started_at='replacement start',wake_requested_at='retained request' WHERE fleet_id=?2",[PID+1,fleet]).unwrap();
                } else if failure == Failure::Tick {
                    assert!(
                        result
                            .as_ref()
                            .err()
                            .unwrap()
                            .to_string()
                            .contains("tick sentinel")
                    );
                    conn.execute("UPDATE monitor_runtime SET wake_requested_at='retained request' WHERE fleet_id=?1",[fleet]).unwrap();
                }
            }
            MonitorEvent::ClearFinished { conn, result } => {
                assert_eq!(result.is_err(), clear_failed);
                let runtime = broker::read_monitor_runtime_record(conn, fleet)
                    .unwrap()
                    .unwrap();
                if !clear_failed && failure != Failure::Replace {
                    assert_eq!(runtime.pid, None);
                }
                log.borrow_mut().push(format!("clear:{}", result.is_ok()));
            }
        }
    };
    let sleep = |seconds, _: &AtomicBool| {
        assert_eq!(seconds, 5);
        log.borrow_mut().push("sleep".into());
        assert!(failure == Failure::Replace, "unexpected sleep");
    };
    let hooks = MonitorLoopHooks {
        pid: PID,
        stop: stop.clone(),
        now: &now,
        register: &register,
        sleep: &sleep,
        observe: &observe,
    };
    let mut writer = Writer {
        failure,
        bytes: Vec::new(),
    };
    let result = run_monitor_loop_with_hooks(&mut conn, &NoWake, &mut writer, fleet, 5, 0, &hooks);
    Case {
        _dir: dir,
        conn,
        fleet,
        log,
        output: writer.bytes,
        result,
    }
}
#[test]
fn first_signal_failure_clears_claim_without_any_handle() {
    let case = run(Failure::FirstSignal, false);
    case.assert_cleanup(0, false, false);
    assert!(
        case.result
            .as_ref()
            .unwrap_err()
            .to_string()
            .starts_with("cannot install the signal handler: signal registration sentinel")
    );
    assert!(case.output.is_empty());
}
#[test]
fn second_signal_failure_unregisters_the_first_exactly_once_before_clear() {
    let case = run(Failure::SecondSignal, false);
    case.assert_cleanup(1, false, false);
    assert!(
        case.result
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("signal registration sentinel")
    );
    assert!(case.output.is_empty());
}
#[test]
fn startup_write_failure_unregisters_both_and_clears_claim() {
    let case = run(Failure::Write, false);
    case.assert_cleanup(2, false, false);
    assert_eq!(
        case.result.as_ref().unwrap_err().to_string(),
        "stdout write failed: startup write sentinel"
    );
    assert!(case.log.borrow().contains(&"write:false".into()));
    assert!(!case.log.borrow().iter().any(|e| e.starts_with("flush:")));
}
#[test]
fn startup_flush_failure_is_an_error_with_full_lease_cleanup() {
    let case = run(Failure::Flush, false);
    case.assert_cleanup(2, false, false);
    assert_eq!(
        case.result.as_ref().unwrap_err().to_string(),
        "stdout flush failed: startup flush sentinel"
    );
    assert!(case.log.borrow().contains(&"flush:false".into()));
    assert!(!case.log.borrow().iter().any(|e| e.starts_with("tick:")));
}
#[test]
fn real_tick_sql_failure_preserves_primary_and_cleans_lease() {
    let case = run(Failure::Tick, false);
    case.assert_cleanup(2, false, false);
    assert!(
        case.result
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("tick sentinel")
    );
    assert!(case.log.borrow().contains(&"tick:error".into()));
}
#[test]
fn all_primary_failure_paths_append_clear_failure_without_retry() {
    for failure in [
        Failure::FirstSignal,
        Failure::SecondSignal,
        Failure::Write,
        Failure::Flush,
        Failure::Tick,
    ] {
        let case = run(failure, true);
        case.assert_cleanup(
            match failure {
                Failure::FirstSignal => 0,
                Failure::SecondSignal => 1,
                _ => 2,
            },
            true,
            false,
        );
        let error = case.result.as_ref().unwrap_err().to_string();
        let primary = match failure {
            Failure::FirstSignal | Failure::SecondSignal => "signal registration sentinel",
            Failure::Write => "startup write sentinel",
            Failure::Flush => "startup flush sentinel",
            _ => "tick sentinel",
        };
        let annotation = format!(
            "cleanup failed for monitor runtime (fleet {}, pid {PID}):",
            case.fleet
        );
        assert!(
            error.find(primary).unwrap() < error.find(&annotation).unwrap(),
            "{error}"
        );
        assert!(error.contains("clear sentinel"));
        assert_eq!(error.matches(&annotation).count(), 1);
    }
}
#[test]
fn successful_loop_with_clear_failure_returns_original_clear_error() {
    let case = run(Failure::None, true);
    case.assert_cleanup(2, true, false);
    let error = case.result.as_ref().unwrap_err().to_string();
    assert!(error.contains("clear sentinel"));
    assert!(!error.contains("cleanup failed for monitor runtime"));
}
#[test]
fn normal_stop_preserves_wake_ledger_and_has_no_leaked_handles_on_repeat() {
    let mut case = run(Failure::None, false);
    case.assert_cleanup(2, false, false);
    assert!(case.result.is_ok());
    let expected = format!(
        "monitor loop started (fleet {}, tick 5s, pid {PID})\n",
        case.fleet
    );
    assert_eq!(case.output, expected.as_bytes());
    // Reuse the cleared runtime and the same open connection. Each re-claim
    // owns two new handles and releases both before the next invocation.
    for iteration in 1..=2 {
        let stop = Arc::new(AtomicBool::new(false));
        let register =
            |signal, flag: Arc<AtomicBool>| -> io::Result<Box<dyn MonitorSignalHandle>> {
                if signal == MonitorSignal::Interrupt {
                    flag.store(true, Ordering::Relaxed);
                }
                Ok(Box::new(Handle {
                    signal,
                    log: case.log.clone(),
                }))
            };
        let hooks = MonitorLoopHooks {
            pid: PID,
            stop,
            now: &now,
            register: &register,
            sleep: &|_, _| panic!("already stopped"),
            observe: &|_| {},
        };
        let mut out = Vec::new();
        run_monitor_loop_with_hooks(&mut case.conn, &NoWake, &mut out, case.fleet, 5, 0, &hooks)
            .unwrap();
        assert_eq!(out, expected.as_bytes());
        assert_eq!(
            case.log
                .borrow()
                .iter()
                .filter(|event| event.starts_with("unregister:"))
                .count(),
            2 * (iteration + 1)
        );
        assert_eq!(
            broker::read_monitor_runtime_record(&case.conn, case.fleet)
                .unwrap()
                .unwrap()
                .pid,
            None
        );
    }
}
#[test]
fn owner_replacement_stops_next_tick_and_cleanup_leaves_new_owner_untouched() {
    let case = run(Failure::Replace, false);
    case.assert_cleanup(2, false, true);
    assert!(case.result.is_ok());
    assert_eq!(
        case.log
            .borrow()
            .iter()
            .filter(|e| e.starts_with("tick:"))
            .cloned()
            .collect::<Vec<_>>(),
        ["tick:continue", "tick:stop"]
    );
    assert_eq!(
        case.log.borrow().iter().filter(|e| *e == "sleep").count(),
        1
    );
}
#[test]
fn refused_claim_never_registers_handles_writes_or_clears_existing_owner() {
    let dir = tempfile::Builder::new()
        .prefix(".step8-monitor-refusal-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let mut conn = test_support::migrated_conn(&dir);
    let (fleet, _) = test_support::create_fleet(&mut conn, "refused");
    let owner = i64::from(std::process::id());
    assert!(broker::claim_monitor_runtime(&mut conn, fleet, owner, 5, 0, WHEN).unwrap());
    conn.execute("UPDATE monitor_runtime SET last_wake_at='wake',wake_requested_at='request' WHERE fleet_id=?1",[fleet]).unwrap();
    let before = broker::read_monitor_runtime_record(&conn, fleet).unwrap();
    let hooks = MonitorLoopHooks {
        pid: PID,
        stop: Arc::new(AtomicBool::new(false)),
        now: &now,
        register: &|_, _| panic!("claim refused"),
        sleep: &|_, _| panic!("claim refused"),
        observe: &|_| panic!("no completed lease operation"),
    };
    let mut out = Vec::new();
    let error =
        run_monitor_loop_with_hooks(&mut conn, &NoWake, &mut out, fleet, 5, 0, &hooks).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("monitor already running for fleet {fleet}")
    );
    assert!(out.is_empty());
    assert_eq!(
        broker::read_monitor_runtime_record(&conn, fleet).unwrap(),
        before
    );
}
