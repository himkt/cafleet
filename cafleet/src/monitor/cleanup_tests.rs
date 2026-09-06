use super::*;
use crate::broker::test_support;
use std::{cell::Cell, io, rc::Rc};

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

struct Handle(Rc<Cell<usize>>);
impl MonitorSignalHandle for Handle {
    fn unregister(self: Box<Self>) -> bool {
        self.0.set(self.0.get() + 1);
        true
    }
}

struct Writer(&'static str);
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0 == "write" {
            Err(io::Error::other("write failure"))
        } else {
            Ok(bytes.len())
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.0 == "flush" {
            Err(io::Error::other("flush failure"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn loop_exits_unregister_signals_and_clear_only_the_owned_runtime() {
    for failure in [
        "none",
        "first signal",
        "second signal",
        "write",
        "flush",
        "tick",
    ] {
        for clear_fails in [false, true] {
            let mut conn = Connection::open_in_memory().unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            let (fleet, _) = test_support::create_fleet(&mut conn, "cleanup");
            conn.execute("INSERT INTO monitor_runtime(fleet_id, tick_seconds, last_wake_at) VALUES (?1,5,'retained ledger')", [fleet]).unwrap();
            if failure == "tick" {
                conn.execute_batch("CREATE TRIGGER reject_tick BEFORE UPDATE OF last_tick_at ON monitor_runtime WHEN OLD.pid IS NOT NULL AND NEW.pid IS NOT NULL BEGIN SELECT RAISE(ABORT,'tick failure'); END;").unwrap();
            }
            if clear_fails {
                conn.execute_batch("CREATE TRIGGER reject_clear BEFORE UPDATE OF pid ON monitor_runtime WHEN NEW.pid IS NULL BEGIN SELECT RAISE(ABORT,'clear failure'); END;").unwrap();
            }
            let removed = Rc::new(Cell::new(0));
            let register =
                |signal, stop: Arc<AtomicBool>| -> io::Result<Box<dyn MonitorSignalHandle>> {
                    if (failure == "first signal" && signal == MonitorSignal::Terminate)
                        || (failure == "second signal" && signal == MonitorSignal::Interrupt)
                    {
                        return Err(io::Error::other("signal failure"));
                    }
                    if failure != "tick" {
                        stop.store(true, Ordering::Relaxed);
                    }
                    Ok(Box::new(Handle(removed.clone())))
                };
            let result = run_monitor_loop_with_hooks(
                &mut conn,
                &NoWake,
                &mut Writer(failure),
                fleet,
                5,
                0,
                &MonitorLoopHooks {
                    pid: 424242,
                    stop: Arc::new(AtomicBool::new(false)),
                    now: &now_utc,
                    register: &register,
                    sleep: &|_, _| panic!("fixture must exit before sleeping"),
                },
            );
            assert_eq!(
                result.is_ok(),
                failure == "none" && !clear_fails,
                "{failure}/{clear_fails}"
            );
            if let Err(error) = result {
                let primary = if failure.contains("signal") {
                    "signal"
                } else if failure == "none" {
                    "clear"
                } else {
                    failure
                };
                assert!(error.to_string().contains(primary), "{error}");
                if clear_fails {
                    assert!(error.to_string().contains("clear failure"));
                }
            }
            assert_eq!(
                removed.get(),
                match failure {
                    "first signal" => 0,
                    "second signal" => 1,
                    _ => 2,
                }
            );
            let runtime = broker::read_monitor_runtime(&conn, fleet).unwrap().unwrap();
            assert_eq!(runtime.pid, clear_fails.then_some(424242));
            assert_eq!(runtime.last_wake_at.as_deref(), Some("retained ledger"));
        }
    }
}
