use super::support::*;
use super::*;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::time::{Duration, Instant};

fn arm(identity: &ControlIdentity) -> NativeCaptureArm {
    NativeCaptureArm {
        schema_version: 1,
        capture_id: "review-pair".into(),
        run_id: identity.run_id,
        owner_epoch: identity.owner_epoch,
        minimum_interval_ms: 100,
        timeout_ms: 1000,
    }
}
fn config(fixture: &CaptureFixture) -> NativeCaptureConfig {
    let policy = fixture.fixture._dir.path().join("capture-policy.json");
    private_write(
        &policy,
        &serde_json::to_vec(&json!({"schema_version":1,"masks":[]})).unwrap(),
    );
    NativeCaptureConfig {
        schema_version: 1,
        database_url: format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
        descriptor_path: fixture.artifacts.paths.descriptor.clone(),
        expected_run_id: fixture.fixture.run.run_id,
        expected_launch_nonce: fixture.fixture.run.launch_nonce.clone(),
        arm_path: fixture.fixture._dir.path().join("arm.json"),
        output_directory: fixture.fixture._dir.path().to_path_buf(),
        producer_manifest_path: fixture.producer_path.clone(),
        capture_policy_path: policy,
    }
}

#[test]
fn runner_batch2_capture_arm_validates_schema_identity_duration_and_safe_capture_id() {
    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let identity = ControlIdentity::from_run(&fixture.fixture.run);
    let now = Instant::now();
    let limits = NativeCaptureLimits::default();
    let valid = arm(&identity);
    let request = valid.to_request(&identity, now, &limits).unwrap();
    assert_eq!(request.deadline, now + Duration::from_millis(1000));
    assert_eq!(request.minimum_interval, Duration::from_millis(100));
    for field in [
        "schema",
        "run",
        "epoch",
        "blank",
        "slash",
        "control",
        "length",
        "interval_low",
        "interval_high",
        "equal_timeout",
        "long_timeout",
        "overflow",
    ] {
        let mut invalid = arm(&identity);
        match field {
            "schema" => invalid.schema_version = 2,
            "run" => invalid.run_id += 1,
            "epoch" => invalid.owner_epoch += 1,
            "blank" => invalid.capture_id = "".into(),
            "slash" => invalid.capture_id = "a/b".into(),
            "control" => invalid.capture_id = "a\n".into(),
            "length" => invalid.capture_id = "a".repeat(129),
            "interval_low" => invalid.minimum_interval_ms = 99,
            "interval_high" => invalid.minimum_interval_ms = 5001,
            "equal_timeout" => invalid.timeout_ms = 100,
            "long_timeout" => invalid.timeout_ms = 30001,
            _ => invalid.timeout_ms = u64::MAX,
        }
        assert!(
            invalid.to_request(&identity, now, &limits).is_err(),
            "{field}"
        );
    }
    let mut value = serde_json::to_value(valid).unwrap();
    value["native_input"] = json!("forbidden private key");
    assert!(serde_json::from_value::<NativeCaptureArm>(value).is_err());
}

#[test]
fn runner_batch2_capture_arm_reader_consumes_once_and_detects_replacement_after_claim() {
    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let identity = ControlIdentity::from_run(&fixture.fixture.run);
    let path = fixture.fixture._dir.path().join("one-shot-arm.json");
    let mut reader =
        NativeCaptureArmReader::new(&path, identity.clone(), NativeCaptureLimits::default())
            .unwrap();
    assert!(reader.poll(Instant::now()).unwrap().is_none());
    private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
    let request = reader.poll(Instant::now()).unwrap().unwrap();
    assert_eq!(request.capture_id, "review-pair");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
    assert!(reader.poll(Instant::now()).is_err());
}

#[test]
fn runner_batch2_capture_arm_reader_claims_malformed_or_oversized_input_without_accepting_repair() {
    for bytes in [b"{\"schema_version\":".to_vec(), vec![b'x'; 16385]] {
        let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let identity = ControlIdentity::from_run(&fixture.fixture.run);
        let path = fixture.fixture._dir.path().join("bad-arm.json");
        let mut reader =
            NativeCaptureArmReader::new(&path, identity.clone(), NativeCaptureLimits::default())
                .unwrap();
        private_write(&path, &bytes);
        assert!(reader.poll(Instant::now()).is_err());
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
        assert!(reader.poll(Instant::now()).is_err());
    }
}

#[test]
fn runner_batch2_capture_arm_reader_rejects_symlinks_public_modes_and_replaced_parent() {
    for field in ["symlink", "mode", "parent"] {
        let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let parent = fixture.fixture._dir.path().join("arm-parent");
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let identity = ControlIdentity::from_run(&fixture.fixture.run);
        let path = parent.join("arm.json");
        let mut reader =
            NativeCaptureArmReader::new(&path, identity.clone(), NativeCaptureLimits::default())
                .unwrap();
        match field {
            "symlink" => {
                let source = fixture.fixture._dir.path().join("source-arm.json");
                private_write(&source, &serde_json::to_vec(&arm(&identity)).unwrap());
                symlink(source, &path).unwrap();
            }
            "mode" => {
                private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            _ => {
                std::fs::rename(&parent, fixture.fixture._dir.path().join("old-parent")).unwrap();
                std::fs::create_dir(&parent).unwrap();
                std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
                private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
            }
        }
        assert!(reader.poll(Instant::now()).is_err(), "{field}");
    }
}

#[test]
fn runner_batch2_capture_config_requires_private_bounded_exact_files_and_existing_evidence() {
    for field in [
        "valid",
        "unknown",
        "run",
        "nonce",
        "relative",
        "missing_evidence",
        "public",
        "symlink",
        "oversized",
    ] {
        let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let config = config(&fixture);
        let path = fixture.fixture._dir.path().join("config.json");
        let mut value = serde_json::to_value(config).unwrap();
        match field {
            "unknown" => value["raw_screen"] = json!("private"),
            "run" => value["expected_run_id"] = json!(0),
            "nonce" => value["expected_launch_nonce"] = json!("A".repeat(64)),
            "relative" => value["descriptor_path"] = json!("relative.json"),
            "missing_evidence" => {
                value["producer_manifest_path"] =
                    json!(fixture.fixture._dir.path().join("absent.json"))
            }
            _ => {}
        }
        let bytes = if field == "oversized" {
            vec![b' '; 65537]
        } else {
            serde_json::to_vec(&value).unwrap()
        };
        private_write(&path, &bytes);
        if field == "public" {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let selected = if field == "symlink" {
            let link = fixture.fixture._dir.path().join("config-link.json");
            symlink(&path, &link).unwrap();
            link
        } else {
            path
        };
        let result = read_native_capture_config(&selected, &NativeCaptureLimits::default());
        assert_eq!(result.is_ok(), field == "valid", "{field}");
    }
}

#[test]
fn runner_batch2_capture_producer_binds_actual_libtest_and_rejects_fabricated_build_evidence() {
    for field in [
        "valid",
        "unknown",
        "libtest_hash",
        "libtest_path",
        "native_hash",
        "parser",
        "collector",
        "reference",
        "oversized",
    ] {
        let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let mut value = fixture.producer_json.clone();
        match field {
            "unknown" => value["private_argv"] = json!([]),
            "libtest_hash" => value["libtest_executable_sha256"] = json!("0".repeat(64)),
            "libtest_path" => value["libtest_executable_path"] = json!("relative"),
            "native_hash" => value["native_executable"]["sha256"] = json!("INVALID"),
            "parser" => value["parser_version"] = json!("0.0.0"),
            "collector" => value["collector_version"] = json!("unknown"),
            "reference" => value["environment_evidence_ref"] = json!("\n"),
            _ => {}
        }
        let path = fixture.fixture._dir.path().join("producer-case.json");
        private_write(
            &path,
            &if field == "oversized" {
                vec![b' '; 65537]
            } else {
                serde_json::to_vec(&value).unwrap()
            },
        );
        assert_eq!(
            read_native_capture_producer(&path, &NativeCaptureLimits::default()).is_ok(),
            field == "valid",
            "{field}"
        );
    }
}

#[test]
fn runner_batch2_capture_bind_checks_actual_native_digest_before_any_observation() {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
    let mut producer = fixture.producer_json.clone();
    producer["native_executable"]["sha256"] = json!("0".repeat(64));
    let path = fixture.fixture._dir.path().join("wrong-native.json");
    private_write(&path, &serde_json::to_vec(&producer).unwrap());
    let producer = read_native_capture_producer(&path, &NativeCaptureLimits::default()).unwrap();
    fixture
        .fixture
        .conn
        .execute(
            "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
            [fixture.fixture.run.run_id],
        )
        .unwrap();
    fixture.fixture.run =
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert!(
        port.bind_prepared(
            &fixture.fixture.run,
            &fixture.placement,
            &fixture.spec,
            producer,
            &fixture.clock
        )
        .is_err()
    );
    assert!(receiver.finalized_identity().is_none());
    assert!(receiver.request(fixture.request("unbound")).is_err());
}

#[test]
fn runner_batch2_capture_publication_accepts_only_the_final_manifest_and_matching_complete_artifact()
 {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    let lease = fixture.pair(&mut port, &receiver, b"safe fixture");
    let output = fixture.fixture._dir.path().join("publication");
    std::fs::create_dir(&output).unwrap();
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
    let publication = publish_native_capture(
        &output,
        &lease,
        &NativeCaptureLimits::default(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        publication.publication_status,
        CapturePublicationStatus::Complete
    );
    assert_eq!(publication.collection_status, NativeCaptureStatus::Complete);
    assert!(publication.artifact_path.ends_with("pair.complete.json"));
    assert!(publication.manifest_path.ends_with("pair.manifest.json"));
    let artifact = std::fs::read(&publication.artifact_path).unwrap();
    assert_eq!(
        publication.artifact_sha256,
        digest(&publication.artifact_path)
    );
    assert_eq!(publication.artifact_bytes, artifact.len() as u64);
    assert_eq!(
        serde_json::from_slice::<Value>(&artifact).unwrap(),
        value(&lease)
    );
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(&publication.manifest_path).unwrap()).unwrap();
    assert_keys(
        &manifest,
        &[
            "schema_version",
            "capture_id",
            "artifact_ref",
            "artifact_sha256",
            "artifact_bytes",
            "collection_status",
            "publication_status",
            "producer_ref",
            "diagnostic",
        ],
    );
    assert_eq!(manifest["artifact_sha256"], publication.artifact_sha256);
    assert_eq!(manifest["artifact_bytes"], publication.artifact_bytes);
    assert_eq!(manifest["collection_status"], "complete");
    assert_eq!(manifest["publication_status"], "complete");
    for path in [&publication.artifact_path, &publication.manifest_path] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(receiver.request(fixture.request("while-exported")).is_err());
}

#[test]
fn runner_batch2_capture_publication_collision_and_expiry_preserve_existing_output_without_success()
{
    for collision in ["artifact", "manifest", "expired"] {
        let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
        let lease = fixture.pair(&mut port, &receiver, b"private");
        let directory = fixture.fixture._dir.path();
        let existing = directory.join(if collision == "artifact" {
            "pair.complete.json"
        } else {
            "pair.manifest.json"
        });
        if collision != "expired" {
            private_write(&existing, b"owned prior output");
        }
        let until = if collision == "expired" {
            Instant::now() - Duration::from_millis(1)
        } else {
            Instant::now() + Duration::from_secs(2)
        };
        assert!(
            publish_native_capture(directory, &lease, &NativeCaptureLimits::default(), until)
                .is_err()
        );
        if collision != "expired" {
            assert_eq!(std::fs::read(&existing).unwrap(), b"owned prior output");
        }
        if collision != "manifest" {
            assert!(!directory.join("pair.manifest.json").exists());
        }
    }
}

#[test]
fn runner_batch2_capture_incomplete_publication_keeps_terminal_reason_and_incomplete_filename() {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    receiver.request(fixture.request("cancelled-pair")).unwrap();
    port.cancel(NativeCaptureStatus::OwnerExit).unwrap();
    let lease = receiver.try_take().unwrap().unwrap();
    let publication = publish_native_capture(
        fixture.fixture._dir.path(),
        &lease,
        &NativeCaptureLimits::default(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        publication.collection_status,
        NativeCaptureStatus::OwnerExit
    );
    assert_eq!(
        publication.publication_status,
        CapturePublicationStatus::Incomplete
    );
    assert!(
        publication
            .artifact_path
            .ends_with("cancelled-pair.incomplete.json")
    );
    assert!(
        !fixture
            .fixture
            ._dir
            .path()
            .join("cancelled-pair.complete.json")
            .exists()
    );
}

#[test]
fn runner_batch2_capture_ignored_entry_requires_explicit_configuration_before_launching() {
    let directory = tempfile::Builder::new()
        .prefix(".p194-entry-")
        .tempdir_in("/tmp")
        .unwrap();
    let output_path = directory.path().join("entry-output");
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output_path)
        .unwrap();
    let mut child = crate::runtime::runner_tests::owned_child::OwnedChild::new(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::runner_tests::native_capture::native_capture_service_child",
                "--ignored",
                "--test-threads=1",
            ])
            .env_remove("CAFLEET_NATIVE_CAPTURE_CONFIG")
            .stdout(std::process::Stdio::from(output.try_clone().unwrap()))
            .stderr(std::process::Stdio::from(output))
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(3);
    let status = child.poll_until(until - Duration::from_secs(1)).unwrap();
    if status.is_none() {
        child.terminate_until(until);
    }
    assert!(
        !status
            .expect("configuration-free ignored entry completes within its bound")
            .success()
    );
    assert!(std::fs::metadata(&output_path).unwrap().len() <= 65536);
    let bytes = std::fs::read(&output_path).unwrap();
    let combined = String::from_utf8_lossy(&bytes);
    assert!(
        combined.contains("CAFLEET_NATIVE_CAPTURE_CONFIG"),
        "{combined}"
    );
}
