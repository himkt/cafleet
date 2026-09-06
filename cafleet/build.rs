use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The SPA's closed extension → content-type registry; an extension present in
/// `webui-dist/` but absent here fails the build.
const CONTENT_TYPES: [(&str, &str); 9] = [
    ("html", "text/html"),
    ("js", "text/javascript"),
    ("css", "text/css"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("ico", "image/x-icon"),
    ("json", "application/json"),
    ("woff2", "font/woff2"),
    ("woff", "font/woff"),
];

fn main() {
    println!("cargo:rerun-if-changed=webui-dist");
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=../skills");
    println!("cargo:rerun-if-changed=../presets");
    println!("cargo:rerun-if-changed=../docs/docs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let dist = Path::new(&manifest_dir).join("webui-dist");
    assert!(
        dist.join("index.html").is_file(),
        "cafleet/webui-dist/ is missing or incomplete (no index.html); build the admin WebUI first with `mise //admin:build`"
    );
    let skills = Path::new(&manifest_dir).join("../skills");
    let presets = Path::new(&manifest_dir).join("../presets");

    let mut generated = String::new();
    let dist_paths = emit_table(&mut generated, "WEBUI_DIST", &dist);
    emit_table(&mut generated, "SKILLS", &skills);
    emit_table(&mut generated, "PRESETS", &presets);
    emit_content_types(&mut generated, &dist_paths);

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"))
        .join("embedded_data.rs");
    fs::write(&out, generated).expect("the generated embed tables are writable");
}

/// Collect every non-hidden file under `dir` as a `/`-separated path relative
/// to `root`.
fn walk(root: &Path, dir: &Path, paths: &mut Vec<String>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        let name = entry.file_name();
        let name = name.to_str().expect("embedded file names are UTF-8");
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, paths);
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("walked entries live under the root");
            paths.push(
                rel.to_str()
                    .expect("embedded paths are UTF-8")
                    .replace(std::path::MAIN_SEPARATOR, "/"),
            );
        }
    }
}

/// Emit `pub static <name>: &[(&str, &[u8])]` mapping each relative path to
/// its `include_bytes!` contents, sorted by path. Returns the path list.
fn emit_table(out: &mut String, name: &str, root: &Path) -> Vec<String> {
    let root = root
        .canonicalize()
        .unwrap_or_else(|e| panic!("cannot resolve {}: {e}", root.display()));
    let mut paths = Vec::new();
    walk(&root, &root, &mut paths);
    paths.sort();
    assert!(
        !paths.is_empty(),
        "the {name} tree at {} is empty",
        root.display()
    );
    writeln!(out, "pub static {name}: &[(&str, &[u8])] = &[").expect("string writes succeed");
    for rel in &paths {
        let abs = root.join(rel).display().to_string();
        writeln!(out, "    ({rel:?}, include_bytes!({abs:?})),").expect("string writes succeed");
    }
    writeln!(out, "];").expect("string writes succeed");
    paths
}

/// Emit `pub static CONTENT_TYPES: &[(&str, &str)]` for exactly the
/// extensions present in the dist, resolved through the closed registry.
fn emit_content_types(out: &mut String, dist_paths: &[String]) {
    let mut extensions: Vec<&str> = dist_paths
        .iter()
        .filter_map(|path| Path::new(path).extension())
        .map(|ext| ext.to_str().expect("embedded paths are UTF-8"))
        .collect();
    extensions.sort_unstable();
    extensions.dedup();
    writeln!(out, "pub static CONTENT_TYPES: &[(&str, &str)] = &[").expect("string writes succeed");
    for ext in extensions {
        let mime = CONTENT_TYPES
            .iter()
            .find(|(known, _)| *known == ext)
            .unwrap_or_else(|| panic!("no content type registered for extension '{ext}'"))
            .1;
        writeln!(out, "    ({ext:?}, {mime:?}),").expect("string writes succeed");
    }
    writeln!(out, "];").expect("string writes succeed");
}
