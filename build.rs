use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=skills");
    println!("cargo:rerun-if-changed=presets");
    println!("cargo:rerun-if-changed=extensions");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let skills = Path::new(&manifest_dir).join("skills");
    let presets = Path::new(&manifest_dir).join("presets");
    let extensions = Path::new(&manifest_dir).join("extensions");

    let mut generated = String::new();
    emit_table(&mut generated, "SKILLS", &skills);
    emit_table(&mut generated, "PRESETS", &presets);
    emit_table(&mut generated, "EXTENSIONS", &extensions);

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"))
        .join("embedded_data.rs");
    fs::write(&out, generated).expect("the generated embed tables are writable");
}

/// Collect every file under `dir` as a `/`-separated path relative to `root`,
/// skipping hidden entries and interpreter caches.
fn walk(root: &Path, dir: &Path, paths: &mut Vec<String>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        let name = entry.file_name();
        let name = name.to_str().expect("embedded file names are UTF-8");
        if is_excluded(name) {
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

/// Hermes rewrites a plugin's `__pycache__` on load, so an embedded cache
/// would never match its installed copy.
fn is_excluded(name: &str) -> bool {
    name.starts_with('.') || name == "__pycache__" || name.ends_with(".pyc")
}

/// Emit `pub static <name>: &[(&str, &[u8])]` mapping each relative path to
/// its `include_bytes!` contents, sorted by path.
fn emit_table(out: &mut String, name: &str, root: &Path) {
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
}
