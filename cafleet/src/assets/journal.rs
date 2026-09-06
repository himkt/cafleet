//! Versioned journal decoding validates paths before any recovery mutation.
use super::files::{self, error};
use super::types::*;
use crate::diagnosis::AssetInstallRecord;
use crate::error::CafleetError;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const JOURNAL_NAME: &str = ".cafleet-install-journal.json";
pub(super) fn journal_path(identity: &Path) -> PathBuf {
    identity.join(JOURNAL_NAME)
}
fn string(v: &Value, key: &str) -> Result<String, CafleetError> {
    v[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| error(format!("invalid install journal field {key}")))
}
fn path(v: &Value, key: &str) -> Result<PathBuf, CafleetError> {
    Ok(PathBuf::from(string(v, key)?))
}
fn record(v: &Value) -> Result<AssetInstallRecord, CafleetError> {
    Ok(AssetInstallRecord {
        coding_agent: string(v, "coding_agent")?,
        path: string(v, "path")?,
        cafleet_version: string(v, "cafleet_version")?,
        installed_at: string(v, "installed_at")?,
    })
}
/// No installed entry or database row has changed in this publication state.
pub(super) fn prepared_original(journal: &InstallJournal) -> bool {
    journal.phase == InstallPhase::Prepared
        && journal.pending.is_none()
        && journal
            .entries
            .iter()
            .all(|entry| entry.state == EntryState::Original)
}

pub(super) fn record_value(r: &AssetInstallRecord) -> Value {
    json!({"coding_agent": r.coding_agent,"path":r.path,"cafleet_version":r.cafleet_version,"installed_at":r.installed_at})
}
macro_rules! parse_enum {
    ($value:expr, $ty:ident, $($variant:ident),+ $(,)?) => {
        match $value.as_str() {
            $(Some(stringify!($variant)) => Ok($ty::$variant),)+
            _ => Err(error(concat!("invalid install journal ", stringify!($ty)))),
        }
    };
}
fn operation(v: &Value) -> Result<InstallOperation, CafleetError> {
    parse_enum!(
        v,
        InstallOperation,
        LockAcquire,
        StageWrite,
        StageValidate,
        JournalPersist,
        IntentPersist,
        BackupRename,
        InstallRename,
        RecordCommit,
        RemoveNew,
        RestoreBackup,
        RestoreRecord,
        CleanupStage,
        CleanupBackup,
        JournalRemove
    )
}
pub(super) fn value(j: &InstallJournal) -> Value {
    json!({"format_version":j.format_version,"transaction_id":j.transaction_id,"phase":format!("{:?}",j.phase),
    "coding_agent":j.coding_agent,"identity":j.identity,"database_path":j.database_path,
    "previous_record":j.previous_record.as_ref().map(record_value),"new_record":record_value(&j.new_record),
    "entries":j.entries.iter().map(|e| json!({"target":e.target,"stage":e.stage,"backup":e.backup,
      "previous":e.previous.as_ref().map(|p|json!({"sha256":p.sha256})),"state":format!("{:?}",e.state),
      "manifest":e.manifest.iter().map(|m|json!({"relative_path":m.relative_path,"size":m.size,"sha256":m.sha256})).collect::<Vec<_>>() })).collect::<Vec<_>>(),
    "pending":j.pending.as_ref().map(|p|json!({"operation":format!("{:?}",p.operation),"entry":p.entry}))})
}
pub(crate) fn read_journal(location: &Path) -> Result<InstallJournal, CafleetError> {
    if !fs::symlink_metadata(location).map_err(error)?.is_file() {
        return Err(error("install journal is not a regular file"));
    }
    let v: Value = serde_json::from_slice(&fs::read(location).map_err(error)?).map_err(error)?;
    for field in ["previous_record", "pending", "entries", "new_record"] {
        if v.get(field).is_none() {
            return Err(error(format!("missing install journal field {field}")));
        }
    }
    if v["format_version"].as_u64() != Some(1) {
        return Err(error("unsupported install journal format"));
    }
    let entries = v["entries"]
        .as_array()
        .ok_or_else(|| error("invalid journal entries"))?
        .iter()
        .map(|e| {
            for field in ["stage", "previous", "state", "manifest"] {
                if e.get(field).is_none() {
                    return Err(error(format!("missing journal entry field {field}")));
                }
            }
            let manifest = e["manifest"]
                .as_array()
                .ok_or_else(|| error("invalid manifest"))?
                .iter()
                .map(|m| {
                    Ok(ManifestFile {
                        relative_path: path(m, "relative_path")?,
                        size: m["size"]
                            .as_u64()
                            .ok_or_else(|| error("invalid manifest size"))?,
                        sha256: string(m, "sha256")?,
                    })
                })
                .collect::<Result<Vec<_>, CafleetError>>()?;
            Ok(JournalEntry {
                target: path(e, "target")?,
                stage: if e["stage"].is_null() {
                    None
                } else {
                    Some(path(e, "stage")?)
                },
                backup: path(e, "backup")?,
                previous: if e["previous"].is_null() {
                    None
                } else {
                    Some(EntryFingerprint {
                        sha256: string(&e["previous"], "sha256")?,
                    })
                },
                manifest,
                state: parse_enum!(
                    &e["state"],
                    EntryState,
                    Original,
                    BackedUp,
                    Installed,
                    Restored
                )?,
            })
        })
        .collect::<Result<Vec<_>, CafleetError>>()?;
    let pending = if v["pending"].is_null() {
        None
    } else {
        Some(JournalOperation {
            operation: operation(&v["pending"]["operation"])?,
            entry: if v["pending"]["entry"].is_null() {
                None
            } else {
                Some(
                    v["pending"]["entry"]
                        .as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or_else(|| error("invalid pending entry"))?,
                )
            },
        })
    };
    let journal = InstallJournal {
        format_version: 1,
        transaction_id: string(&v, "transaction_id")?,
        phase: parse_enum!(
            &v["phase"],
            InstallPhase,
            Prepared,
            Swapping,
            Recording,
            RollingBack,
            Committed
        )?,
        coding_agent: string(&v, "coding_agent")?,
        identity: path(&v, "identity")?,
        database_path: path(&v, "database_path")?,
        previous_record: if v["previous_record"].is_null() {
            None
        } else {
            Some(record(&v["previous_record"])?)
        },
        new_record: record(&v["new_record"])?,
        entries,
        pending,
    };
    validate(&journal, location)?;
    Ok(journal)
}
fn validate(j: &InstallJournal, location: &Path) -> Result<(), CafleetError> {
    let bad = || error("inconsistent install journal paths or transaction");
    if !super::TARGET_AGENTS.contains(&j.coding_agent.as_str())
        || !j.identity.is_absolute()
        || !j.database_path.is_absolute()
        || j.transaction_id.is_empty()
        || !j
            .transaction_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || files::normalize(&journal_path(&j.identity), false)?
            != files::normalize(location, false)?
    {
        return Err(bad());
    }
    for r in std::iter::once(&j.new_record).chain(j.previous_record.iter()) {
        if r.coding_agent != j.coding_agent || Path::new(&r.path) != j.identity {
            return Err(bad());
        }
    }
    let expected_count = if j.coding_agent == "claude" { 3 } else { 4 };
    if j.entries.len() != expected_count {
        return Err(bad());
    }
    let mut targets = BTreeSet::new();
    for (i, e) in j.entries.iter().enumerate() {
        if !e.target.is_absolute()
            || files::normalize(&e.target, false)? != e.target
            || !targets.insert(e.target.clone())
            || e.backup != files::scratch(&e.target, &j.transaction_id, "backup")
        {
            return Err(bad());
        }
        let name = if i == 0 {
            "cafleet"
        } else if i == 1 {
            "cafleet-design-doc"
        } else if i + 1 == expected_count {
            "cafleet-research"
        } else if j.coding_agent == "codex" {
            "cafleet.rules"
        } else {
            "cafleet.md"
        };
        if e.target.file_name().is_none_or(|n| n != name) {
            return Err(bad());
        }
        let research = i + 1 == expected_count;
        if research {
            if e.stage.is_some() || !e.manifest.is_empty() {
                return Err(bad());
            }
        } else if e.stage.as_ref() != Some(&files::scratch(&e.target, &j.transaction_id, "stage"))
            || e.manifest.is_empty()
        {
            return Err(bad());
        }
        let mut paths = BTreeSet::new();
        for m in &e.manifest {
            if !files::safe_relative(&m.relative_path)
                || !paths.insert(&m.relative_path)
                || m.sha256.len() != 64
                || !m.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(bad());
            }
        }
        if i < 2
            && !e
                .manifest
                .iter()
                .any(|m| m.relative_path == Path::new("SKILL.md"))
        {
            return Err(bad());
        }
        if i == 2
            && !research
            && (e.manifest.len() != 1 || !e.manifest[0].relative_path.as_os_str().is_empty())
        {
            return Err(bad());
        }
        if (i != 2 || research) && e.target.parent() != j.entries[0].target.parent() {
            return Err(bad());
        }
    }
    if j.pending
        .as_ref()
        .is_some_and(|p| p.entry.is_some_and(|i| i >= j.entries.len()))
    {
        return Err(bad());
    }
    Ok(())
}
pub(super) fn read_intent(path: &Path) -> Result<InstallIntent, CafleetError> {
    if !fs::symlink_metadata(path).map_err(error)?.is_file() {
        return Err(error("install intent is not a regular file"));
    }
    let v: Value = serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)?;
    let intent = InstallIntent {
        transaction_id: string(&v, "transaction_id")?,
        journal: path_from_value(&v)?,
        state: parse_enum!(&v["state"], IntentState, Active, Finished)?,
    };
    if !intent.journal.is_absolute() || intent.journal.file_name().is_none_or(|n| n != JOURNAL_NAME)
    {
        return Err(error("invalid intent journal path"));
    }
    Ok(intent)
}
fn path_from_value(v: &Value) -> Result<PathBuf, CafleetError> {
    path(v, "journal")
}
pub(super) fn intent_value(j: &InstallJournal, state: IntentState) -> Value {
    json!({"transaction_id":j.transaction_id,"journal":journal_path(&j.identity),"state":format!("{state:?}")})
}
