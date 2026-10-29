use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CafleetError;

macro_rules! runtime_enum {
    ($name:ident, $field:literal, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $wire)] $variant),+ }
        impl $name {
            pub const VALUES: &'static [&'static str] = &[$($wire),+];
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }
        impl TryFrom<&str> for $name {
            type Error = CafleetError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(CafleetError::InvalidStoredValue {
                        field: $field.into(), value: value.into(),
                    }),
                }
            }
        }
        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                Self::try_from(value.as_str()?)
                    .map_err(|error| FromSqlError::Other(Box::new(error)))
            }
        }
    };
}

runtime_enum!(RunStatus, "member_runs.status", {
    Starting => "starting", Prepared => "prepared", Running => "running",
    Degraded => "degraded", Stopping => "stopping", Recovering => "recovering",
    Exited => "exited", Failed => "failed", Lost => "lost"
});
runtime_enum!(ModeSource, "member_runs.mode_source", { Default => "default", Explicit => "explicit" });
runtime_enum!(OwnerKind, "member_runs.owner_kind", { Creator => "creator", Runner => "runner", Reconciler => "reconciler" });
runtime_enum!(Reachability, "member_runs.reachability", { Reachable => "reachable", Suspect => "suspect" });
runtime_enum!(CleanupState, "member_runs.cleanup_state", { Complete => "complete", Incomplete => "incomplete" });
runtime_enum!(Coverage, "member_runs.coverage", { Tested => "tested", Unknown => "unknown" });
runtime_enum!(ProfileMatch, "member_runs.profile_match", { Exact => "exact", Fallback => "fallback" });
runtime_enum!(EventEvidence, "member_runs.event_evidence", { Metadata => "metadata", Redacted => "redacted" });
runtime_enum!(InteractionClass, "member_interactions.classification", {
    PermissionRequest => "permission_request", Question => "question", Trust => "trust",
    Update => "update", UnknownInteraction => "unknown_interaction"
});
runtime_enum!(InteractionCloseReason, "member_interactions.close_reason", {
    NotVisible => "not_visible", Superseded => "superseded",
    ProcessExited => "process_exited", MonitorLost => "monitor_lost"
});
runtime_enum!(RuntimeEventKind, "member_runtime_events.kind", {
    CoverageUnknown => "coverage_unknown", InteractionOpened => "interaction_opened",
    InteractionObscured => "interaction_obscured", InteractionClosed => "interaction_closed",
    PermissionDeniedObserved => "permission_denied_observed",
    AutomaticRefusalObserved => "automatic_refusal_observed",
    AutomaticReviewUnavailable => "automatic_review_unavailable", RunnerDegraded => "runner_degraded",
    RunnerLost => "runner_lost", RunnerExited => "runner_exited"
});
runtime_enum!(RuntimeCauseKind, "cause.kind", {
    CoverageUnknown => "coverage_unknown", NativeExit => "native_exit",
    StoragePressure => "storage_pressure", StorageRecovered => "storage_recovered",
    ObserverError => "observer_error", MonitorLost => "monitor_lost",
    RuntimePathMissing => "runtime_path_missing"
});
runtime_enum!(NotificationState, "member_runtime_events.notification_state", { Inbox => "inbox", Archived => "archived" });
runtime_enum!(ArchiveReason, "member_runtime_events.archive_reason", {
    MemberInactive => "member_inactive", FleetDeleted => "fleet_deleted", DirectorInactive => "director_inactive"
});
runtime_enum!(InputKind, "member_input_operations.kind", { Preview => "preview", Prompt => "prompt", Exec => "exec", PermissionResponse => "permission_response" });
runtime_enum!(InputState, "member_input_operations.state", {
    Ready => "ready", InProgress => "in_progress", Accepted => "accepted",
    Uncertain => "uncertain", Settled => "settled"
});
runtime_enum!(InputDisposition, "member_input_operations.disposition", {
    Submitted => "submitted", NotSubmitted => "not_submitted", AbandonedUncertain => "abandoned_uncertain"
});

impl RunStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Failed | Self::Lost)
    }
}

#[derive(Debug, Clone)]
pub struct NewRun {
    pub member_id: i64,
    pub fleet_id: i64,
    pub director_member_id: i64,
    pub backend: String,
    pub database_identity_json: Value,
    pub launch_nonce: String,
    pub runtime_path: String,
    pub requested_mode: Option<String>,
    pub resolved_mode: String,
    pub mode_source: ModeSource,
    pub permission_argv_json: Value,
    pub event_evidence: EventEvidence,
    pub native_version: Option<String>,
    pub profile_version: Option<String>,
    pub profile_match: Option<ProfileMatch>,
    pub coverage: Coverage,
    pub owner_pid: i64,
    pub owner_start_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunRecord {
    pub run_id: i64,
    pub member_id: i64,
    pub fleet_id: i64,
    pub director_member_id: i64,
    pub backend: String,
    pub database_identity_json: Value,
    pub launch_nonce: String,
    pub runtime_path: String,
    pub requested_mode: Option<String>,
    pub resolved_mode: String,
    pub mode_source: ModeSource,
    pub permission_argv_json: Value,
    pub event_evidence: EventEvidence,
    pub coverage: Coverage,
    pub status: RunStatus,
    pub created_at: String,
    pub native_version: Option<String>,
    pub profile_version: Option<String>,
    pub profile_match: Option<ProfileMatch>,
    pub effective_policy_json: Option<Value>,
    pub owner_epoch: i64,
    pub owner_kind: OwnerKind,
    pub owner_pid: i64,
    pub owner_start_identity: String,
    pub reachability: Reachability,
    pub cleanup_state: CleanupState,
    pub event_seq_reserved_through: i64,
    pub observation_gap_count: i64,
    pub runtime_dir_identity_json: Option<Value>,
    pub run_lock_identity_json: Option<Value>,
    pub runner_pid: Option<i64>,
    pub runner_start_identity: Option<String>,
    pub child_pid: Option<i64>,
    pub child_start_identity: Option<String>,
    pub started_at: Option<String>,
    pub last_health_at: Option<String>,
    pub ended_at: Option<String>,
    pub exit_code: Option<i64>,
    pub exit_signal: Option<i64>,
    pub cleanup_diagnostics_json: Option<Value>,
    pub recovery_origin_status: Option<RunStatus>,
}

impl RunRecord {
    pub fn owner(&self) -> RunOwner {
        RunOwner {
            run_id: self.run_id,
            database_identity_json: self.database_identity_json.clone(),
            owner_epoch: self.owner_epoch,
            owner_kind: self.owner_kind,
            owner_pid: self.owner_pid,
            owner_start_identity: self.owner_start_identity.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunOwner {
    pub run_id: i64,
    pub database_identity_json: Value,
    pub owner_epoch: i64,
    pub owner_kind: OwnerKind,
    pub owner_pid: i64,
    pub owner_start_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSnapshot {
    pub run: RunRecord,
    pub current_interaction_id: Option<i64>,
    pub unresolved_input_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InteractionRecord {
    pub interaction_id: i64,
    pub run_id: i64,
    pub episode_seq: i64,
    pub classification: InteractionClass,
    pub evidence_digest: String,
    pub opened_at: String,
    pub evidence_json: Value,
    pub closed_at: Option<String>,
    pub close_reason: Option<InteractionCloseReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedTransition {
    pub run_id: i64,
    pub event_seq: i64,
    pub episode_seq: Option<i64>,
    pub kind: RuntimeEventKind,
    pub classification: Option<InteractionClass>,
    pub observed_at: String,
    pub evidence_json: Value,
    pub close_reason: Option<InteractionCloseReason>,
    pub cause_json: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceMode {
    Live,
    Recovery,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEventRecord {
    pub event_id: i64,
    pub run_id: i64,
    pub event_seq: i64,
    pub interaction_id: Option<i64>,
    pub kind: RuntimeEventKind,
    pub observed_at: String,
    pub committed_at: String,
    pub payload_json: Value,
    pub notification_state: NotificationState,
    pub message_id: Option<i64>,
    pub archive_reason: Option<ArchiveReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEventView {
    pub event: RuntimeEventRecord,
    pub run: RuntimeSnapshot,
    pub interaction: Option<InteractionRecord>,
}

#[derive(Debug, Clone)]
pub struct NewInput {
    pub operation_key: String,
    pub kind: InputKind,
    pub payload_digest: String,
    pub message_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputOperation {
    pub input_id: i64,
    pub run_id: i64,
    pub operation_key: String,
    pub kind: InputKind,
    pub payload_digest: String,
    pub state: InputState,
    pub created_at: String,
    pub updated_at: String,
    pub message_id: Option<i64>,
    pub disposition: Option<InputDisposition>,
    pub evidence_ref: Option<String>,
    pub permission_response: Option<PermissionResponseRecord>,
}

runtime_enum!(PermissionDecision, "member_permission_responses.decision", { Allow => "allow", Deny => "deny" });
runtime_enum!(DirectorMode, "member_permission_responses.director_mode", { Auto => "auto", Manual => "manual", Unknown => "unknown" });
runtime_enum!(DecisionSource, "member_permission_responses.decision_source", { DirectorAuto => "director-auto", User => "user" });

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionResponseMetadata {
    pub event_id: i64,
    pub interaction_id: i64,
    pub expected_owner_epoch: i64,
    pub request_token: String,
    pub decision: PermissionDecision,
    pub director_member_id: i64,
    pub director_mode: DirectorMode,
    pub decision_source: DecisionSource,
    pub mode_evidence_ref: String,
    pub decision_evidence_ref: String,
    pub answer_profile_version: String,
    pub answer_rule_id: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionResponseRecord {
    pub input_id: i64,
    pub metadata: PermissionResponseMetadata,
    pub decided_at: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionResponseClaim {
    pub interaction_id: i64,
    pub input_id: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionReceipt {
    pub input: InputOperation,
    pub response: PermissionResponseRecord,
    pub claim: Option<PermissionResponseClaim>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionReceiptIdentity {
    pub database_identity_json: Value,
    pub run_id: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct NewPermissionResponse {
    pub operation_key: String,
    pub metadata: PermissionResponseMetadata,
}
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionReservation {
    Retained(PermissionReceipt),
    Reserved(PermissionReceipt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct InteractionEvidenceUpdate {
    pub run_id: i64,
    pub episode_seq: i64,
    pub output_seq: u64,
    pub evidence_json: Value,
}
