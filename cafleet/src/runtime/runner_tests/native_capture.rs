use super::*;
use serde_json::Value;

mod collection;
mod files;
mod support;

#[test]
#[ignore = "Director-owned native verification requires an explicit private configuration"]
fn native_capture_service_child() {
    let path = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG")
        .expect("CAFLEET_NATIVE_CAPTURE_CONFIG is required for explicit native verification");
    let path = std::path::Path::new(&path);
    assert!(
        path.is_absolute(),
        "capture configuration path must be absolute"
    );
    let status = run_native_capture_verification(path).expect("validated native capture service");
    assert_eq!(status, 0);
}
