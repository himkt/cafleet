use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::broker::records::stored_enum;

stored_enum!(RunStatus, "member_runs.status", {
    Starting => "starting", Prepared => "prepared", Running => "running",
    Degraded => "degraded", Stopping => "stopping", Recovering => "recovering",
    Exited => "exited", Failed => "failed", Lost => "lost"
});
stored_enum!(ModeSource, "member_runs.mode_source", { Default => "default", Explicit => "explicit" });
stored_enum!(OwnerKind, "member_runs.owner_kind", { Creator => "creator", Runner => "runner", Reconciler => "reconciler" });
stored_enum!(Reachability, "member_runs.reachability", { Reachable => "reachable", Suspect => "suspect" });
stored_enum!(CleanupState, "member_runs.cleanup_state", { Complete => "complete", Incomplete => "incomplete" });
stored_enum!(Coverage, "member_runs.coverage", { Tested => "tested", Unknown => "unknown" });
stored_enum!(EventEvidence, "member_runs.event_evidence", { Metadata => "metadata", Redacted => "redacted" });
stored_enum!(AutomaticReview, "effective_policy.automatic_review", { Available => "available", Unavailable => "unavailable" });
stored_enum!(InteractionClass, "member_interactions.classification", {
    PermissionRequest => "permission_request", Question => "question", Trust => "trust",
    Update => "update", UnknownInteraction => "unknown_interaction"
});
stored_enum!(InteractionCloseReason, "member_interactions.close_reason", {
    NotVisible => "not_visible", Superseded => "superseded",
    ProcessExited => "process_exited", MonitorLost => "monitor_lost"
});
stored_enum!(test_values RuntimeEventKind,"member_runtime_events.kind", {
    CoverageUnknown => "coverage_unknown", InteractionOpened => "interaction_opened",
    InteractionObscured => "interaction_obscured", InteractionClosed => "interaction_closed",
    PermissionDeniedObserved => "permission_denied_observed",
    AutomaticRefusalObserved => "automatic_refusal_observed",
    AutomaticReviewUnavailable => "automatic_review_unavailable",
    InputDeliveryResult => "input_delivery_result", RunnerDegraded => "runner_degraded",
    RunnerLost => "runner_lost", RunnerExited => "runner_exited"
});
stored_enum!(RuntimeCauseKind, "cause.kind", {
    CoverageUnknown => "coverage_unknown", NativeExit => "native_exit",
    StoragePressure => "storage_pressure", StorageRecovered => "storage_recovered",
    ObserverError => "observer_error", MonitorLost => "monitor_lost",
    RuntimePathMissing => "runtime_path_missing"
});
stored_enum!(NotificationState, "member_runtime_events.notification_state", { Inbox => "inbox", Archived => "archived" });
stored_enum!(ArchiveReason, "member_runtime_events.archive_reason", {
    MemberInactive => "member_inactive", FleetDeleted => "fleet_deleted", DirectorInactive => "director_inactive"
});
stored_enum!(InputKind, "member_input_operations.kind", { Preview => "preview", Prompt => "prompt", Exec => "exec", PermissionResponse => "permission_response" });
stored_enum!(InputState, "member_input_operations.state", {
    Ready => "ready", InProgress => "in_progress", Accepted => "accepted",
    Uncertain => "uncertain", Settled => "settled"
});
stored_enum!(InputDisposition, "member_input_operations.disposition", {
    Submitted => "submitted", NotSubmitted => "not_submitted", AbandonedUncertain => "abandoned_uncertain"
});

impl RunStatus {
    pub(crate) fn terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Failed | Self::Lost)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectivePolicyObservation {
    pub(crate) automatic_review: AutomaticReview,
    pub(crate) observed_mode: Option<String>,
    pub(crate) rule_id: String,
    pub(crate) observed_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NewRun {
    pub(crate) member_id: i64,
    pub(crate) fleet_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) backend: String,
    pub(crate) database_identity_json: Value,
    pub(crate) launch_nonce: String,
    pub(crate) runtime_path: String,
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv_json: Value,
    pub(crate) event_evidence: EventEvidence,
    pub(crate) native_version: Option<String>,
    pub(crate) profile_version: Option<String>,
    pub(crate) coverage: Coverage,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunRecord {
    pub(crate) run_id: i64,
    pub(crate) member_id: i64,
    pub(crate) fleet_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) backend: String,
    pub(crate) database_identity_json: Value,
    pub(crate) launch_nonce: String,
    pub(crate) runtime_path: String,
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv_json: Value,
    pub(crate) event_evidence: EventEvidence,
    pub(crate) coverage: Coverage,
    pub(crate) status: RunStatus,
    pub(crate) created_at: String,
    pub(crate) native_version: Option<String>,
    pub(crate) profile_version: Option<String>,
    pub(crate) effective_policy_json: Option<Value>,
    pub(crate) owner_epoch: i64,
    pub(crate) owner_kind: OwnerKind,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
    pub(crate) reachability: Reachability,
    pub(crate) cleanup_state: CleanupState,
    pub(crate) event_seq_reserved_through: i64,
    pub(crate) observation_gap_count: i64,
    pub(crate) runtime_dir_identity_json: Option<Value>,
    pub(crate) run_lock_identity_json: Option<Value>,
    pub(crate) runner_pid: Option<i64>,
    pub(crate) runner_start_identity: Option<String>,
    pub(crate) child_pid: Option<i64>,
    pub(crate) child_start_identity: Option<String>,
    pub(crate) started_at: Option<String>,
    pub(crate) last_health_at: Option<String>,
    pub(crate) ended_at: Option<String>,
    pub(crate) exit_code: Option<i64>,
    pub(crate) exit_signal: Option<i64>,
    pub(crate) cleanup_diagnostics_json: Option<Value>,
    pub(crate) recovery_origin_status: Option<RunStatus>,
}

impl RunRecord {
    pub(crate) fn owner(&self) -> RunOwner {
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
pub(crate) struct RunOwner {
    pub(crate) run_id: i64,
    pub(crate) database_identity_json: Value,
    pub(crate) owner_epoch: i64,
    pub(crate) owner_kind: OwnerKind,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) run: RunRecord,
    pub(crate) current_interaction_id: Option<i64>,
    pub(crate) unresolved_input_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractionRecord {
    pub(crate) interaction_id: i64,
    pub(crate) run_id: i64,
    pub(crate) episode_seq: i64,
    pub(crate) classification: InteractionClass,
    pub(crate) evidence_digest: String,
    pub(crate) opened_at: String,
    pub(crate) evidence_json: Value,
    pub(crate) closed_at: Option<String>,
    pub(crate) close_reason: Option<InteractionCloseReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObservedTransition {
    pub(crate) run_id: i64,
    pub(crate) event_seq: i64,
    pub(crate) episode_seq: Option<i64>,
    pub(crate) kind: RuntimeEventKind,
    pub(crate) classification: Option<InteractionClass>,
    pub(crate) observed_at: String,
    pub(crate) evidence_json: Value,
    pub(crate) close_reason: Option<InteractionCloseReason>,
    pub(crate) cause_json: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceMode {
    Live,
    Recovery,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeEventRecord {
    pub(crate) event_id: i64,
    pub(crate) run_id: i64,
    pub(crate) event_seq: i64,
    pub(crate) interaction_id: Option<i64>,
    pub(crate) kind: RuntimeEventKind,
    pub(crate) observed_at: String,
    pub(crate) committed_at: String,
    pub(crate) payload_json: Value,
    pub(crate) notification_state: NotificationState,
    pub(crate) message_id: Option<i64>,
    pub(crate) archive_reason: Option<ArchiveReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeEventView {
    pub(crate) event: RuntimeEventRecord,
    pub(crate) run: RuntimeSnapshot,
    pub(crate) interaction: Option<InteractionRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewInput {
    pub(crate) operation_key: String,
    pub(crate) kind: InputKind,
    pub(crate) payload_digest: String,
    pub(crate) message_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InputOperation {
    pub(crate) input_id: i64,
    pub(crate) run_id: i64,
    pub(crate) operation_key: String,
    pub(crate) kind: InputKind,
    pub(crate) payload_digest: String,
    pub(crate) state: InputState,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) message_id: Option<i64>,
    pub(crate) disposition: Option<InputDisposition>,
    pub(crate) evidence_ref: Option<String>,
    pub(crate) permission_response: Option<PermissionResponseRecord>,
    pub(crate) delivery_method: Option<DeliveryMethod>,
    pub(crate) delivery_reason: Option<String>,
    pub(crate) deferred_at: Option<String>,
    pub(crate) defer_reason: Option<String>,
    pub(crate) payload_digest_version: i64,
}

stored_enum!(DeliveryStatus, "delivery.status", {
    Submitted => "submitted", Queued => "queued", Held => "held", Uncertain => "uncertain",
    NotSubmitted => "not_submitted", NotApplicable => "not_applicable"
});
stored_enum!(DeliveryMethod, "member_input_operations.delivery_method", {
    Profile => "profile", Fallback => "fallback", Unwrapped => "unwrapped"
});

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveryProjection {
    pub(crate) status: DeliveryStatus,
    pub(crate) method: Option<DeliveryMethod>,
    pub(crate) reason: Option<String>,
}
impl DeliveryProjection {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"status":self.status.as_str(),
            "method":self.method.map(DeliveryMethod::as_str),"reason":self.reason})
    }
}

stored_enum!(values PermissionDecision, "member_permission_responses.decision", { Allow => "allow", Deny => "deny" });
stored_enum!(values DirectorMode, "member_permission_responses.director_mode", { Auto => "auto", Manual => "manual", Unknown => "unknown" });
stored_enum!(values DecisionSource,"member_permission_responses.decision_source", { DirectorAuto => "director-auto", User => "user" });

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionResponseMetadata {
    pub(crate) event_id: i64,
    pub(crate) interaction_id: i64,
    pub(crate) expected_owner_epoch: i64,
    pub(crate) request_token: String,
    pub(crate) decision: PermissionDecision,
    pub(crate) director_member_id: i64,
    pub(crate) director_mode: DirectorMode,
    pub(crate) decision_source: DecisionSource,
    pub(crate) mode_evidence_ref: String,
    pub(crate) decision_evidence_ref: String,
    pub(crate) answer_profile_version: String,
    pub(crate) answer_rule_id: String,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionResponseRecord {
    pub(crate) input_id: i64,
    pub(crate) metadata: PermissionResponseMetadata,
    pub(crate) decided_at: String,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionResponseClaim {
    pub(crate) interaction_id: i64,
    pub(crate) input_id: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionReceipt {
    pub(crate) input: InputOperation,
    pub(crate) response: PermissionResponseRecord,
    pub(crate) claim: Option<PermissionResponseClaim>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionReceiptIdentity {
    pub(crate) database_identity_json: Value,
    pub(crate) run_id: i64,
}
impl PermissionReceiptIdentity {
    pub(crate) fn for_run(run: &RunRecord) -> Self {
        Self {
            database_identity_json: run.database_identity_json.clone(),
            run_id: run.run_id,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewPermissionResponse {
    pub(crate) operation_key: String,
    pub(crate) metadata: PermissionResponseMetadata,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PermissionReservation {
    Retained(PermissionReceipt),
    Reserved(PermissionReceipt),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractionEvidenceUpdate {
    pub(crate) run_id: i64,
    pub(crate) episode_seq: i64,
    pub(crate) output_seq: u64,
    pub(crate) evidence_json: Value,
}
