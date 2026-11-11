mod artifacts;
pub(crate) mod control;
mod input;
pub(crate) mod input_client;
mod lifecycle;
pub(crate) mod observation;
mod press;
mod process;
pub(crate) mod recovery;
pub(crate) mod service;
pub(crate) mod system;
mod transport;
mod writer;

pub(crate) use crate::coding_agent::MatchEvidence;
pub(crate) use crate::coding_agent::ObservationKind;
pub(crate) use crate::coding_agent::ScreenObservation;
pub(crate) use crate::coding_agent::SemanticFingerprint;
use crate::error::CafleetError;
pub(crate) use crate::error::Result;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MultiplexerError;
pub(crate) use crate::runtime::artifacts::*;
pub(crate) use crate::runtime::control::CONTROL_PROTOCOL_VERSION;
pub(crate) use crate::runtime::control::ControlDecoder;
pub(crate) use crate::runtime::control::HealthSnapshot;
pub(crate) use crate::runtime::control::InputControlOperation;
pub(crate) use crate::runtime::control::InputControlReply;
pub(crate) use crate::runtime::control::InputControlRequest;
pub(crate) use crate::runtime::control::LifecycleRequest;
pub(crate) use crate::runtime::control::MAX_CONTROL_FRAME;
pub(crate) use crate::runtime::control::MissingEvidence;
pub(crate) use crate::runtime::control::PermissionInspectInput;
pub(crate) use crate::runtime::control::PermissionInspection;
pub(crate) use crate::runtime::control::PermissionRespondInput;
pub(crate) use crate::runtime::control::PermissionResponseResult;
pub(crate) use crate::runtime::control::ReceiptAction;
pub(crate) use crate::runtime::control::ReceiptReply;
pub(crate) use crate::runtime::control::SubmissionResult;
pub(crate) use crate::runtime::control::SubmissionStage;
pub(crate) use crate::runtime::control::SubmissionStatus;
pub(crate) use crate::runtime::control::SubmitInput;
pub(crate) use crate::runtime::control::diagnose_runtime_at;
#[cfg(test)]
pub(crate) use crate::runtime::control::encode_control_frame;
pub(crate) use crate::runtime::control::probe_health;
pub(crate) use crate::runtime::control::request_shutdown;
pub(crate) use crate::runtime::input::*;
pub(crate) use crate::runtime::input_client::*;
pub(crate) use crate::runtime::lifecycle::*;
pub(crate) use crate::runtime::observation::*;
pub(crate) use crate::runtime::press::KeyAcknowledgment;
pub(crate) use crate::runtime::press::press_member;
pub(crate) use crate::runtime::process::*;
pub(crate) use crate::runtime::recovery::*;
pub(crate) use crate::runtime::service::run;
pub(crate) use crate::runtime::service::terminal_size;
use crate::runtime::system::SystemRunner;
pub(crate) use crate::runtime::transport::*;
pub(crate) use crate::runtime::writer::*;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;

/// The multiplexer that targets `placement`'s recorded backend and server.
pub(crate) fn mux_for_placement(
    placement: &crate::broker::Placement,
    deadline: Option<crate::time::Deadline>,
) -> Result<Box<dyn Multiplexer>> {
    let app = |cause: MultiplexerError| error(cause.to_string());
    let mux = crate::multiplexer::multiplexer_for_server(
        &placement.backend,
        &placement.mux_server,
        Rc::new(SystemRunner),
    )
    .map_err(app)?;
    let mux = match deadline {
        Some(deadline) => mux.bounded(deadline),
        None => mux,
    };
    mux.ensure_available().map_err(app)?;
    Ok(mux)
}

/// The multiplexer built from `fleet_id`'s recorded Director placement.
pub(crate) fn mux_for_fleet(
    conn: &rusqlite::Connection,
    fleet_id: i64,
    deadline: Option<crate::time::Deadline>,
) -> Result<Box<dyn Multiplexer>> {
    mux_for_placement(
        &crate::broker::director_placement(conn, fleet_id)?,
        deadline,
    )
}

pub(crate) fn error(message: impl Into<String>) -> CafleetError {
    CafleetError::App(message.into())
}

pub(crate) fn io_error(cause: impl std::fmt::Display) -> CafleetError {
    error(format!("member runtime: {cause}"))
}

pub(crate) fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(error(message))
    }
}

pub(crate) fn check_deadline(now: std::time::Instant, deadline: std::time::Instant) -> Result<()> {
    if now >= deadline {
        Err(CafleetError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

/// Sleep one polling tick, never past `deadline`.
pub(crate) fn nap_until(deadline: std::time::Instant) {
    std::thread::sleep(
        std::time::Duration::from_millis(2)
            .min(deadline.saturating_duration_since(std::time::Instant::now())),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerminalSize {
    pub(crate) rows: u16,
    pub(crate) cols: u16,
}

impl TerminalSize {
    pub(crate) fn validate(self) -> Result<()> {
        ensure(
            self.rows > 0 && self.cols > 0,
            "terminal dimensions must be positive",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpawnSpec {
    pub(crate) argv: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) environment: Vec<(String, String)>,
    pub(crate) evidence_policy: Value,
    pub(crate) size: TerminalSize,
}

impl SpawnSpec {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure(
            !self.argv.is_empty() && !self.argv[0].is_empty(),
            "native argv requires an executable",
        )?;
        ensure(
            self.argv.iter().all(|s| !s.contains('\0')),
            "native argv contains a NUL",
        )?;
        ensure(
            self.cwd.is_absolute() && self.cwd.is_dir(),
            "native cwd must be an existing absolute directory",
        )?;
        ensure(
            self.environment
                .iter()
                .all(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')),
            "invalid native environment override",
        )?;
        ensure(
            self.evidence_policy.is_object(),
            "evidence policy must be an object",
        )?;
        self.size.validate()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use crate::runtime::artifacts::test_support::short_tempdir;
    pub(crate) use crate::runtime::lifecycle::test_support::SLOW_SUM;
    pub(crate) use crate::runtime::process::test_support::FixtureProcessTable;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::placement;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::db::test_support::migrated_conn;
    use crate::runtime::*;
    pub(crate) const STARTED_AT: &str = "2026-10-27T14:00:00.000000+00:00";

    pub(crate) fn mark_exited(conn: &Connection, run_id: i64) {
        assert_eq!(
            conn.execute(
                "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=7,ended_at='2026-10-27T18:13:00Z' WHERE run_id=?1",
                [run_id],
            )
            .unwrap(),
            1
        );
    }

    pub(crate) fn read_fixture_bytes(
        stream: &mut std::os::unix::net::UnixStream,
        mut bytes: &mut [u8],
        deadline: std::time::Instant,
    ) {
        use std::io::Read;
        while !bytes.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture request read timed out"
            );
            match stream.read(bytes) {
                Ok(0) => panic!("actual health client closed an incomplete request"),
                Ok(count) => bytes = &mut bytes[count..],
                Err(cause)
                    if matches!(
                        cause.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    std::thread::sleep(
                        std::time::Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                Err(cause) => panic!("fixture request read failed: {cause}"),
            }
        }
    }

    pub(crate) fn write_fixture_bytes(
        stream: &mut std::os::unix::net::UnixStream,
        mut bytes: &[u8],
        deadline: std::time::Instant,
    ) {
        use std::io::Write;
        while !bytes.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture reply write timed out"
            );
            match stream.write(bytes) {
                Ok(0) => panic!("fixture reply write made no progress"),
                Ok(count) => bytes = &bytes[count..],
                Err(cause)
                    if matches!(
                        cause.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    std::thread::sleep(
                        std::time::Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                Err(cause) => panic!("fixture reply write failed: {cause}"),
            }
        }
    }

    pub(crate) struct Fixture {
        pub(crate) _dir: TempDir,
        pub(crate) conn: Connection,
        pub(crate) run: RunRecord,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let dir = short_tempdir();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "runner-contract");
            let mut placement = placement(Some("%2"));
            placement.coding_agent = "codex".into();
            let member = crate::broker::members::test_support::register_member(
                &mut conn,
                fleet,
                "worker",
                "assignment",
                Some(&placement),
            )
            .unwrap()
            .member_id;
            let process = ProcessIdentity::current().unwrap();
            let identity = storage::database_identity(&conn).unwrap();
            let new = NewRun {
                member_id: member,
                fleet_id: fleet,
                director_member_id: director,
                backend: "codex".into(),
                database_identity_json: identity.clone(),
                launch_nonce: "a".repeat(64),
                runtime_path: RuntimePaths::for_run(&identity, 1)
                    .unwrap()
                    .run_dir
                    .to_str()
                    .unwrap()
                    .into(),
                requested_mode: None,
                resolved_mode: "approve-for-me".into(),
                mode_source: ModeSource::Default,
                permission_argv_json: json!(["--approve-for-me"]),
                event_evidence: EventEvidence::Metadata,
                native_version: None,
                coverage: Coverage::Unknown,
                owner_pid: process.pid,
                owner_start_identity: process.start_identity,
            };
            let run = storage::test_support::create_run(&mut conn, &new).unwrap();
            Self {
                _dir: dir,
                conn,
                run,
            }
        }

        pub(crate) fn artifacts(&mut self) -> RunArtifacts {
            let artifacts = RunArtifacts::create(&mut self.conn, self.run.run_id).unwrap();
            self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
            artifacts
        }

        /// Creates the run's artifacts, transfers its lock to this process as the runner and
        /// marks the run `running`.
        pub(crate) fn running_owner(&mut self) -> (RunArtifacts, RunLock, RunOwner) {
            let artifacts = self.artifacts();
            let lock = RunLock::acquire(&self.conn, &self.run.owner())
                .unwrap()
                .unwrap();
            let owner = lock
                .transfer_to_runner(
                    &mut self.conn,
                    &self.run.owner(),
                    ProcessIdentity::current().unwrap(),
                )
                .unwrap();
            assert_eq!(
                self.conn
                    .execute(
                        "UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2",
                        rusqlite::params![STARTED_AT, owner.run_id],
                    )
                    .unwrap(),
                1
            );
            self.run = storage::get_run(&self.conn, owner.run_id).unwrap();
            (artifacts, lock, owner)
        }

        pub(crate) fn spec(&self) -> SpawnSpec {
            SpawnSpec {
                argv: vec![
                    "/usr/bin/env".into(),
                    "literal assignment 世界 '$x' `text`".into(),
                ],
                cwd: self._dir.path().to_path_buf(),
                environment: Vec::new(),
                evidence_policy: json!({"mode":"metadata"}),
                size: TerminalSize { rows: 24, cols: 80 },
            }
        }
    }
}
