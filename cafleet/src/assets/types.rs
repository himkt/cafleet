use std::path::PathBuf;

use crate::diagnosis::AssetInstallRecord;
use crate::error::CafleetError;

#[derive(Debug)]
pub(crate) struct InstallPlan {
    pub(crate) coding_agent: String,
    pub(crate) identity: PathBuf,
    pub(crate) version: String,
    pub(crate) entries: Vec<PlannedEntry>,
}
#[derive(Debug)]
pub(crate) struct PlannedEntry {
    pub(crate) kind: EntryKind,
    pub(crate) target: PathBuf,
    pub(crate) manifest: Vec<ManifestFile>,
    pub(super) payload: Vec<&'static [u8]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Skill(&'static str),
    Preset,
    ObsoleteResearch,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManifestFile {
    pub(crate) relative_path: PathBuf,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}
pub(crate) struct InstallHooks<'a> {
    pub(crate) lock_mode: LockMode,
    pub(crate) checkpoint: &'a dyn Fn(&InstallEvent) -> Result<(), InstallFault>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// The default caller waits and uses a no-op checkpoint; injected callers consume this API.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum LockMode {
    Wait,
    Try,
}
#[derive(Debug)]
// The default caller waits and uses a no-op checkpoint; injected callers consume this API.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum InstallFault {
    Fail(String),
    Interrupt,
}
#[derive(Debug, Clone)]
// The default caller waits and uses a no-op checkpoint; injected callers consume this API.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct InstallEvent {
    pub(crate) operation: InstallOperation,
    pub(crate) edge: Edge,
    pub(crate) entry: Option<usize>,
    pub(crate) path: Option<PathBuf>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) journal: PathBuf,
    pub(crate) phase: Option<InstallPhase>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Before,
    After,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallOperation {
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
    JournalRemove,
}
#[derive(Debug)]
pub(crate) enum InstallFailure {
    Failed(CafleetError),
    Interrupted(InstallEvent),
    Busy(PathBuf),
}
impl From<CafleetError> for InstallFailure {
    fn from(error: CafleetError) -> Self {
        Self::Failed(error)
    }
}
impl InstallFailure {
    pub(super) fn into_error(self) -> CafleetError {
        match self {
            Self::Failed(error) => error,
            Self::Interrupted(event) => CafleetError::App(format!(
                "assets installation interrupted at {} ({:?}); run 'cafleet setup' to recover",
                event.journal.display(),
                event.operation
            )),
            Self::Busy(path) => CafleetError::App(format!(
                "assets installation lock busy at {}",
                path.display()
            )),
        }
    }
}
#[derive(Debug)]
pub(crate) struct InstallOutcome {
    pub(crate) installed_record: AssetInstallRecord,
    pub(crate) cleanup_pending: Option<IncompleteInstall>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) recovered_only: bool,
}
#[derive(Debug)]
pub(crate) enum RecoveryOutcome {
    #[cfg_attr(not(test), allow(dead_code))]
    None,
    RolledBack,
    Committed {
        installed_record: AssetInstallRecord,
        cleanup_pending: Option<IncompleteInstall>,
    },
}
#[derive(Debug, Clone)]
pub(crate) struct IncompleteInstall {
    pub(crate) identity: PathBuf,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) journal: PathBuf,
    pub(crate) cause: String,
}
impl IncompleteInstall {
    pub(crate) fn diagnostic(&self) -> String {
        format!(
            "incomplete assets install at {}; run 'cafleet setup' to recover",
            self.identity.display()
        )
    }
}
#[derive(Debug, Clone)]
pub(crate) struct InstallJournal {
    pub(crate) format_version: u32,
    pub(crate) transaction_id: String,
    pub(crate) phase: InstallPhase,
    pub(crate) coding_agent: String,
    pub(crate) identity: PathBuf,
    pub(crate) database_path: PathBuf,
    pub(crate) previous_record: Option<AssetInstallRecord>,
    pub(crate) new_record: AssetInstallRecord,
    pub(crate) entries: Vec<JournalEntry>,
    pub(crate) pending: Option<JournalOperation>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallPhase {
    Prepared,
    Swapping,
    Recording,
    RollingBack,
    Committed,
}
#[derive(Debug)]
pub(crate) struct InstallIntent {
    pub(crate) transaction_id: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) journal: PathBuf,
    pub(crate) state: IntentState,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntentState {
    Active,
    Finished,
}
#[derive(Debug, Clone)]
pub(crate) struct JournalEntry {
    pub(crate) target: PathBuf,
    pub(crate) stage: Option<PathBuf>,
    pub(crate) backup: PathBuf,
    pub(crate) previous: Option<EntryFingerprint>,
    pub(crate) manifest: Vec<ManifestFile>,
    pub(crate) state: EntryState,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryState {
    Original,
    BackedUp,
    Installed,
    Restored,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryFingerprint {
    pub(crate) sha256: String,
}
#[derive(Debug, Clone)]
pub(crate) struct JournalOperation {
    pub(crate) operation: InstallOperation,
    pub(crate) entry: Option<usize>,
}
