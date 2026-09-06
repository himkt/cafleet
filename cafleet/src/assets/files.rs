//! Filesystem primitives never follow a final entry symlink when inspecting or removing it.
use super::types::{EntryFingerprint, ManifestFile};
use crate::error::CafleetError;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

pub(super) fn error(error: impl std::fmt::Display) -> CafleetError {
    CafleetError::App(error.to_string())
}
pub(super) fn exists(path: &Path) -> Result<bool, CafleetError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(error(format!("{}: {e}", path.display()))),
    }
}
pub(super) fn sync_dir(path: &Path) -> Result<(), CafleetError> {
    File::open(path).and_then(|f| f.sync_all()).map_err(error)
}
pub(super) fn remove(path: &Path) -> Result<(), CafleetError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(e)),
    };
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(error)?;
    sync_dir(path.parent().ok_or_else(|| error("entry has no parent"))?)
}
pub(super) fn rename(from: &Path, to: &Path) -> Result<(), CafleetError> {
    fs::rename(from, to).map_err(error)?;
    sync_dir(to.parent().ok_or_else(|| error("entry has no parent"))?)
}
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn fingerprint(path: &Path) -> Result<Option<EntryFingerprint>, CafleetError> {
    if !exists(path)? {
        return Ok(None);
    }
    fn walk(root: &Path, path: &Path, hash: &mut Sha256) -> Result<(), CafleetError> {
        let meta = fs::symlink_metadata(path).map_err(error)?;
        let name = path
            .strip_prefix(root)
            .map_err(error)?
            .as_os_str()
            .as_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        if meta.file_type().is_symlink() {
            hash.update(b"link");
            let link = fs::read_link(path).map_err(error)?;
            let bytes = link.as_os_str().as_bytes();
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else if meta.is_dir() {
            hash.update(b"dir");
            let mut children = fs::read_dir(path)
                .map_err(error)?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?;
            children.sort();
            for child in children {
                walk(root, &child, hash)?;
            }
        } else if meta.is_file() {
            hash.update(b"file");
            let bytes = fs::read(path).map_err(error)?;
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else {
            return Err(error(format!(
                "unsupported assets entry {}",
                path.display()
            )));
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    walk(path, path, &mut hash)?;
    Ok(Some(EntryFingerprint {
        sha256: hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    }))
}
pub(super) fn manifest_at(path: &Path) -> Result<Vec<ManifestFile>, CafleetError> {
    fn walk(root: &Path, path: &Path, files: &mut Vec<ManifestFile>) -> Result<(), CafleetError> {
        let meta = fs::symlink_metadata(path).map_err(error)?;
        if meta.is_dir() {
            for child in fs::read_dir(path).map_err(error)? {
                walk(root, &child.map_err(error)?.path(), files)?;
            }
        } else if meta.is_file() {
            let bytes = fs::read(path).map_err(error)?;
            files.push(ManifestFile {
                relative_path: path.strip_prefix(root).map_err(error)?.to_path_buf(),
                size: bytes.len() as u64,
                sha256: digest(&bytes),
            });
        } else {
            return Err(error(format!(
                "non-regular staged assets entry {}",
                path.display()
            )));
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(path, path, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}
pub(super) fn verify(path: &Path, manifest: &[ManifestFile]) -> Result<(), CafleetError> {
    if manifest_at(path)? == manifest {
        Ok(())
    } else {
        Err(error(format!(
            "assets manifest mismatch at {}",
            path.display()
        )))
    }
}
pub(super) fn normalize(path: &Path, create: bool) -> Result<PathBuf, CafleetError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("assets target has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| error("assets target has no basename"))?;
    if create {
        fs::create_dir_all(parent).map_err(error)?;
    }
    // Read-only diagnosis can resolve a not-yet-created suffix from its nearest existing ancestor.
    fn canonical_missing(path: &Path) -> Result<PathBuf, CafleetError> {
        match path.canonicalize() {
            Ok(path) => Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let parent = path.parent().ok_or_else(|| error(e))?;
                Ok(canonical_missing(parent)?
                    .join(path.file_name().ok_or_else(|| error("invalid path"))?))
            }
            Err(e) => Err(error(e)),
        }
    }
    Ok(canonical_missing(parent)?.join(name))
}
pub(super) fn safe_relative(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_)))
}
pub(super) fn scratch(target: &Path, tx: &str, kind: &str) -> PathBuf {
    target.with_file_name(format!(
        ".cafleet-install-{}-{tx}.{kind}",
        digest(target.as_os_str().as_bytes())
    ))
}
/// The caller treats any failure here as uncertain durability, retaining recovery evidence.
pub(super) fn durable_json(path: &Path, value: &serde_json::Value) -> Result<(), CafleetError> {
    let temp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name().unwrap().to_string_lossy()
    ));
    // An abandoned temp is never used as authoritative recovery state.
    remove(&temp)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(error)?;
    file.write_all(&serde_json::to_vec(value).map_err(error)?)
        .map_err(error)?;
    file.sync_all().map_err(error)?;
    rename(&temp, path)
}
