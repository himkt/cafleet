use super::*;
mod wire;
pub(super) use wire::*;

use crate::broker::runtime::{
    DecisionSource, DirectorMode, InputDisposition, InputKind, InputState, InteractionClass,
    PermissionDecision,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubmissionStatus {
    Accepted,
    Held,
    Rejected,
    Uncertain,
}
impl SubmissionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Held => "held",
            Self::Rejected => "rejected",
            Self::Uncertain => "uncertain",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubmissionStage {
    ZeroBytes,
    Partial,
    Submitted,
    Unknown,
}
impl SubmissionStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ZeroBytes => "zero_bytes",
            Self::Partial => "partial",
            Self::Submitted => "submitted",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmissionResult {
    pub(crate) status: SubmissionStatus,
    pub(crate) input_id: Option<i64>,
    pub(crate) stage: SubmissionStage,
    pub(crate) output_seq: Option<u64>,
    pub(crate) reason: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionInspectInput {
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    pub(crate) director_member_id: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionRespondInput {
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) expected_run_id: i64,
    pub(crate) expected_owner_epoch: i64,
    pub(crate) request_token: String,
    pub(crate) decision: PermissionDecision,
    pub(crate) decision_source: DecisionSource,
    pub(crate) director_mode: DirectorMode,
    pub(crate) mode_evidence_ref: String,
    pub(crate) decision_evidence_ref: String,
    pub(crate) operation_key: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionResponseResult {
    pub(crate) submission: SubmissionResult,
    pub(crate) member_id: i64,
    pub(crate) run_id: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    pub(crate) decision: PermissionDecision,
    pub(crate) decision_source: DecisionSource,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MissingEvidence {
    Operation,
    Arguments,
    Cwd,
    Environment,
    Targets,
    Effects,
    ChoiceScope,
}
impl MissingEvidence {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Operation => "operation",
            Self::Arguments => "arguments",
            Self::Cwd => "cwd",
            Self::Environment => "environment",
            Self::Targets => "targets",
            Self::Effects => "effects",
            Self::ChoiceScope => "choice_scope",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionInspection {
    pub(crate) event_id: i64,
    pub(crate) message_id: i64,
    pub(crate) member_id: i64,
    pub(crate) fleet_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) run_id: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) interaction_id: i64,
    pub(crate) episode_seq: i64,
    pub(crate) classification: Option<InteractionClass>,
    pub(crate) visibility: Visibility,
    pub(crate) backend: String,
    pub(crate) native_version: Option<String>,
    pub(crate) profile_version: Option<String>,
    pub(crate) answer_profile_version: Option<String>,
    pub(crate) answer_rule_id: Option<String>,
    pub(crate) answer_capability: crate::broker::runtime::Coverage,
    pub(crate) capability_reason: Option<String>,
    pub(crate) supported_decisions: Vec<PermissionDecision>,
    pub(crate) request_token: Option<String>,
    pub(crate) output_seq: u64,
    pub(crate) evidence: Value,
    pub(crate) missing_evidence: Vec<MissingEvidence>,
}
pub(crate) struct SubmitInput {
    pub(crate) operation_key: String,
    pub(crate) input_kind: InputKind,
    pub(crate) message_id: Option<i64>,
    pub(crate) literal: String,
}
pub(crate) enum ReceiptAction {
    Query {
        operation_key: String,
    },
    Retire {
        input_id: i64,
    },
    Reconcile {
        input_id: i64,
        expected: InputState,
        disposition: InputDisposition,
        evidence_ref: String,
    },
}
pub(crate) struct InputControlRequest {
    pub(crate) protocol_version: u32,
    pub(crate) identity: ControlIdentity,
    pub(crate) expires_at_mono_ns: u64,
    pub(crate) operation: InputControlOperation,
}
pub(crate) enum InputControlOperation {
    SubmitPrompt(SubmitInput),
    SubmitExec(SubmitInput),
    PermissionInspect(PermissionInspectInput),
    PermissionRespond(PermissionRespondInput),
    Receipt(ReceiptAction),
}
pub(crate) enum ReceiptReply {
    Missing,
    Present {
        input_id: i64,
        submission: SubmissionResult,
    },
    Retired {
        input_id: i64,
    },
    Settled {
        input_id: i64,
    },
}
pub(crate) enum InputControlReply {
    Submission(SubmissionResult),
    Inspection(Box<PermissionInspection>),
    PermissionResponse(PermissionResponseResult),
    Receipt(ReceiptReply),
}
