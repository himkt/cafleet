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
    pub status: SubmissionStatus,
    pub input_id: Option<i64>,
    pub stage: SubmissionStage,
    pub output_seq: Option<u64>,
    pub reason: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionInspectInput {
    pub interaction_id: i64,
    pub event_id: i64,
    pub director_member_id: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionRespondInput {
    pub interaction_id: i64,
    pub event_id: i64,
    pub director_member_id: i64,
    pub expected_run_id: i64,
    pub expected_owner_epoch: i64,
    pub request_token: String,
    pub decision: PermissionDecision,
    pub decision_source: DecisionSource,
    pub director_mode: DirectorMode,
    pub mode_evidence_ref: String,
    pub decision_evidence_ref: String,
    pub operation_key: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionResponseResult {
    pub submission: SubmissionResult,
    pub member_id: i64,
    pub run_id: i64,
    pub owner_epoch: i64,
    pub interaction_id: i64,
    pub event_id: i64,
    pub decision: PermissionDecision,
    pub decision_source: DecisionSource,
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
    pub event_id: i64,
    pub message_id: i64,
    pub member_id: i64,
    pub fleet_id: i64,
    pub director_member_id: i64,
    pub run_id: i64,
    pub owner_epoch: i64,
    pub interaction_id: i64,
    pub episode_seq: i64,
    pub classification: Option<InteractionClass>,
    pub visibility: Visibility,
    pub backend: String,
    pub native_version: Option<String>,
    pub profile_version: Option<String>,
    pub answer_profile_version: Option<String>,
    pub answer_rule_id: Option<String>,
    pub answer_capability: crate::broker::runtime::Coverage,
    pub capability_reason: Option<String>,
    pub supported_decisions: Vec<PermissionDecision>,
    pub request_token: Option<String>,
    pub output_seq: u64,
    pub evidence: Value,
    pub missing_evidence: Vec<MissingEvidence>,
}
pub(crate) struct SubmitInput {
    pub operation_key: String,
    pub input_kind: InputKind,
    pub message_id: Option<i64>,
    pub literal: String,
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
    pub protocol_version: u32,
    pub identity: ControlIdentity,
    pub expires_at_mono_ns: u64,
    pub operation: InputControlOperation,
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
