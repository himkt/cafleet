use super::*;
use serde_json::Value;

mod collection;
mod files;
mod review_existing;
mod review_policy;
mod support;

#[test]
#[ignore = "Director-owned native verification requires an explicit private configuration"]
fn native_capture_service_child() {
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    let path = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG")
        .expect("CAFLEET_NATIVE_CAPTURE_CONFIG is required for explicit native verification");
    let path = std::path::Path::new(&path);
    assert!(
        path.is_absolute(),
        "capture configuration path must be absolute"
    );
    let config = read_native_capture_config(path, &NativeCaptureLimits::default()).unwrap();
    let bounded_digest = |path: &std::path::Path| {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .unwrap();
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes).unwrap();
        assert!(
            bytes.len() <= 65536,
            "bounded private outcome evidence required"
        );
        sha256_hex(&bytes)
    };
    let config_sha256 = bounded_digest(path);
    let producer_sha256 = bounded_digest(&config.producer_manifest_path);
    let process = ProcessIdentity::current().unwrap();
    let result = run_native_capture_verification(path);
    let value = match &result {
        Ok(status) => serde_json::json!({"schema_version":1,"config_sha256":config_sha256,
            "run_id":config.expected_run_id,"launch_nonce":config.expected_launch_nonce,
            "producer_sha256":producer_sha256,"process":process,"result":"ok","status":status,"error":null}),
        Err(cause) => serde_json::json!({"schema_version":1,"config_sha256":config_sha256,
            "run_id":config.expected_run_id,"launch_nonce":config.expected_launch_nonce,
            "producer_sha256":producer_sha256,"process":process,"result":"error","status":null,
            "error":cause.message().chars().take(2048).collect::<String>()}),
    };
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() <= 16 * 1024);
    let target = path.with_extension("service-outcome.json");
    let temporary = path.with_extension("service-outcome.pending");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&temporary)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
    std::fs::hard_link(&temporary, &target).unwrap();
    std::fs::remove_file(&temporary).unwrap();
    match result {
        Ok(0) => {}
        Ok(status) => std::process::exit(status),
        Err(cause) => panic!("native capture service returned an error: {cause}"),
    }
}
