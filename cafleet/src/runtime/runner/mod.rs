mod input_protocol;
pub(crate) use input_protocol::*;
mod input_client;
pub(crate) use input_client::*;
mod input_clock;
mod input_recovery;
pub(crate) use input_clock::*;
pub(crate) use input_recovery::ExclusiveReceiptGuard;
mod input;
pub(crate) use input::InputGuard;
mod observation;
pub(crate) use observation::*;
mod outbox;
pub(crate) use outbox::*;
pub(crate) mod recovery;
pub(crate) use recovery::*;
pub(crate) use writer::{EventWriter, InertWriterHooks, WriterBoundary, WriterHooks};
pub(crate) mod episodes;
pub(crate) use crate::coding_agent::{
    MatchEvidence, ObservationKind, ScreenObservation, SemanticFingerprint,
};
pub(crate) use episodes::*;
mod artifacts;
mod codec;
mod control;
mod creation;
pub(crate) mod health;
mod lifecycle;
mod process;
pub(crate) mod service;
mod terminal;
mod transport;
mod writer;

pub(crate) use artifacts::*;
pub(crate) use codec::*;
pub(crate) use control::{NativeLaunchHost, request_shutdown};
pub(crate) use creation::*;
pub(crate) use health::*;
pub(crate) use lifecycle::*;
pub(crate) use process::*;
#[cfg(test)]
pub(crate) use service::{RunnerTestServices, run_with_test_services};
pub(crate) use service::{run, terminal_size};
pub(crate) use terminal::*;
pub(crate) use transport::*;

use crate::error::CafleetError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

type Result<T> = std::result::Result<T, CafleetError>;

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
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::placement;
    use crate::broker::runtime::{self as storage, *};
    use crate::db::test_support::migrated_conn;
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
            let dir = tempfile::Builder::new()
                .prefix(".p194-")
                .tempdir_in("/tmp")
                .unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "runner-contract");
            let mut placement = placement(Some("%2"));
            placement.coding_agent = "codex".into();
            let member = crate::broker::register_member(
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
                profile_version: None,
                profile_match: None,
                coverage: Coverage::Unknown,
                owner_pid: process.pid,
                owner_start_identity: process.start_identity,
            };
            let run = storage::create_run(&mut conn, &new).unwrap();
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

        pub(crate) fn spec(&self) -> SpawnSpec {
            SpawnSpec {
                argv: vec![
                    "/usr/bin/env".into(),
                    "literal assignment 世界 '$x' `text`".into(),
                ],
                cwd: self._dir.path().to_path_buf(),
                environment: vec![(
                    "CAFLEET_DATABASE_URL".into(),
                    format!("sqlite:///{}", self.conn.path().unwrap()),
                )],
                evidence_policy: json!({"mode":"metadata"}),
                size: TerminalSize { rows: 24, cols: 80 },
            }
        }
    }
}
