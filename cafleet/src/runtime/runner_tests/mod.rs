mod artifacts;
mod control_codec;
mod control_service;
mod creation;
mod fleet_deletion;
mod health;
mod launch_readiness;
mod lifecycle;
mod mechanisms;
mod shutdown;
mod terminal;
mod transport;

use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;

use super::runner::*;
use crate::broker::runtime::{self as storage, *};
use crate::broker::tests as common;

struct Fixture {
    _dir: TempDir,
    conn: Connection,
    run: RunRecord,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix(".p194-")
            .tempdir_in("/tmp")
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "runner-contract");
        let mut placement = common::placement(Some("%2"));
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
            resolved_mode: "never".into(),
            mode_source: ModeSource::Default,
            permission_argv_json: json!([
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write"
            ]),
            event_evidence: EventEvidence::Metadata,
            native_version: Some("0.163.0".into()),
            profile_version: None,
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

    fn artifacts(&mut self) -> RunArtifacts {
        let artifacts = RunArtifacts::create(&mut self.conn, self.run.run_id).unwrap();
        self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
        artifacts
    }

    fn spec(&self) -> SpawnSpec {
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
