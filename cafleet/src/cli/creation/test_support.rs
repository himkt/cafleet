//! Test-only observations shared by the real fleet/member creation paths.

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use rusqlite::Connection;
use tempfile::TempDir;

use super::{CleanupEvent, CreationHooks, GuardResource};
use crate::{
    broker::{
        fleets::{BootstrapEvent, BootstrapHooks},
        test_support,
    },
    config::Settings,
    error::CafleetError,
    multiplexer::{AnyMultiplexer, CommandRunner, HerdrMultiplexer, RunError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Command { operation: String, succeeded: bool },
    WriteLockAvailable(bool),
    Bootstrap(BootstrapEvent),
    Cleanup(CleanupEvent),
    AfterRealRollback(Option<i64>),
}

pub(crate) struct Fixture {
    pub dir: TempDir,
    pub settings: Settings,
    pub events: Rc<RefCell<Vec<Event>>>,
    pub diagnostic: bool,
}

impl Fixture {
    pub fn new(member: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(".creation-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = test_support::migrated_conn(&dir);
        if member {
            test_support::create_fleet(&mut conn, "fixture");
        }
        let url = format!("sqlite:///{}", dir.path().join("broker_test.db").display());
        let settings =
            Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone()))
                .unwrap();
        Self {
            dir,
            settings,
            events: Rc::default(),
            diagnostic: false,
        }
    }

    pub fn conn(&self) -> Connection {
        crate::db::connect(&self.settings.database_url).unwrap()
    }

    pub fn sql(&self, sql: &str) {
        self.conn().execute_batch(sql).unwrap();
    }

    pub fn count(&self, table: &str) -> i64 {
        self.conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub fn assert_empty_bootstrap(&self) {
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(self.count(table), 0, "{table}");
        }
    }

    pub fn assert_deregistered(&self) {
        let conn = self.conn();
        let status: String = conn
            .query_row("SELECT status FROM members WHERE member_id=3", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "deregistered");
        assert_eq!(self.count("member_placements"), 2);
    }

    pub fn mux(
        &self,
        failure: Option<(&str, RunError)>,
        close_fails: bool,
        unknown: bool,
    ) -> AnyMultiplexer {
        AnyMultiplexer::Herdr(HerdrMultiplexer::new(
            Rc::new(Runner {
                events: self.events.clone(),
                url: self.settings.database_url.clone(),
                failure: failure.map(|(op, error)| (op.to_string(), error)),
                close_fails,
                unknown,
            }),
            HashMap::from([("HERDR_ENV".into(), "1".into())]),
        ))
    }

    pub fn timeline(&self) -> Vec<String> {
        self.events
            .borrow()
            .iter()
            .map(|event| match event {
                Event::Command {
                    operation,
                    succeeded,
                } => format!("{operation}:{}", if *succeeded { "ok" } else { "error" }),
                Event::WriteLockAvailable(available) => format!("write-lock:{available}"),
                Event::Bootstrap(BootstrapEvent::Begun) => "begin".into(),
                Event::Bootstrap(BootstrapEvent::CommitFinished { error, .. }) => {
                    format!("commit:{}", if error.is_none() { "ok" } else { "error" })
                }
                Event::Bootstrap(BootstrapEvent::RollbackFinished {
                    error, autocommit, ..
                }) => {
                    assert!(error.is_none() && *autocommit, "{event:?}");
                    "rollback:ok".into()
                }
                Event::AfterRealRollback(_) => "after-real-rollback".into(),
                Event::Cleanup(CleanupEvent::PaneKillFinished { error, .. }) => {
                    format!("cli-kill:{}", if error.is_none() { "ok" } else { "error" })
                }
                Event::Cleanup(CleanupEvent::DeregisterFinished { result, .. }) => format!(
                    "deregister:{}",
                    if result == &Ok(true) { "ok" } else { "error" }
                ),
                Event::Cleanup(CleanupEvent::GuardDisarmed {
                    resource: GuardResource::Pane { .. },
                }) => "disarm:pane".into(),
                Event::Cleanup(CleanupEvent::GuardDisarmed {
                    resource: GuardResource::Registration { .. },
                }) => "disarm:registration".into(),
            })
            .collect()
    }
}

impl BootstrapHooks for Fixture {
    fn observe(&self, event: BootstrapEvent) {
        self.events.borrow_mut().push(Event::Bootstrap(event));
    }

    fn after_rollback(&self, fleet_id: Option<i64>) -> Result<(), CafleetError> {
        // This verifies real recovery before injecting a diagnostic. It does
        // not simulate SQLite failing to execute ROLLBACK.
        assert_eq!(
            self.events.borrow().last(),
            Some(&Event::Bootstrap(BootstrapEvent::RollbackFinished {
                fleet_id,
                error: None,
                autocommit: true,
            }))
        );
        self.assert_empty_bootstrap();
        self.sql("BEGIN IMMEDIATE; ROLLBACK;");
        self.events
            .borrow_mut()
            .push(Event::AfterRealRollback(fleet_id));
        if self.diagnostic {
            Err(CafleetError::App("synthetic rollback diagnostic".into()))
        } else {
            Ok(())
        }
    }
}

impl CreationHooks for Fixture {
    fn observe_cleanup(&self, event: CleanupEvent) {
        if let CleanupEvent::DeregisterFinished {
            member_id,
            result: Ok(true),
        } = &event
        {
            let conn = self.conn();
            let status: String = conn
                .query_row(
                    "SELECT status FROM members WHERE member_id=?1",
                    [member_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                status, "deregistered",
                "event must follow the real DB operation"
            );
        }
        self.events.borrow_mut().push(Event::Cleanup(event));
    }
}

struct Runner {
    events: Rc<RefCell<Vec<Event>>>,
    url: String,
    failure: Option<(String, RunError)>,
    close_fails: bool,
    unknown: bool,
}

impl CommandRunner for Runner {
    fn binary_exists(&self, name: &str) -> bool {
        name == "herdr"
    }
    fn sleep(&self, _: f64) {
        panic!("creation must not send exit/notification keystrokes")
    }
    fn run(&self, argv: &[String], _: Option<u64>) -> Result<String, RunError> {
        assert_eq!(&argv[..2], &["herdr", "pane"]);
        let op = argv[2].as_str();
        let result = if let Some((_, error)) =
            self.failure.as_ref().filter(|(failed, _)| failed == op)
        {
            Err(error.clone())
        } else {
            match op {
                "current" => Ok(r#"{"result":{"pane":{"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1"}}}"#.into()),
                "list" => Ok(r#"{"result":{"panes":[{"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1"}]}}"#.into()),
                "split" if self.unknown => Ok(r#"{"result":{"pane":{}}}"#.into()),
                "split" => Ok(r#"{"result":{"pane":{"pane_id":"w1:p9"}}}"#.into()),
                "run" => Ok(String::new()),
                "get" => Err(RunError::Failed { stderr: "optional layout unavailable".into() }),
                "close" => {
                    assert_eq!(argv[3], "w1:p9", "never guess another pane");
                    let conn = crate::db::connect(&self.url).unwrap();
                    conn.busy_timeout(Duration::ZERO).unwrap();
                    let lock = match conn.execute_batch("BEGIN IMMEDIATE") {
                        Ok(()) => { conn.execute_batch("ROLLBACK").unwrap(); true }
                        Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::DatabaseBusy => false,
                        other => panic!("unexpected lock probe: {other:?}"),
                    };
                    self.events.borrow_mut().push(Event::WriteLockAvailable(lock));
                    if self.close_fails { Err(RunError::Failed { stderr: "secondary close failure".into() }) } else { Ok(String::new()) }
                }
                other => panic!("unexpected creation command: {other} {argv:?}"),
            }
        };
        self.events.borrow_mut().push(Event::Command {
            operation: op.into(),
            succeeded: result.is_ok(),
        });
        result
    }
}
