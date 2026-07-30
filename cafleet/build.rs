use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=webui-dist");
    println!("cargo:rerun-if-changed=migrations");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let dist = Path::new(&manifest_dir).join("webui-dist");
    assert!(
        dist.join("index.html").is_file(),
        "cafleet/webui-dist/ is missing or incomplete (no index.html); build the admin WebUI first with `mise //admin:build`"
    );
}
