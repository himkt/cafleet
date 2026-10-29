use super::*;
use crate::broker::runtime::{self as storage, RunRecord};
use rusqlite::Connection;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HealthSnapshot {
    pub identity: ControlIdentity,
    pub runner: ProcessIdentity,
    pub child: Option<ProcessIdentity>,
    pub output_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationHealth>,
}
impl HealthSnapshot {
    pub(crate) fn decode(value: Value, run: &RunRecord) -> Result<Self> {
        let result: Self = serde_json::from_value(value).map_err(io_error)?;
        result.validate(run)?;
        Ok(result)
    }
    pub(crate) fn validate(&self, run: &RunRecord) -> Result<()> {
        self.identity.validate(run)?;
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        ensure(
            run.runner_pid == Some(self.runner.pid)
                && run.runner_start_identity.as_deref()
                    == Some(self.runner.start_identity.as_str())
                && run.owner_pid == self.runner.pid
                && run.owner_start_identity == self.runner.start_identity
                && self.runner.matches()?,
            "health runner generation mismatch",
        )?;
        match &self.child {
            Some(child) => ensure(
                run.child_pid == Some(child.pid)
                    && run.child_start_identity.as_deref() == Some(child.start_identity.as_str())
                    && child.matches()?,
                "health child generation mismatch",
            ),
            None => ensure(
                run.child_pid.is_none(),
                "health omitted native child identity",
            ),
        }
    }
}
pub(crate) fn probe_health(run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot> {
    let reply = super::service::request(run, "health", deadline)?;
    HealthSnapshot::decode(reply, run)
}
pub(crate) struct RuntimeDiagnosis {
    pub diagnostics: Vec<String>,
}
pub(crate) fn diagnose_runtime(
    conn: &Connection,
    run_id: i64,
    deadline: Instant,
) -> Result<RuntimeDiagnosis> {
    let source = conn
        .path()
        .ok_or_else(|| error("runtime diagnosis requires a file-backed registry"))?;
    let mut observed = crate::db::connect_read_only(&format!("sqlite:///{source}"))?;
    with_database_deadline(&mut observed, deadline, |conn| {
        diagnose_runtime_observed(conn, run_id, deadline)
    })
}
fn diagnose_runtime_observed(
    conn: &Connection,
    run_id: i64,
    deadline: Instant,
) -> Result<RuntimeDiagnosis> {
    check_deadline(Instant::now(), deadline)?;
    let run = storage::get_run(conn, run_id)?;
    let mut diagnostics = Vec::new();
    match validate_run_paths(conn, &run) {
        Ok(paths) => {
            if paths.descriptor.try_exists().map_err(io_error)? {
                diagnostics.push(
                    "leftover launch descriptor; use member recover after ownership validation"
                        .into(),
                );
            }
            match RunLock::acquire(conn,&run.owner()){
                Ok(Some(_))=>diagnostics.push("matching owner lock is currently available; explicit recovery establishes loss".into()),
                Ok(None)=>diagnostics.push("matching owner lock held: live_or_unreachable".into()),
                Err(cause)=>diagnostics.push(cause.message().into_owned()),
            }
            if Instant::now() < deadline {
                match probe_health(&run, deadline) {
                    Ok(_) => diagnostics.push("matching runtime health reply observed".into()),
                    Err(cause) => diagnostics.push(format!(
                        "runtime endpoint unavailable: {cause}; inspect with member recover"
                    )),
                }
            }
        }
        Err(cause) => diagnostics.push(format!("{cause}; explicit member recover required")),
    }
    Ok(RuntimeDiagnosis { diagnostics })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationHealth {
    pub visibility: Visibility,
    pub eligibility: InputEligibility,
    pub pending_count: u64,
    pub pending_evidence: bool,
    pub gap_count: u64,
    pub degraded: bool,
    pub diagnostic: Option<String>,
}
impl ObservationHealth {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure(
            self.pending_count <= 130,
            "health pending count exceeds capacity",
        )?;
        ensure(
            serde_json::to_vec(self).map_err(io_error)?.len() <= 1024,
            "health observation exceeds encoded limit",
        )?;
        if let Some(code) = &self.diagnostic {
            ensure(
                code.len() <= 128
                    && code.is_ascii()
                    && [
                        "parser_unavailable",
                        "detector_unavailable",
                        "sequence_unavailable",
                        "sink_unavailable",
                        "observation_gap",
                    ]
                    .contains(&code.as_str()),
                "invalid health diagnostic",
            )?;
        }
        if self.eligibility == InputEligibility::ComposerEmpty {
            ensure(
                self.visibility == Visibility::NotVisible
                    && !self.degraded
                    && self.pending_count == 0
                    && !self.pending_evidence,
                "health composer proof conflicts with held state",
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::{Duration, Instant};

    use super::*;
    use crate::broker::runtime::RunStatus;
    use crate::runtime::runner::test_support::Fixture;
    use rusqlite::params;
    use serde_json::json;

    fn activate(fixture: &mut Fixture) -> (RunArtifacts, RunLock) {
        let artifacts = fixture.artifacts();
        let held = RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .unwrap();
        let process = ProcessIdentity::current().unwrap();
        let next = held
            .transfer_to_runner(&mut fixture.conn, &fixture.run.owner(), process.clone())
            .unwrap();
        fixture.conn.execute("UPDATE member_runs SET status='running',runner_pid=?1,runner_start_identity=?2,child_pid=?1,child_start_identity=?2 WHERE run_id=?3",params![process.pid,process.start_identity,next.run_id]).unwrap();
        fixture.run = storage::get_run(&fixture.conn, next.run_id).unwrap();
        (artifacts, held)
    }

    struct SilentServer {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl SilentServer {
        fn start(path: &std::path::Path) -> Self {
            let listener = UnixListener::bind(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let observed = stop.clone();
            let thread = std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(2);
                let mut connections = Vec::new();
                while !observed.load(Ordering::SeqCst) && Instant::now() < end {
                    match listener.accept() {
                        Ok((connection, _)) => connections.push(connection),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => panic!("fixture socket accept failed: {error}"),
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            Self {
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for SilentServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    fn counts(conn: &Connection) -> (i64, i64, i64) {
        let scalar = |table| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        };
        (
            scalar("member_interactions"),
            scalar("member_runtime_events"),
            scalar("messages"),
        )
    }

    #[test]
    fn runner_health_real_silent_endpoint_is_bounded_and_probe_is_read_only() {
        let mut fixture = Fixture::new();
        let (artifacts, _held) = activate(&mut fixture);
        let _server = SilentServer::start(&artifacts.paths.control_socket);
        let before = std::fs::read(fixture.conn.path().unwrap()).unwrap();
        let start = Instant::now();
        assert!(probe_health(&fixture.run, start + Duration::from_millis(80)).is_err());
        assert!(start.elapsed() < Duration::from_millis(750));
        assert_eq!(std::fs::read(fixture.conn.path().unwrap()).unwrap(), before);
    }

    #[test]
    fn runner_doctor_runtime_diagnosis_reports_leftovers_and_preserves_registry_and_artifact_bytes()
    {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        artifacts
            .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
            .unwrap();
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='failed' WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let files = [
            std::path::PathBuf::from(fixture.conn.path().unwrap()),
            artifacts.paths.namespace_manifest.clone(),
            artifacts.paths.descriptor.clone(),
            artifacts.paths.owner_lock.clone(),
        ];
        let before = files
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let start = Instant::now();
        let diagnosis = diagnose_runtime(
            &fixture.conn,
            fixture.run.run_id,
            start + Duration::from_millis(80),
        )
        .unwrap();
        assert!(start.elapsed() < Duration::from_millis(750));
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|line| line.contains("descriptor"))
        );
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|line| line.contains("recover"))
        );
        for (path, bytes) in files.iter().zip(before) {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn runner_registry_show_returns_historical_health_without_contacting_a_live_endpoint() {
        let mut fixture = Fixture::new();
        let (artifacts, _held) = activate(&mut fixture);
        let _server = SilentServer::start(&artifacts.paths.control_socket);
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        let before = counts(&fixture.conn);
        let start = Instant::now();
        let member =
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .unwrap();
        assert!(start.elapsed() < Duration::from_millis(750));
        assert_eq!(
            crate::presentation::member(&member)["runtime"]["last_health_at"],
            "2000-01-01T00:00:00Z"
        );
        assert_eq!(counts(&fixture.conn), before);
        assert_eq!(
            storage::get_run(&fixture.conn, fixture.run.run_id)
                .unwrap()
                .status,
            RunStatus::Running
        );
    }

    mod observation_contract {
        use super::*;

        fn wire(run: &RunRecord) -> serde_json::Value {
            json!({"identity":ControlIdentity::from_run(run),"runner":{"pid":run.runner_pid.unwrap(),"start_identity":run.runner_start_identity.as_ref().unwrap()},"child":{"pid":run.child_pid.unwrap(),"start_identity":run.child_start_identity.as_ref().unwrap()},"output_seq":17,"observation":{"visibility":"unknown","eligibility":{"state":"hold","reason":"unknown"},"pending_count":0,"pending_evidence":false,"gap_count":0,"degraded":false,"diagnostic":null}})
        }

        #[test]
        fn runner_batch2_health_decode_validates_legacy_identity_without_granting_composer_proof() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            let mut value = wire(&fixture.run);
            value.as_object_mut().unwrap().remove("observation");
            let decoded = HealthSnapshot::decode(value, &fixture.run).unwrap();
            assert!(decoded.observation.is_none());
        }

        #[test]
        fn runner_batch2_health_decode_accepts_bounded_hold_and_checked_empty_composer_states() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for visibility in ["visible", "obscured", "not_visible", "unknown"] {
                let mut value = wire(&fixture.run);
                value["observation"]["visibility"] = json!(visibility);
                value["observation"]["pending_count"] = json!(130);
                value["observation"]["pending_evidence"] = json!(true);
                value["observation"]["degraded"] = json!(true);
                value["observation"]["diagnostic"] = json!("observation_gap");
                let decoded = HealthSnapshot::decode(value, &fixture.run).unwrap();
                decoded.observation.unwrap().validate().unwrap();
            }
            let mut value = wire(&fixture.run);
            value["observation"]["visibility"] = json!("not_visible");
            value["observation"]["eligibility"] = json!({"state":"composer_empty"});
            assert!(HealthSnapshot::decode(value, &fixture.run).is_ok());
        }

        #[test]
        fn runner_batch2_health_decode_rejects_unknown_wire_fields_enums_unbounded_and_private_diagnostics()
         {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for field in [
                "unknown_field",
                "visibility",
                "state",
                "reason",
                "pending",
                "negative",
                "float",
                "raw_diagnostic",
                "long_diagnostic",
                "unicode_diagnostic",
            ] {
                let mut value = wire(&fixture.run);
                let status = &mut value["observation"];
                match field {
                    "unknown_field" => status["screen"] = json!("private screen"),
                    "visibility" => status["visibility"] = json!("invented"),
                    "state" => status["eligibility"]["state"] = json!("allow"),
                    "reason" => status["eligibility"]["reason"] = json!("invented"),
                    "pending" => status["pending_count"] = json!(131),
                    "negative" => status["gap_count"] = json!(-1),
                    "float" => status["pending_count"] = json!(0.5),
                    "raw_diagnostic" => {
                        status["diagnostic"] = json!("private exception configuration canary")
                    }
                    "long_diagnostic" => status["diagnostic"] = json!("x".repeat(129)),
                    _ => status["diagnostic"] = json!("世界"),
                }
                assert!(
                    HealthSnapshot::decode(value, &fixture.run).is_err(),
                    "{field}"
                );
            }
        }

        #[test]
        fn runner_batch2_health_decode_requires_hold_for_visibility_pressure_and_degradation() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for field in [
                "visible",
                "obscured",
                "unknown",
                "pending_count",
                "pending_evidence",
                "degraded",
            ] {
                let mut value = wire(&fixture.run);
                value["observation"]["visibility"] = json!("not_visible");
                value["observation"]["eligibility"] = json!({"state":"composer_empty"});
                match field {
                    "pending_count" => value["observation"][field] = json!(1),
                    "pending_evidence" | "degraded" => value["observation"][field] = json!(true),
                    _ => value["observation"]["visibility"] = json!(field),
                }
                assert!(
                    HealthSnapshot::decode(value, &fixture.run).is_err(),
                    "{field}"
                );
            }
        }

        #[test]
        fn runner_batch2_health_decode_preserves_process_generation_and_control_identity_checks() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for field in ["epoch", "nonce", "runner", "child"] {
                let mut value = wire(&fixture.run);
                match field {
                    "epoch" => {
                        value["identity"]["owner_epoch"] = json!(fixture.run.owner_epoch + 1)
                    }
                    "nonce" => value["identity"]["launch_nonce"] = json!("b".repeat(64)),
                    "runner" => value["runner"]["start_identity"] = json!("stale generation"),
                    _ => value["child"]["start_identity"] = json!("stale generation"),
                }
                assert!(
                    HealthSnapshot::decode(value, &fixture.run).is_err(),
                    "{field}"
                );
            }
        }
    }

    mod recovery_entry {
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        use std::path::PathBuf;

        use super::*;

        const SPOOL_CANARY: &[u8] = b"private-recovery-diagnosis-canary:not-a-complete-valid-frame";

        fn registry_snapshot(conn: &Connection) -> Vec<crate::db::test_support::TableSnapshot> {
            crate::db::test_support::table_snapshot(conn, |_| true)
        }

        #[derive(Debug, PartialEq)]
        struct FileSnapshot {
            path: PathBuf,
            device: u64,
            inode: u64,
            mode: u32,
            bytes: Vec<u8>,
        }

        #[derive(Debug, PartialEq)]
        struct OwnedArtifacts {
            directories: Vec<(PathBuf, u64, u64, u32)>,
            files: Vec<FileSnapshot>,
        }

        fn owned_snapshot(
            fixture: &Fixture,
            artifacts: &RunArtifacts,
            held: &RunLock,
        ) -> OwnedArtifacts {
            held.validate(&fixture.conn, &fixture.run.owner()).unwrap();
            let directories = [&artifacts.paths.namespace_dir, &artifacts.paths.run_dir]
                .into_iter()
                .map(|path| {
                    let metadata = std::fs::symlink_metadata(path).unwrap();
                    assert!(metadata.is_dir());
                    (
                        path.clone(),
                        metadata.dev(),
                        metadata.ino(),
                        metadata.mode(),
                    )
                })
                .collect();
            let files = [
                &artifacts.paths.namespace_manifest,
                &artifacts.paths.owner_lock,
                &artifacts.paths.outbox,
                &artifacts.paths.descriptor,
            ]
            .into_iter()
            .map(|path| {
                let metadata = std::fs::symlink_metadata(path).unwrap();
                assert!(metadata.is_file());
                FileSnapshot {
                    path: path.clone(),
                    device: metadata.dev(),
                    inode: metadata.ino(),
                    mode: metadata.mode(),
                    bytes: std::fs::read(path).unwrap(),
                }
            })
            .collect();
            OwnedArtifacts { directories, files }
        }

        fn seed_owned_spool(fixture: &Fixture, artifacts: &RunArtifacts, held: &RunLock) {
            held.validate(&fixture.conn, &fixture.run.owner()).unwrap();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&artifacts.paths.outbox)
                .unwrap();
            file.write_all(SPOOL_CANARY).unwrap();
            file.sync_all().unwrap();
            artifacts
                .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
                .unwrap();
        }

        fn diagnose_without_content_effects(fixture: &Fixture, expected_lock: &str) {
            let started = Instant::now();
            let diagnosis = diagnose_runtime(
                &fixture.conn,
                fixture.run.run_id,
                started + Duration::from_millis(80),
            )
            .unwrap();
            assert!(started.elapsed() < Duration::from_millis(750));
            let text = diagnosis.diagnostics.join("\n");
            assert!(text.contains(expected_lock), "{text}");
            assert!(text.contains("recover"), "{text}");
            assert!(text.contains("descriptor"), "{text}");
            assert!(!text.contains(std::str::from_utf8(SPOOL_CANARY).unwrap()));
        }

        #[test]
        fn runner_recovery_entry_doctor_preserves_held_owner_corrupt_spool_rows_and_identities() {
            let mut fixture = Fixture::new();
            let (artifacts, held) = activate(&mut fixture);
            seed_owned_spool(&fixture, &artifacts, &held);
            let before_rows = registry_snapshot(&fixture.conn);
            let before_artifacts = owned_snapshot(&fixture, &artifacts, &held);
            diagnose_without_content_effects(
                &fixture,
                "matching owner lock held: live_or_unreachable",
            );
            assert_eq!(registry_snapshot(&fixture.conn), before_rows);
            assert_eq!(
                owned_snapshot(&fixture, &artifacts, &held),
                before_artifacts
            );
        }

        #[test]
        fn runner_recovery_entry_doctor_preserves_available_owner_corrupt_spool_without_takeover() {
            let mut fixture = Fixture::new();
            let (artifacts, held) = activate(&mut fixture);
            seed_owned_spool(&fixture, &artifacts, &held);
            let before_rows = registry_snapshot(&fixture.conn);
            let before_artifacts = owned_snapshot(&fixture, &artifacts, &held);
            drop(held);
            diagnose_without_content_effects(
                &fixture,
                "matching owner lock is currently available",
            );
            let held = RunLock::acquire_released(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .expect("diagnosis releases its probe and preserves the original owner identity");
            assert_eq!(registry_snapshot(&fixture.conn), before_rows);
            assert_eq!(
                owned_snapshot(&fixture, &artifacts, &held),
                before_artifacts
            );
        }
    }
}
