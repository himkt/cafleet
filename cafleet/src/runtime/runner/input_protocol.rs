use super::*;
use crate::runtime::runner::codec::{
    decode_legacy_control_body, frame_control_body, validate_version,
};
use serde::Deserializer;
use serde::de::{DeserializeOwned, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, json};
use std::fmt;

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
impl SubmissionResult {
    pub(crate) fn held(reason: &str) -> Self {
        Self {
            status: SubmissionStatus::Held,
            input_id: None,
            stage: SubmissionStage::ZeroBytes,
            output_seq: None,
            reason: Some(reason.into()),
        }
    }
}
#[derive(Clone, Serialize)]
pub(crate) struct PermissionInspectInput {
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    pub(crate) director_member_id: i64,
}
#[derive(Clone, Serialize)]
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
impl PermissionRespondInput {
    pub(crate) fn inspection(&self) -> PermissionInspectInput {
        PermissionInspectInput {
            interaction_id: self.interaction_id,
            event_id: self.event_id,
            director_member_id: self.director_member_id,
        }
    }
}
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
#[derive(Clone, Copy, Serialize, Deserialize)]
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
    /// The wire values in declaration order, which is also the required reply order.
    const VALUES: &'static [&'static str] = &[
        "operation",
        "arguments",
        "cwd",
        "environment",
        "targets",
        "effects",
        "choice_scope",
    ];
    pub(crate) fn as_str(&self) -> &'static str {
        Self::VALUES[*self as usize]
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
#[derive(Clone)]
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

struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object fields")
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                value: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(
                self,
                value: i64,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_u64<E: serde::de::Error>(
                self,
                value: u64,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_f64<E: serde::de::Error>(
                self,
                value: f64,
            ) -> std::result::Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| UniqueValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("finite JSON number required"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.into())))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut object = Map::new();
                while let Some(key) = entries.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON field"));
                    }
                    let UniqueValue(value) = entries.next_value()?;
                    object.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(object)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

fn decode_strict_json(body: &[u8]) -> Result<Value> {
    serde_json::from_slice::<UniqueValue>(body)
        .map(|value| value.0)
        .map_err(|_| error("malformed or duplicate control JSON fields"))
}

struct InputRoute(bool);
impl<'de> Deserialize<'de> for InputRoute {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct RouteVisitor;
        impl<'de> Visitor<'de> for RouteVisitor {
            type Value = InputRoute;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a control request object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut input = false;
                while let Some(key) = entries.next_key::<String>()? {
                    if key == "kind" {
                        let value: Value = entries.next_value()?;
                        input |= value.as_str().is_some_and(|kind| {
                            matches!(
                                kind,
                                "submit_prompt"
                                    | "submit_exec"
                                    | "permission_inspect"
                                    | "permission_respond"
                                    | "receipt"
                            )
                        });
                    } else {
                        entries.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(InputRoute(input))
            }
        }
        deserializer.deserialize_map(RouteVisitor)
    }
}

fn routes_input(body: &[u8]) -> Result<bool> {
    serde_json::from_slice::<InputRoute>(body)
        .map(|route| route.0)
        .map_err(|_| error("malformed control request object"))
}

struct Object<'a>(&'a Map<String, Value>);
impl<'a> Object<'a> {
    fn new(value: &'a Value, fields: &[&str]) -> Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| error("control object required"))?;
        ensure(
            object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field)),
            "control object requires its exact fields",
        )?;
        Ok(Self(object))
    }
    fn value(&self, field: &str) -> &Value {
        self.0
            .get(field)
            .expect("validated control field is present")
    }
    fn get<T: DeserializeOwned>(&self, field: &str) -> Result<T> {
        serde_json::from_value(self.value(field).clone())
            .map_err(|_| error(format!("invalid control {field}")))
    }
    fn positive(&self, field: &str) -> Result<i64> {
        let value: i64 = self.get(field)?;
        ensure(value > 0, "positive control identity required")?;
        Ok(value)
    }
}

fn decode_identity(value: &Value) -> Result<ControlIdentity> {
    let object = Object::new(
        value,
        &[
            "database_identity_json",
            "fleet_id",
            "member_id",
            "director_member_id",
            "run_id",
            "owner_epoch",
            "launch_nonce",
        ],
    )?;
    crate::broker::runtime::validate_identity(object.value("database_identity_json"))
        .map_err(|_| error("invalid control database identity"))?;
    let launch_nonce: String = object.get("launch_nonce")?;
    crate::broker::runtime::hex(&launch_nonce, 64, "launch_nonce")
        .map_err(|_| error("invalid control launch nonce"))?;
    Ok(ControlIdentity {
        database_identity_json: object.value("database_identity_json").clone(),
        fleet_id: object.positive("fleet_id")?,
        member_id: object.positive("member_id")?,
        director_member_id: object.positive("director_member_id")?,
        run_id: object.positive("run_id")?,
        owner_epoch: object.positive("owner_epoch")?,
        launch_nonce,
    })
}

fn request_fields(kind: &str, value: &Value) -> Result<Vec<&'static str>> {
    let fields: &[&str] = match kind {
        "submit_prompt" | "submit_exec" => {
            &["operation_key", "input_kind", "message_id", "literal"]
        }
        "permission_inspect" => &["interaction_id", "event_id", "director_member_id"],
        "permission_respond" => &[
            "interaction_id",
            "event_id",
            "director_member_id",
            "expected_run_id",
            "expected_owner_epoch",
            "request_token",
            "decision",
            "decision_source",
            "director_mode",
            "mode_evidence_ref",
            "decision_evidence_ref",
            "operation_key",
        ],
        "receipt" => match value.get("action").and_then(Value::as_str) {
            Some("query") => &["action", "operation_key"],
            Some("retire") => &["action", "input_id"],
            Some("reconcile") => &[
                "action",
                "input_id",
                "expected_state",
                "disposition",
                "evidence_ref",
            ],
            _ => return Err(error("invalid control receipt action")),
        },
        _ => return Err(error("unknown input control operation")),
    };
    Ok(
        ["protocol_version", "kind", "identity", "expires_at_mono_ns"]
            .into_iter()
            .chain(fields.iter().copied())
            .collect(),
    )
}

fn decode_request_value(value: &Value) -> Result<InputControlRequest> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| error("control kind required"))?;
    let object = Object::new(value, &request_fields(kind, value)?)?;
    validate_version(value)?;
    let identity = decode_identity(object.value("identity"))?;
    let expires_at_mono_ns: u64 = object.get("expires_at_mono_ns")?;
    ensure(expires_at_mono_ns > 0, "positive input expiration required")?;
    let operation = match kind {
        "submit_prompt" | "submit_exec" => {
            let input = SubmitInput {
                operation_key: object.get("operation_key")?,
                input_kind: object.get("input_kind")?,
                message_id: object.get("message_id")?,
                literal: object.get("literal")?,
            };
            validate_operation_key(&input.operation_key)?;
            ensure(
                crate::input::valid_input(&input.literal),
                "valid single-line input literal required",
            )?;
            let valid_kind = match (kind, input.input_kind, input.message_id) {
                ("submit_prompt", InputKind::Preview, Some(id)) => id > 0,
                ("submit_prompt", InputKind::Prompt, None)
                | ("submit_exec", InputKind::Exec, None) => true,
                _ => false,
            };
            ensure(
                valid_kind,
                "control input kind and message must match the operation",
            )?;
            if kind == "submit_prompt" {
                InputControlOperation::SubmitPrompt(input)
            } else {
                InputControlOperation::SubmitExec(input)
            }
        }
        "permission_inspect" => {
            let input = PermissionInspectInput {
                interaction_id: object.positive("interaction_id")?,
                event_id: object.positive("event_id")?,
                director_member_id: object.positive("director_member_id")?,
            };
            ensure(
                input.director_member_id == identity.director_member_id,
                "control Director identity mismatch",
            )?;
            InputControlOperation::PermissionInspect(input)
        }
        "permission_respond" => {
            let input = PermissionRespondInput {
                interaction_id: object.positive("interaction_id")?,
                event_id: object.positive("event_id")?,
                director_member_id: object.positive("director_member_id")?,
                expected_run_id: object.positive("expected_run_id")?,
                expected_owner_epoch: object.positive("expected_owner_epoch")?,
                request_token: object.get("request_token")?,
                decision: object.get("decision")?,
                decision_source: object.get("decision_source")?,
                director_mode: object.get("director_mode")?,
                mode_evidence_ref: object.get("mode_evidence_ref")?,
                decision_evidence_ref: object.get("decision_evidence_ref")?,
                operation_key: object.get("operation_key")?,
            };
            ensure(
                input.director_member_id == identity.director_member_id
                    && input.expected_run_id == identity.run_id
                    && input.expected_owner_epoch == identity.owner_epoch,
                "control permission identity mismatch",
            )?;
            super::input_client::validate_respond(&input)?;
            InputControlOperation::PermissionRespond(input)
        }
        "receipt" => {
            InputControlOperation::Receipt(match object.get::<String>("action")?.as_str() {
                "query" => {
                    let operation_key: String = object.get("operation_key")?;
                    validate_operation_key(&operation_key)?;
                    ReceiptAction::Query { operation_key }
                }
                "retire" => ReceiptAction::Retire {
                    input_id: object.positive("input_id")?,
                },
                "reconcile" => {
                    let evidence_ref: String = object.get("evidence_ref")?;
                    validate_permission_reference(&evidence_ref)?;
                    ReceiptAction::Reconcile {
                        input_id: object.positive("input_id")?,
                        expected: object.get("expected_state")?,
                        disposition: object.get("disposition")?,
                        evidence_ref,
                    }
                }
                _ => return Err(error("invalid control receipt action")),
            })
        }
        _ => return Err(error("unknown input control operation")),
    };
    Ok(InputControlRequest {
        identity,
        expires_at_mono_ns,
        operation,
    })
}

fn decode_input_request(body: &[u8]) -> Result<InputControlRequest> {
    decode_request_value(&decode_strict_json(body)?)
}

pub(in crate::runtime::runner) enum ServiceControlRequest {
    Legacy(Value),
    Input(Box<InputControlRequest>),
}
pub(in crate::runtime::runner) fn decode_service_request(
    body: &[u8],
) -> Result<ServiceControlRequest> {
    if routes_input(body)? {
        decode_input_request(body).map(|request| ServiceControlRequest::Input(Box::new(request)))
    } else {
        decode_legacy_control_body(body).map(ServiceControlRequest::Legacy)
    }
}

pub(in crate::runtime::runner) fn encode_input_request(
    request: &InputControlRequest,
) -> Result<Vec<u8>> {
    let (kind, fields) = match &request.operation {
        InputControlOperation::SubmitPrompt(input) | InputControlOperation::SubmitExec(input) => {
            let kind = if matches!(request.operation, InputControlOperation::SubmitPrompt(_)) {
                "submit_prompt"
            } else {
                "submit_exec"
            };
            (
                kind,
                json!({"operation_key":input.operation_key,"input_kind":input.input_kind,"message_id":input.message_id,"literal":input.literal}),
            )
        }
        InputControlOperation::PermissionInspect(input) => (
            "permission_inspect",
            serde_json::to_value(input).map_err(io_error)?,
        ),
        InputControlOperation::PermissionRespond(input) => (
            "permission_respond",
            serde_json::to_value(input).map_err(io_error)?,
        ),
        InputControlOperation::Receipt(action) => (
            "receipt",
            match action {
                ReceiptAction::Query { operation_key } => {
                    json!({"action":"query","operation_key":operation_key})
                }
                ReceiptAction::Retire { input_id } => {
                    json!({"action":"retire","input_id":input_id})
                }
                ReceiptAction::Reconcile {
                    input_id,
                    expected,
                    disposition,
                    evidence_ref,
                } => json!({"action":"reconcile","input_id":input_id,
                "expected_state":expected,"disposition":disposition,"evidence_ref":evidence_ref}),
            },
        ),
    };
    let mut value = json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"kind":kind,"identity":request.identity,"expires_at_mono_ns":request.expires_at_mono_ns});
    value
        .as_object_mut()
        .expect("control envelope is an object")
        .extend(
            fields
                .as_object()
                .expect("control payload is an object")
                .clone(),
        );
    decode_request_value(&value)?;
    frame_control_body(&serde_json::to_vec(&value).map_err(io_error)?)
}

const SUBMISSION_FIELDS: &[&str] = &["status", "input_id", "stage", "output_seq", "reason"];
const RESPONSE_FIELDS: &[&str] = &[
    "status",
    "input_id",
    "member_id",
    "run_id",
    "owner_epoch",
    "interaction_id",
    "event_id",
    "decision",
    "decision_source",
    "stage",
    "output_seq",
    "reason",
];
const INSPECTION_FIELDS: &[&str] = &[
    "event_id",
    "message_id",
    "member_id",
    "fleet_id",
    "director_member_id",
    "run_id",
    "owner_epoch",
    "interaction_id",
    "episode_seq",
    "classification",
    "visibility",
    "backend",
    "native_version",
    "profile_version",
    "answer_profile_version",
    "answer_rule_id",
    "answer_capability",
    "capability_reason",
    "supported_decisions",
    "request_token",
    "output_seq",
    "evidence",
    "missing_evidence",
];

fn submission(value: &Value) -> Result<SubmissionResult> {
    Object::new(value, SUBMISSION_FIELDS)?;
    let result: SubmissionResult = serde_json::from_value(value.clone())
        .map_err(|_| error("invalid control submission fields"))?;
    if let Some(id) = result.input_id {
        ensure(id > 0, "positive control input ID required")?;
    }
    Ok(result)
}

fn response(value: &Value) -> Result<PermissionResponseResult> {
    let object = Object::new(value, RESPONSE_FIELDS)?;
    let submission_value = Value::Object(
        SUBMISSION_FIELDS
            .iter()
            .map(|field| ((*field).into(), object.value(field).clone()))
            .collect(),
    );
    Ok(PermissionResponseResult {
        submission: submission(&submission_value)?,
        member_id: object.positive("member_id")?,
        run_id: object.positive("run_id")?,
        owner_epoch: object.positive("owner_epoch")?,
        interaction_id: object.positive("interaction_id")?,
        event_id: object.positive("event_id")?,
        decision: object.get("decision")?,
        decision_source: object.get("decision_source")?,
    })
}

fn inspection(value: &Value) -> Result<Box<PermissionInspection>> {
    let object = Object::new(value, INSPECTION_FIELDS)?;
    for field in [
        "event_id",
        "message_id",
        "member_id",
        "fleet_id",
        "director_member_id",
        "run_id",
        "owner_epoch",
        "interaction_id",
        "episode_seq",
    ] {
        object.positive(field)?;
    }
    let result: PermissionInspection = serde_json::from_value(value.clone())
        .map_err(|_| error("invalid control inspection fields"))?;
    if let Some(token) = &result.request_token {
        validate_request_token(token)?;
    }
    validate_order(
        result
            .supported_decisions
            .iter()
            .map(|decision| decision.as_str()),
        PermissionDecision::VALUES,
    )?;
    validate_order(
        result.missing_evidence.iter().map(MissingEvidence::as_str),
        MissingEvidence::VALUES,
    )?;
    crate::broker::runtime::evidence::validate(&result.evidence)
        .map_err(|_| error("invalid control inspection evidence"))?;
    Ok(Box::new(result))
}

fn validate_order<'a>(values: impl Iterator<Item = &'a str>, domain: &[&str]) -> Result<()> {
    let mut previous = None;
    for value in values {
        let position = domain
            .iter()
            .position(|item| *item == value)
            .expect("serde admits only the enum's wire values");
        ensure(
            previous.is_none_or(|prior| position > prior),
            "control inspection subset order mismatch",
        )?;
        previous = Some(position);
    }
    Ok(())
}

fn receipt(value: &Value) -> Result<ReceiptReply> {
    let object = Object::new(value, &["status", "input_id", "submission"])?;
    match object.get::<String>("status")?.as_str() {
        "missing" => {
            ensure(
                object.value("input_id").is_null() && object.value("submission").is_null(),
                "missing receipt requires null input and submission",
            )?;
            Ok(ReceiptReply::Missing)
        }
        "present" => {
            let input_id = object.positive("input_id")?;
            let submission = submission(object.value("submission"))?;
            ensure(
                submission.input_id == Some(input_id),
                "receipt input identity mismatch",
            )?;
            Ok(ReceiptReply::Present {
                input_id,
                submission,
            })
        }
        "retired" | "settled" => {
            let input_id = object.positive("input_id")?;
            ensure(
                object.value("submission").is_null(),
                "consumed receipt requires null submission",
            )?;
            if object.value("status") == "retired" {
                Ok(ReceiptReply::Retired { input_id })
            } else {
                Ok(ReceiptReply::Settled { input_id })
            }
        }
        _ => Err(error("invalid control receipt status")),
    }
}

fn result(value: &Value) -> Result<InputControlReply> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| error("control result kind required"))?;
    match kind {
        "submission" => {
            let object = Object::new(value, &["kind", "submission"])?;
            submission(object.value("submission")).map(InputControlReply::Submission)
        }
        "inspection" => {
            let object = Object::new(value, &["kind", "request"])?;
            inspection(object.value("request")).map(InputControlReply::Inspection)
        }
        "permission_response" => {
            let object = Object::new(value, &["kind", "response"])?;
            response(object.value("response")).map(InputControlReply::PermissionResponse)
        }
        "receipt" => {
            let object = Object::new(value, &["kind", "receipt"])?;
            receipt(object.value("receipt")).map(InputControlReply::Receipt)
        }
        _ => Err(error("invalid control result kind")),
    }
}

fn validate_binding(reply: &InputControlReply, request: &InputControlRequest) -> Result<()> {
    let identity = &request.identity;
    let matches = match (&request.operation, reply) {
        (
            InputControlOperation::SubmitPrompt(_) | InputControlOperation::SubmitExec(_),
            InputControlReply::Submission(_),
        ) => true,
        (
            InputControlOperation::PermissionInspect(input),
            InputControlReply::Inspection(result),
        ) => {
            result.event_id == input.event_id
                && result.interaction_id == input.interaction_id
                && result.director_member_id == input.director_member_id
                && result.member_id == identity.member_id
                && result.fleet_id == identity.fleet_id
                && result.run_id == identity.run_id
                && result.owner_epoch == identity.owner_epoch
        }
        (
            InputControlOperation::PermissionRespond(input),
            InputControlReply::PermissionResponse(result),
        ) => {
            result.event_id == input.event_id
                && result.interaction_id == input.interaction_id
                && result.member_id == identity.member_id
                && result.run_id == input.expected_run_id
                && result.owner_epoch == input.expected_owner_epoch
                && result.decision == input.decision
                && result.decision_source == input.decision_source
        }
        (InputControlOperation::Receipt(action), InputControlReply::Receipt(result)) => {
            match (action, result) {
                (
                    ReceiptAction::Query { .. },
                    ReceiptReply::Missing | ReceiptReply::Present { .. },
                ) => true,
                (
                    ReceiptAction::Retire { input_id: expected },
                    ReceiptReply::Retired { input_id },
                )
                | (
                    ReceiptAction::Reconcile {
                        input_id: expected, ..
                    },
                    ReceiptReply::Settled { input_id },
                ) => expected == input_id,
                _ => false,
            }
        }
        _ => false,
    };
    ensure(matches, "control reply kind or request identity mismatch")
}

pub(in crate::runtime::runner) fn decode_input_reply(
    body: &[u8],
    request: &InputControlRequest,
) -> Result<InputControlReply> {
    let value = decode_strict_json(body)?;
    if value.get("error").is_some() {
        let object = Object::new(&value, &["protocol_version", "error"])?;
        validate_version(&value)?;
        let cause: String = object.get("error")?;
        return Err(error(format!("runner control: {cause}")));
    }
    let object = Object::new(&value, &["protocol_version", "result"])?;
    validate_version(&value)?;
    let result = result(object.value("result"))?;
    validate_binding(&result, request)?;
    Ok(result)
}

fn encode_result(reply: &InputControlReply) -> Value {
    match reply {
        InputControlReply::Submission(submission) => {
            json!({"kind":"submission","submission":submission})
        }
        InputControlReply::Inspection(inspection) => {
            let public = crate::presentation::permission_inspection(inspection);
            json!({"kind":"inspection","request":public.get("request").expect("inspection projection contains its request")})
        }
        InputControlReply::PermissionResponse(response) => {
            let public = crate::presentation::permission_response_result(response);
            json!({"kind":"permission_response","response":public.get("response").expect("response projection contains its response")})
        }
        InputControlReply::Receipt(receipt) => json!({"kind":"receipt","receipt":match receipt {
            ReceiptReply::Missing => json!({"status":"missing","input_id":null,"submission":null}),
            ReceiptReply::Present { input_id, submission } => json!({"status":"present","input_id":input_id,"submission":submission}),
            ReceiptReply::Retired { input_id } => json!({"status":"retired","input_id":input_id,"submission":null}),
            ReceiptReply::Settled { input_id } => json!({"status":"settled","input_id":input_id,"submission":null}),
        }}),
    }
}

pub(in crate::runtime::runner) fn encode_input_reply(
    reply: std::result::Result<&InputControlReply, &CafleetError>,
) -> Result<Vec<u8>> {
    let value = match reply {
        Ok(reply) => {
            let encoded = encode_result(reply);
            result(&encoded)?;
            json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"result":encoded})
        }
        Err(cause) => json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"error":cause.message()}),
    };
    frame_control_body(&serde_json::to_vec(&value).map_err(io_error)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::runner::test_support::Fixture;
    use serde_json::{Value, json};

    struct Case {
        fixture: Fixture,
    }

    const OPERATIONS: [(&str, Option<&str>); 7] = [
        ("submit_prompt", None),
        ("submit_exec", None),
        ("permission_inspect", None),
        ("permission_respond", None),
        ("receipt", Some("query")),
        ("receipt", Some("retire")),
        ("receipt", Some("reconcile")),
    ];

    #[test]
    fn missing_evidence_wire_values_follow_declaration_order() {
        use MissingEvidence::*;
        for value in [
            Operation,
            Arguments,
            Cwd,
            Environment,
            Targets,
            Effects,
            ChoiceScope,
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), json!(value.as_str()));
        }
    }

    impl Case {
        fn new() -> Self {
            Self {
                fixture: Fixture::new(),
            }
        }
        fn request(&self, kind: &str, action: Option<&str>) -> Value {
            let run = &self.fixture.run;
            let mut value = json!({"protocol_version":1,"kind":kind,"identity":{
                "database_identity_json":run.database_identity_json,"fleet_id":run.fleet_id,"member_id":run.member_id,
                "director_member_id":run.director_member_id,"run_id":run.run_id,"owner_epoch":run.owner_epoch,"launch_nonce":run.launch_nonce},
                "expires_at_mono_ns":50_000_000_000u64});
            let fields = match kind {
                "submit_prompt" => {
                    json!({"operation_key":"fixture:prompt","input_kind":"prompt","message_id":null,"literal":"literal 世界 '$x'"})
                }
                "submit_exec" => {
                    json!({"operation_key":"fixture:exec","input_kind":"exec","message_id":null,"literal":"printf literal"})
                }
                "permission_inspect" => {
                    json!({"interaction_id":12,"event_id":11,"director_member_id":run.director_member_id})
                }
                "permission_respond" => {
                    json!({"interaction_id":12,"event_id":11,"director_member_id":run.director_member_id,
                    "expected_run_id":run.run_id,"expected_owner_epoch":run.owner_epoch,"request_token":"a".repeat(64),"decision":"allow",
                    "decision_source":"director-auto","director_mode":"auto","mode_evidence_ref":"fixture:mode","decision_evidence_ref":"fixture:decision","operation_key":"fixture:response"})
                }
                "receipt" => match action.expect("receipt fixture specifies its action") {
                    "query" => json!({"action":"query","operation_key":"fixture:query"}),
                    "retire" => json!({"action":"retire","input_id":13}),
                    "reconcile" => {
                        json!({"action":"reconcile","input_id":13,"expected_state":"uncertain","disposition":"submitted","evidence_ref":"fixture:settlement"})
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            value
        }
        fn inspection(&self, supported: bool) -> Value {
            let run = &self.fixture.run;
            let evidence=crate::broker::runtime::evidence::EvidencePolicy::metadata().apply(&json!({
                "backend":"codex","native_version":run.native_version,"profile_version":null,"rule_id":"fixture-wire-evidence","screen_seq":2,
                "operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false
            })).unwrap();
            json!({"event_id":11,"message_id":14,"member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,
                "run_id":run.run_id,"owner_epoch":run.owner_epoch,"interaction_id":12,"episode_seq":1,
                "classification":if supported {json!("permission_request")} else {Value::Null},"visibility":if supported {"visible"} else {"unknown"},
                "backend":"codex","native_version":run.native_version,"profile_version":if supported {json!("fixture-detector")} else {Value::Null},
                "answer_profile_version":if supported {json!("fixture-answer")} else {Value::Null},"answer_rule_id":if supported {json!("fixture-rule")} else {Value::Null},
                "answer_capability":if supported {"tested"} else {"unknown"},"capability_reason":if supported {Value::Null} else {json!("fixture unsupported")},
                "supported_decisions":if supported {json!(["allow","deny"])} else {json!([])},"request_token":if supported {json!("a".repeat(64))} else {Value::Null},
                "output_seq":2,"evidence":evidence,
                "missing_evidence":["operation","arguments","cwd","environment","targets","effects","choice_scope"]})
        }
        fn response(&self) -> Value {
            let run = &self.fixture.run;
            json!({"status":"accepted","input_id":13,"member_id":run.member_id,"run_id":run.run_id,"owner_epoch":run.owner_epoch,
                "interaction_id":12,"event_id":11,"decision":"allow","decision_source":"director-auto","stage":"submitted","output_seq":2,"reason":null})
        }
    }
    fn bytes(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }
    fn decoded(value: &Value) -> InputControlRequest {
        decode_input_request(&bytes(value)).unwrap()
    }
    fn rejection(good: &Value, bad: &[u8]) -> CafleetError {
        decoded(good);
        match decode_input_request(bad) {
            Err(error) => error,
            Ok(_) => panic!("invalid original request rejects"),
        }
    }
    fn prefix(value: &Value, fields: &str) -> Vec<u8> {
        let original = serde_json::to_string(value).unwrap();
        format!("{{{fields}{}", &original[1..]).into_bytes()
    }
    fn duplicate(value: &Value, field: &str) -> Vec<u8> {
        assert!(value.get(field).is_some());
        prefix(
            value,
            &format!(
                "{}:{},",
                serde_json::to_string(field).unwrap(),
                value[field]
            ),
        )
    }
    fn nested_duplicate(value: &Value, pointer: &str, field: &str) -> Vec<u8> {
        let target = value.pointer(pointer).expect("the duplicate target exists");
        let original = serde_json::to_string(value).unwrap();
        let target_text = serde_json::to_string(target).unwrap();
        assert_eq!(original.matches(&target_text).count(), 1);
        original
            .replacen(
                &target_text,
                std::str::from_utf8(&duplicate(target, field)).unwrap(),
                1,
            )
            .into_bytes()
    }
    fn submission(status: &str, stage: &str, id: Option<i64>) -> Value {
        let delivery = match status {
            "accepted" => json!({"status":"submitted","method":"profile","reason":null}),
            "queued" => json!({"status":"queued","method":null,"reason":"interaction_open"}),
            "held" => json!({"status":"held","method":null,"reason":"observer_unavailable"}),
            "rejected" => json!({"status":"not_submitted","method":null,"reason":"runner_stopped"}),
            _ => json!({"status":"uncertain","method":"fallback","reason":"composer_unverified"}),
        };
        json!({"status":status,"input_id":id,"stage":stage,"output_seq":null,"reason":if status=="accepted" {Value::Null} else {json!("fixture outcome")},"delivery":delivery})
    }
    fn reply(kind: &str, field: &str, payload: Value) -> Value {
        let mut result = json!({"kind":kind});
        result[field] = payload;
        json!({"protocol_version":1,"result":result})
    }
    fn decoded_reply(good: &Value, request: &Value) -> InputControlReply {
        decode_input_reply(&bytes(good), &decoded(request)).unwrap()
    }
    fn reject_reply(good: &Value, bad: &[u8], request: &Value) {
        decoded_reply(good, request);
        assert!(
            decode_input_reply(bad, &decoded(request)).is_err(),
            "invalid original reply rejects"
        );
    }
    fn assert_submission(actual: &SubmissionResult, expected: &Value) {
        assert_eq!(actual.status.as_str(), expected["status"].as_str().unwrap());
        assert_eq!(actual.stage.as_str(), expected["stage"].as_str().unwrap());
        assert_eq!(actual.input_id, expected["input_id"].as_i64());
        assert_eq!(actual.output_seq, expected["output_seq"].as_u64());
        assert_eq!(actual.reason.as_deref(), expected["reason"].as_str());
        assert_eq!(
            serde_json::to_value(actual).unwrap()["delivery"],
            expected["delivery"]
        );
    }
    fn body(frame: &[u8]) -> &[u8] {
        assert!(frame.len() >= 4);
        assert_eq!(
            usize::try_from(u32::from_be_bytes(frame[..4].try_into().unwrap())).unwrap(),
            frame.len() - 4
        );
        &frame[4..]
    }
    fn assert_frame(frame: &[u8], expected: &Value) {
        assert!(body(frame).len() <= 65536);
        assert_eq!(
            serde_json::from_slice::<Value>(body(frame)).unwrap(),
            *expected
        );
    }
    fn padded(mut value: Value, pointer: &str, target: usize) -> Value {
        *value.pointer_mut(pointer).unwrap() = json!("");
        let overhead = bytes(&value).len();
        assert!(target >= overhead + 3);
        *value.pointer_mut(pointer).unwrap() =
            json!(format!("界{}", "x".repeat(target - overhead - 3)));
        assert_eq!(bytes(&value).len(), target);
        value
    }

    #[test]
    fn request_valid_flat_operations_and_lifecycle_routing_have_positive_baselines() {
        let case = Case::new();
        for (kind, action) in OPERATIONS {
            let value = case.request(kind, action);
            let request = decoded(&value);
            assert_eq!(
                request.identity,
                ControlIdentity::from_run(&case.fixture.run)
            );
            assert_eq!(request.expires_at_mono_ns, 50_000_000_000);
            match request.operation {
                InputControlOperation::SubmitPrompt(input) => {
                    assert_eq!(input.input_kind, InputKind::Prompt);
                    assert_eq!(input.message_id, None);
                    assert_eq!(input.literal, value["literal"].as_str().unwrap());
                }
                InputControlOperation::SubmitExec(input) => {
                    assert_eq!(input.input_kind, InputKind::Exec);
                    assert_eq!(input.message_id, None);
                }
                InputControlOperation::PermissionInspect(input) => {
                    assert_eq!(input.interaction_id, 12);
                    assert_eq!(input.event_id, 11);
                }
                InputControlOperation::PermissionRespond(input) => {
                    assert_eq!(input.expected_run_id, case.fixture.run.run_id);
                    assert_eq!(input.decision, PermissionDecision::Allow);
                    assert_eq!(input.request_token, "a".repeat(64));
                }
                InputControlOperation::Receipt(ReceiptAction::Query { operation_key }) => {
                    assert_eq!(operation_key, "fixture:query")
                }
                InputControlOperation::Receipt(ReceiptAction::Retire { input_id }) => {
                    assert_eq!(input_id, 13)
                }
                InputControlOperation::Receipt(ReceiptAction::Reconcile {
                    input_id,
                    expected,
                    disposition,
                    evidence_ref,
                }) => {
                    assert_eq!(input_id, 13);
                    assert_eq!(expected, InputState::Uncertain);
                    assert_eq!(disposition, InputDisposition::Submitted);
                    assert_eq!(evidence_ref, "fixture:settlement");
                }
            }
            assert!(matches!(
                decode_service_request(&bytes(&value)).unwrap(),
                ServiceControlRequest::Input(_)
            ));
        }
        let mut preview = case.request("submit_prompt", None);
        preview["input_kind"] = json!("preview");
        preview["message_id"] = json!(14);
        assert!(matches!(
            decoded(&preview).operation,
            InputControlOperation::SubmitPrompt(SubmitInput {
                input_kind: InputKind::Preview,
                message_id: Some(14),
                ..
            })
        ));
        for kind in ["health", "prepare", "start", "shutdown"] {
            let value = json!({"protocol_version":1,"kind":kind,"identity":case.request("submit_prompt",None)["identity"],"fixture_legacy":true});
            match decode_service_request(&bytes(&value)).unwrap() {
                ServiceControlRequest::Legacy(actual) => assert_eq!(actual, value),
                ServiceControlRequest::Input(_) => panic!("legacy routing remains compatible"),
            }
        }
    }

    #[test]
    fn request_input_then_lifecycle_duplicate_kind_cannot_escape_strict_routing() {
        let case = Case::new();
        let good = case.request("submit_prompt", None);
        decoded(&good);
        let mut legacy = good.clone();
        legacy["kind"] = json!("health");
        assert!(decode_service_request(&prefix(&legacy, r#""kind":"submit_prompt","#)).is_err());
    }
    #[test]
    fn request_lifecycle_then_input_duplicate_kind_cannot_escape_strict_routing() {
        let good = Case::new().request("submit_prompt", None);
        decoded(&good);
        assert!(decode_service_request(&prefix(&good, r#""kind":"health","#)).is_err());
    }
    #[test]
    fn request_every_envelope_and_operation_field_rejects_raw_duplication() {
        let case = Case::new();
        for (kind, action) in OPERATIONS {
            let good = case.request(kind, action);
            for field in good.as_object().unwrap().keys() {
                let raw = duplicate(&good, field);
                rejection(&good, &raw);
                assert!(decode_service_request(&raw).is_err(), "{kind} {field}");
            }
        }
    }
    #[test]
    fn request_identity_and_database_identity_reject_nested_raw_duplicates() {
        let good = Case::new().request("submit_prompt", None);
        for pointer in ["/identity", "/identity/database_identity_json"] {
            for field in good.pointer(pointer).unwrap().as_object().unwrap().keys() {
                rejection(&good, &nested_duplicate(&good, pointer, field));
            }
        }
    }
    #[test]
    fn request_required_fields_distinguish_absence_from_explicit_null() {
        let case = Case::new();
        for (kind, action) in OPERATIONS {
            let good = case.request(kind, action);
            for field in good.as_object().unwrap().keys() {
                let mut bad = good.clone();
                bad.as_object_mut().unwrap().remove(field);
                rejection(&good, &bytes(&bad));
            }
            for field in good["identity"].as_object().unwrap().keys() {
                let mut bad = good.clone();
                bad["identity"].as_object_mut().unwrap().remove(field);
                rejection(&good, &bytes(&bad));
            }
        }
    }
    #[test]
    fn request_unknown_fields_and_other_receipt_action_fields_reject() {
        let case = Case::new();
        for (kind, action) in OPERATIONS {
            let good = case.request(kind, action);
            for pointer in ["", "/identity"] {
                let mut bad = good.clone();
                bad.pointer_mut(pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), json!(true));
                rejection(&good, &bytes(&bad));
            }
        }
        for (action, field, value) in [
            ("query", "input_id", json!(13)),
            ("retire", "operation_key", json!("fixture:key")),
            ("retire", "expected_state", json!("ready")),
            ("reconcile", "operation_key", json!("fixture:key")),
        ] {
            let good = case.request("receipt", Some(action));
            let mut bad = good.clone();
            bad[field] = value;
            rejection(&good, &bytes(&bad));
        }
    }
    #[test]
    fn request_numeric_type_version_expiry_and_enum_domains_are_strict() {
        let case = Case::new();
        let good = case.request("permission_respond", None);
        for pointer in [
            "/protocol_version",
            "/expires_at_mono_ns",
            "/identity/run_id",
            "/identity/fleet_id",
            "/identity/member_id",
            "/identity/director_member_id",
            "/identity/owner_epoch",
            "/interaction_id",
            "/event_id",
            "/director_member_id",
            "/expected_run_id",
            "/expected_owner_epoch",
        ] {
            for invalid in [
                Value::Null,
                json!(true),
                json!("1"),
                json!(1.0),
                json!(0),
                json!(-1),
            ] {
                let mut bad = good.clone();
                *bad.pointer_mut(pointer).unwrap() = invalid;
                rejection(&good, &bytes(&bad));
            }
        }
        for invalid in [json!(2), json!(u64::MAX)] {
            let mut bad = good.clone();
            bad["protocol_version"] = invalid;
            rejection(&good, &bytes(&bad));
        }
        let mut maximum = good.clone();
        maximum["expires_at_mono_ns"] = json!(u64::MAX);
        decoded(&maximum);
        let overflow = String::from_utf8(bytes(&good))
            .unwrap()
            .replace("50000000000", "18446744073709551616");
        rejection(&good, overflow.as_bytes());
        let mut huge = good.clone();
        huge["event_id"] = json!(i64::MAX);
        decoded(&huge);
        huge["event_id"] = json!(u64::try_from(i64::MAX).unwrap() + 1);
        rejection(&good, &bytes(&huge));
        for (field, invalid) in [
            ("decision", "ALLOW"),
            ("decision_source", "director_auto"),
            ("director_mode", "AUTO"),
        ] {
            let mut bad = good.clone();
            bad[field] = json!(invalid);
            rejection(&good, &bytes(&bad));
        }
        for (kind, action, field, invalid) in [
            ("submit_prompt", None, "input_kind", "permission_response"),
            ("receipt", Some("query"), "action", "unknown"),
            ("receipt", Some("reconcile"), "expected_state", "unknown"),
            ("receipt", Some("reconcile"), "disposition", "unknown"),
        ] {
            let good = case.request(kind, action);
            let mut bad = good.clone();
            bad[field] = json!(invalid);
            rejection(&good, &bytes(&bad));
        }
    }
    #[test]
    fn request_kind_payload_permission_binding_and_reference_validation() {
        let case = Case::new();
        let mut literal_key = case.request("submit_prompt", None);
        literal_key["operation_key"] = json!("\nfixture:literal-key\u{7f}");
        match decoded(&literal_key).operation {
            InputControlOperation::SubmitPrompt(input) => {
                assert_eq!(input.operation_key, "\nfixture:literal-key\u{7f}")
            }
            _ => unreachable!(),
        }
        for (kind, field, invalid) in [
            ("submit_prompt", "input_kind", json!("exec")),
            ("submit_exec", "input_kind", json!("prompt")),
            ("submit_prompt", "message_id", json!(14)),
            ("submit_exec", "message_id", json!(14)),
            ("submit_prompt", "literal", json!(" ")),
            ("submit_exec", "literal", json!("a\nb")),
            ("submit_prompt", "operation_key", json!(" ")),
        ] {
            let good = case.request(kind, None);
            let mut bad = good.clone();
            bad[field] = invalid;
            rejection(&good, &bytes(&bad));
        }
        let good = case.request("submit_prompt", None);
        for byte in (0u8..=31).chain(std::iter::once(127)) {
            let mut bad = good.clone();
            bad["literal"] = json!(format!("a{}b", char::from(byte)));
            rejection(&good, &bytes(&bad));
        }
        let mut preview = good.clone();
        preview["input_kind"] = json!("preview");
        preview["message_id"] = json!(14);
        decoded(&preview);
        preview["message_id"] = Value::Null;
        rejection(&good, &bytes(&preview));
        let good = case.request("permission_respond", None);
        for (field, invalid) in [
            ("expected_run_id", json!(case.fixture.run.run_id + 1)),
            (
                "expected_owner_epoch",
                json!(case.fixture.run.owner_epoch + 1),
            ),
            ("director_member_id", json!(case.fixture.run.member_id)),
            ("director_mode", json!("manual")),
            ("request_token", json!("A".repeat(64))),
            ("request_token", json!("a".repeat(63))),
        ] {
            let mut bad = good.clone();
            bad[field] = invalid;
            rejection(&good, &bytes(&bad));
        }
        for field in ["mode_evidence_ref", "decision_evidence_ref"] {
            let mut edge = good.clone();
            edge[field] = json!("界".repeat(682) + "ab");
            decoded(&edge);
            for byte in (0u8..=31).chain(std::iter::once(127)) {
                let mut bad = good.clone();
                bad[field] = json!(format!("fixture:{}ref", char::from(byte)));
                rejection(&good, &bytes(&bad));
            }
            for invalid in [
                " ".into(),
                "x".repeat(2049),
                "a\nb".into(),
                "界".repeat(683),
            ] {
                let mut bad = good.clone();
                bad[field] = json!(invalid);
                rejection(&good, &bytes(&bad));
            }
        }
        for mode in ["auto", "manual", "unknown"] {
            let mut valid = good.clone();
            valid["decision_source"] = json!("user");
            valid["director_mode"] = json!(mode);
            decoded(&valid);
        }
    }
    #[test]
    fn request_encoding_is_flat_with_required_nulls_and_exact_utf8_frame_cap() {
        let case = Case::new();
        for (kind, action) in OPERATIONS {
            let good = case.request(kind, action);
            assert_frame(&encode_input_request(&decoded(&good)).unwrap(), &good);
        }
        let good = padded(case.request("submit_prompt", None), "/literal", 65536);
        assert_frame(&encode_input_request(&decoded(&good)).unwrap(), &good);
        let oversized = padded(case.request("submit_prompt", None), "/literal", 65537);
        let mut request = decoded(&case.request("submit_prompt", None));
        match &mut request.operation {
            InputControlOperation::SubmitPrompt(input) => {
                input.literal = oversized["literal"].as_str().unwrap().into()
            }
            _ => unreachable!(),
        }
        assert!(encode_input_request(&request).is_err());
        let mut invalid = decoded(&case.request("submit_prompt", None));
        invalid.expires_at_mono_ns = 0;
        assert!(encode_input_request(&invalid).is_err());
    }
    #[test]
    fn invalid_request_diagnostics_keep_payload_canaries_private() {
        let mut good = Case::new().request("permission_respond", None);
        let canaries = [
            "PRIVATE_KEY_7d69".into(),
            "12bd".repeat(16),
            "PRIVATE_MODE_981e".into(),
            "PRIVATE_DECISION_773a".into(),
        ];
        for (field, canary) in [
            "operation_key",
            "request_token",
            "mode_evidence_ref",
            "decision_evidence_ref",
        ]
        .into_iter()
        .zip(&canaries)
        {
            good[field] = json!(canary);
        }
        for field in ["request_token", "decision", "expected_run_id"] {
            let mut bad = good.clone();
            bad[field] = json!("PRIVATE_INVALID_8d14");
            let error = rejection(&good, &bytes(&bad));
            for canary in canaries
                .iter()
                .map(String::as_str)
                .chain(["PRIVATE_INVALID_8d14"])
            {
                assert!(!error.message().contains(canary));
            }
        }
        let mut good = Case::new().request("submit_prompt", None);
        good["literal"] = json!("PRIVATE_LITERAL_42a1");
        let error = rejection(&good, &duplicate(&good, "literal"));
        assert!(!error.message().contains("PRIVATE_LITERAL_42a1"));
    }

    #[test]
    fn reply_structured_submission_states_decode_with_exact_scalar_values() {
        let request = Case::new().request("submit_prompt", None);
        for (status, stage, id) in [
            ("accepted", "submitted", Some(13)),
            ("queued", "zero_bytes", Some(13)),
            ("held", "zero_bytes", None),
            ("rejected", "zero_bytes", None),
            ("uncertain", "partial", Some(13)),
            ("uncertain", "submitted", Some(13)),
            ("uncertain", "unknown", Some(13)),
        ] {
            let payload = submission(status, stage, id);
            let good = reply("submission", "submission", payload.clone());
            match decoded_reply(&good, &request) {
                InputControlReply::Submission(actual) => assert_submission(&actual, &payload),
                _ => panic!("submission result has exact kind"),
            }
        }
    }

    #[test]
    fn reply_queued_submission_requires_zero_bytes_a_durable_id_and_a_delivery_projection() {
        let request = Case::new().request("submit_prompt", None);
        let good = reply(
            "submission",
            "submission",
            submission("queued", "zero_bytes", Some(13)),
        );
        for (field, value) in [
            ("stage", json!("partial")),
            ("stage", json!("submitted")),
            ("input_id", Value::Null),
            ("delivery", Value::Null),
            (
                "delivery",
                json!({"status":"queued","method":"bogus","reason":"interaction_open"}),
            ),
            (
                "delivery",
                json!({"status":"queued","method":null,"reason":"interaction_open","extra":true}),
            ),
            ("delivery", json!({"status":"queued","method":null})),
        ] {
            let mut bad = good.clone();
            bad["result"]["submission"][field] = value;
            reject_reply(&good, &bytes(&bad), &request);
        }
        let mut missing = good.clone();
        missing["result"]["submission"]
            .as_object_mut()
            .unwrap()
            .remove("delivery");
        reject_reply(&good, &bytes(&missing), &request);
    }
    #[test]
    fn reply_inspection_supported_and_unknown_preserve_complete_public_object() {
        let case = Case::new();
        let request = case.request("permission_inspect", None);
        for supported in [false, true] {
            let payload = case.inspection(supported);
            let good = reply("inspection", "request", payload.clone());
            let actual = match decoded_reply(&good, &request) {
                InputControlReply::Inspection(actual) => actual,
                _ => panic!("inspection result has exact kind"),
            };
            assert_eq!(actual.event_id, 11);
            assert_eq!(actual.message_id, 14);
            assert_eq!(actual.member_id, case.fixture.run.member_id);
            assert_eq!(actual.fleet_id, case.fixture.run.fleet_id);
            assert_eq!(
                actual.director_member_id,
                case.fixture.run.director_member_id
            );
            assert_eq!(actual.run_id, case.fixture.run.run_id);
            assert_eq!(actual.owner_epoch, case.fixture.run.owner_epoch);
            assert_eq!(actual.interaction_id, 12);
            assert_eq!(actual.episode_seq, 1);
            assert_eq!(
                serde_json::to_value(actual.classification).unwrap(),
                payload["classification"]
            );
            assert_eq!(
                serde_json::to_value(actual.visibility).unwrap(),
                payload["visibility"]
            );
            assert_eq!(actual.backend, "codex");
            assert_eq!(actual.native_version, case.fixture.run.native_version);
            assert_eq!(
                actual.profile_version.as_deref(),
                payload["profile_version"].as_str()
            );
            assert_eq!(
                actual.answer_profile_version.as_deref(),
                payload["answer_profile_version"].as_str()
            );
            assert_eq!(
                actual.answer_rule_id.as_deref(),
                payload["answer_rule_id"].as_str()
            );
            assert_eq!(
                actual.answer_capability.as_str(),
                payload["answer_capability"].as_str().unwrap()
            );
            assert_eq!(
                actual.capability_reason.as_deref(),
                payload["capability_reason"].as_str()
            );
            assert_eq!(
                serde_json::to_value(&actual.supported_decisions).unwrap(),
                payload["supported_decisions"]
            );
            assert_eq!(
                actual.request_token.as_deref(),
                payload["request_token"].as_str()
            );
            assert_eq!(actual.output_seq, 2);
            assert_eq!(actual.evidence, payload["evidence"]);
            assert_eq!(
                serde_json::to_value(actual.missing_evidence).unwrap(),
                payload["missing_evidence"]
            );
        }
    }
    #[test]
    fn reply_permission_response_flattens_submission_into_public_shape() {
        let case = Case::new();
        let request = case.request("permission_respond", None);
        let payload = case.response();
        let good = reply("permission_response", "response", payload.clone());
        let actual = match decoded_reply(&good, &request) {
            InputControlReply::PermissionResponse(actual) => actual,
            _ => panic!("response result has exact kind"),
        };
        assert_eq!(actual.submission.input_id, Some(13));
        assert_eq!(actual.submission.status, SubmissionStatus::Accepted);
        assert_eq!(actual.submission.stage, SubmissionStage::Submitted);
        assert_eq!(actual.submission.output_seq, Some(2));
        assert_eq!(actual.submission.reason, None);
        assert_eq!(actual.member_id, case.fixture.run.member_id);
        assert_eq!(actual.run_id, case.fixture.run.run_id);
        assert_eq!(actual.owner_epoch, case.fixture.run.owner_epoch);
        assert_eq!(actual.interaction_id, 12);
        assert_eq!(actual.event_id, 11);
        assert_eq!(actual.decision, PermissionDecision::Allow);
        assert_eq!(actual.decision_source, DecisionSource::DirectorAuto);
        let mut nested = good.clone();
        nested["result"]["response"]["submission"] = submission("accepted", "submitted", Some(13));
        reject_reply(&good, &bytes(&nested), &request);
    }
    #[test]
    fn reply_receipt_variants_require_exact_ids_and_nulls() {
        let case = Case::new();
        for (action, status, id, payload) in [
            ("query", "missing", None, Value::Null),
            (
                "query",
                "present",
                Some(13),
                submission("accepted", "submitted", Some(13)),
            ),
            ("retire", "retired", Some(13), Value::Null),
            ("reconcile", "settled", Some(13), Value::Null),
        ] {
            let request = case.request("receipt", Some(action));
            let good = reply(
                "receipt",
                "receipt",
                json!({"status":status,"input_id":id,"submission":payload}),
            );
            match decoded_reply(&good, &request) {
                InputControlReply::Receipt(ReceiptReply::Missing) => assert_eq!(status, "missing"),
                InputControlReply::Receipt(ReceiptReply::Present {
                    input_id,
                    submission: actual,
                }) => {
                    assert_eq!(input_id, 13);
                    assert_submission(&actual, &payload)
                }
                InputControlReply::Receipt(ReceiptReply::Retired { input_id }) => {
                    assert_eq!(status, "retired");
                    assert_eq!(input_id, 13)
                }
                InputControlReply::Receipt(ReceiptReply::Settled { input_id }) => {
                    assert_eq!(status, "settled");
                    assert_eq!(input_id, 13)
                }
                _ => panic!("receipt result has exact kind"),
            }
        }
    }
    #[test]
    fn reply_envelope_requires_version_exact_fields_and_result_xor_text_error() {
        let request = Case::new().request("submit_prompt", None);
        let good = reply(
            "submission",
            "submission",
            submission("accepted", "submitted", Some(13)),
        );
        for field in ["protocol_version", "result"] {
            let mut bad = good.clone();
            bad.as_object_mut().unwrap().remove(field);
            reject_reply(&good, &bytes(&bad), &request);
        }
        for invalid in [
            Value::Null,
            json!(true),
            json!("1"),
            json!(1.0),
            json!(0),
            json!(2),
        ] {
            let mut bad = good.clone();
            bad["protocol_version"] = invalid;
            reject_reply(&good, &bytes(&bad), &request);
        }
        for invalid in [Value::Null, json!(true), json!({})] {
            let bad = json!({"protocol_version":1,"error":invalid});
            reject_reply(&good, &bytes(&bad), &request);
        }
        let mut both = good.clone();
        both["error"] = json!("fixture");
        reject_reply(&good, &bytes(&both), &request);
        let mut extra = good.clone();
        extra["unexpected"] = json!(true);
        reject_reply(&good, &bytes(&extra), &request);
        reject_reply(&good, &duplicate(&good, "protocol_version"), &request);
        reject_reply(&good, &duplicate(&good, "result"), &request);
    }
    #[test]
    fn reply_original_objects_reject_nested_duplicates() {
        let case = Case::new();
        for (request, good, paths) in [
            (
                case.request("submit_prompt", None),
                reply(
                    "submission",
                    "submission",
                    submission("accepted", "submitted", Some(13)),
                ),
                vec!["/result", "/result/submission"],
            ),
            (
                case.request("permission_inspect", None),
                reply("inspection", "request", case.inspection(true)),
                vec!["/result", "/result/request", "/result/request/evidence"],
            ),
            (
                case.request("receipt", Some("query")),
                reply(
                    "receipt",
                    "receipt",
                    json!({"status":"present","input_id":13,"submission":submission("accepted","submitted",Some(13))}),
                ),
                vec!["/result", "/result/receipt", "/result/receipt/submission"],
            ),
        ] {
            for path in paths {
                for field in good.pointer(path).unwrap().as_object().unwrap().keys() {
                    reject_reply(&good, &nested_duplicate(&good, path, field), &request);
                }
            }
        }
    }
    #[test]
    fn reply_missing_null_extra_and_domain_validation_is_recursive() {
        let case = Case::new();
        for (request, good, paths) in [
            (
                case.request("submit_prompt", None),
                reply(
                    "submission",
                    "submission",
                    submission("held", "zero_bytes", None),
                ),
                vec!["/result", "/result/submission"],
            ),
            (
                case.request("permission_inspect", None),
                reply("inspection", "request", case.inspection(false)),
                vec!["/result", "/result/request"],
            ),
            (
                case.request("permission_respond", None),
                reply("permission_response", "response", case.response()),
                vec!["/result", "/result/response"],
            ),
            (
                case.request("receipt", Some("query")),
                reply(
                    "receipt",
                    "receipt",
                    json!({"status":"missing","input_id":null,"submission":null}),
                ),
                vec!["/result", "/result/receipt"],
            ),
        ] {
            for path in paths {
                for field in good.pointer(path).unwrap().as_object().unwrap().keys() {
                    let mut bad = good.clone();
                    bad.pointer_mut(path)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(field);
                    reject_reply(&good, &bytes(&bad), &request);
                    let mut wrong_type = good.clone();
                    wrong_type.pointer_mut(path).unwrap()[field] = json!(true);
                    reject_reply(&good, &bytes(&wrong_type), &request);
                }
                let mut bad = good.clone();
                bad.pointer_mut(path)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), json!(true));
                reject_reply(&good, &bytes(&bad), &request);
            }
        }
        let request = case.request("permission_inspect", None);
        let good = reply("inspection", "request", case.inspection(true));
        let mut private_evidence = good.clone();
        private_evidence["result"]["request"]["evidence"]["operation"] =
            json!("PRIVATE_EVIDENCE_1d7a");
        decoded_reply(&good, &request);
        match decode_input_reply(&bytes(&private_evidence), &decoded(&request)) {
            Err(error) => assert!(!error.message().contains("PRIVATE_EVIDENCE_1d7a")),
            Ok(_) => panic!("invalid metadata evidence rejects before decoding success"),
        }
        for (field, value) in [
            ("visibility", json!("missing")),
            ("classification", json!("working")),
            ("answer_capability", json!("guessed")),
            ("supported_decisions", json!(["permit"])),
            ("missing_evidence", json!(["invented"])),
            ("output_seq", json!(-1)),
            ("event_id", json!(0)),
            ("request_token", json!("A".repeat(64))),
        ] {
            let mut bad = good.clone();
            bad["result"]["request"][field] = value;
            reject_reply(&good, &bytes(&bad), &request);
        }
        let request = case.request("submit_prompt", None);
        let good = reply(
            "submission",
            "submission",
            submission("accepted", "submitted", Some(13)),
        );
        for (field, value) in [
            ("status", json!("unknown")),
            ("stage", json!("invented")),
            ("input_id", json!(0)),
            ("output_seq", json!(-1)),
            ("reason", json!(true)),
        ] {
            let mut bad = good.clone();
            bad["result"]["submission"][field] = value;
            reject_reply(&good, &bytes(&bad), &request);
        }
    }
    #[test]
    fn reply_well_formed_other_operation_result_kind_rejects() {
        let case = Case::new();
        let pairs = [
            (
                case.request("submit_prompt", None),
                reply(
                    "submission",
                    "submission",
                    submission("accepted", "submitted", Some(13)),
                ),
            ),
            (
                case.request("permission_inspect", None),
                reply("inspection", "request", case.inspection(true)),
            ),
            (
                case.request("permission_respond", None),
                reply("permission_response", "response", case.response()),
            ),
            (
                case.request("receipt", Some("query")),
                reply(
                    "receipt",
                    "receipt",
                    json!({"status":"missing","input_id":null,"submission":null}),
                ),
            ),
        ];
        for (index, (request, good)) in pairs.iter().enumerate() {
            for (other, (_, alternative)) in pairs.iter().enumerate() {
                if index != other {
                    reject_reply(good, &bytes(alternative), request);
                }
            }
        }
    }
    #[test]
    fn reply_operation_specific_identities_and_receipt_ids_match_request() {
        let case = Case::new();
        for (request, good, payload, fields) in [
            (
                case.request("permission_inspect", None),
                reply("inspection", "request", case.inspection(true)),
                "request",
                vec![
                    "event_id",
                    "member_id",
                    "fleet_id",
                    "director_member_id",
                    "run_id",
                    "owner_epoch",
                    "interaction_id",
                ],
            ),
            (
                case.request("permission_respond", None),
                reply("permission_response", "response", case.response()),
                "response",
                vec![
                    "event_id",
                    "member_id",
                    "run_id",
                    "owner_epoch",
                    "interaction_id",
                ],
            ),
        ] {
            for field in fields {
                let mut bad = good.clone();
                bad["result"][payload][field] =
                    json!(good["result"][payload][field].as_i64().unwrap() + 1);
                reject_reply(&good, &bytes(&bad), &request);
            }
            if payload == "response" {
                for (field, value) in [("decision", "deny"), ("decision_source", "user")] {
                    let mut bad = good.clone();
                    bad["result"][payload][field] = json!(value);
                    reject_reply(&good, &bytes(&bad), &request);
                }
            }
        }
        let request = case.request("receipt", Some("query"));
        let good = reply(
            "receipt",
            "receipt",
            json!({"status":"present","input_id":13,"submission":submission("accepted","submitted",Some(13))}),
        );
        let mut bad = good.clone();
        bad["result"]["receipt"]["submission"]["input_id"] = json!(14);
        reject_reply(&good, &bytes(&bad), &request);
        for (action, status) in [("retire", "retired"), ("reconcile", "settled")] {
            let request = case.request("receipt", Some(action));
            let good = reply(
                "receipt",
                "receipt",
                json!({"status":status,"input_id":13,"submission":null}),
            );
            let mut bad = good.clone();
            bad["result"]["receipt"]["input_id"] = json!(14);
            reject_reply(&good, &bytes(&bad), &request);
        }
    }

    /// The keys of `result[field]` in wire order; `serde_json`'s `preserve_order` keeps them.
    fn result_keys(encoded: &[u8], field: &str) -> Vec<String> {
        let value: Value = serde_json::from_slice(body(encoded)).unwrap();
        value["result"][field]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn reply_encoding_matches_independent_shapes_public_order_and_body_cap() {
        let case = Case::new();
        for (request, expected) in [
            (
                case.request("submit_prompt", None),
                reply(
                    "submission",
                    "submission",
                    submission("accepted", "submitted", Some(13)),
                ),
            ),
            (
                case.request("permission_inspect", None),
                reply("inspection", "request", case.inspection(false)),
            ),
            (
                case.request("permission_respond", None),
                reply("permission_response", "response", case.response()),
            ),
            (
                case.request("receipt", Some("query")),
                reply(
                    "receipt",
                    "receipt",
                    json!({"status":"missing","input_id":null,"submission":null}),
                ),
            ),
        ] {
            let actual = decoded_reply(&expected, &request);
            let encoded = encode_input_reply(Ok(&actual)).unwrap();
            assert_frame(&encoded, &expected);
            if request["kind"] == "permission_inspect" {
                assert_eq!(result_keys(&encoded, "request"),"event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,profile_version,answer_profile_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq,evidence,missing_evidence".split(',').collect::<Vec<_>>());
            }
            if request["kind"] == "permission_respond" {
                assert_eq!(result_keys(&encoded, "response"),"status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,decision_source,stage,output_seq,reason".split(',').collect::<Vec<_>>());
            }
        }
        let expected = json!({"protocol_version":1,"error":"fixture failure"});
        assert_frame(
            &encode_input_reply(Err(&CafleetError::App("fixture failure".into()))).unwrap(),
            &expected,
        );
        let request = case.request("submit_prompt", None);
        let base = reply(
            "submission",
            "submission",
            submission("uncertain", "unknown", Some(13)),
        );
        let exact = padded(base.clone(), "/result/submission/reason", 65536);
        let actual = decoded_reply(&exact, &request);
        assert_frame(&encode_input_reply(Ok(&actual)).unwrap(), &exact);
        let oversized = padded(base, "/result/submission/reason", 65537);
        let actual = InputControlReply::Submission(SubmissionResult {
            status: SubmissionStatus::Uncertain,
            input_id: Some(13),
            stage: SubmissionStage::Unknown,
            output_seq: None,
            reason: Some(
                oversized["result"]["submission"]["reason"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
        });
        assert!(encode_input_reply(Ok(&actual)).is_err());
    }

    mod identity {
        use super::*;

        const PAYLOAD_CANARY: &str = "WIRE_R1_PRIVATE_LITERAL_CANARY";
        const IDENTITY_CANARY: &str = "WIRE_R1_PRIVATE_IDENTITY_CANARY";

        fn control(case: &Case) -> Value {
            let mut good = case.request("submit_prompt", None);
            good["literal"] = json!(PAYLOAD_CANARY);
            let request = decoded(&good);
            assert_eq!(
                request.identity,
                ControlIdentity::from_run(&case.fixture.run)
            );
            assert_frame(&encode_input_request(&request).unwrap(), &good);
            good
        }

        fn private_error(error: &CafleetError, good: &Value) {
            let diagnostic = format!("{error:?}\n{error}\n{}", error.message());
            for private in [
                PAYLOAD_CANARY,
                IDENTITY_CANARY,
                good["identity"]["database_identity_json"]["canonical_path"]
                    .as_str()
                    .unwrap(),
                good["identity"]["database_identity_json"]["instance_id"]
                    .as_str()
                    .unwrap(),
                good["identity"]["launch_nonce"].as_str().unwrap(),
            ] {
                assert!(
                    !diagnostic.contains(private),
                    "identity validation preserves payload privacy"
                );
            }
        }

        fn reject_decode(good: &Value, bad: &Value, label: &str) {
            match decode_input_request(&bytes(bad)) {
                Err(error) => private_error(&error, good),
                Ok(_) => panic!("owning identity schema rejects decode case: {label}"),
            }
        }

        fn reject_encode(good: &Value, database: Value, nonce: &str, label: &str) {
            let mut request = decoded(good);
            request.identity.database_identity_json = database;
            request.identity.launch_nonce = nonce.into();
            match encode_input_request(&request) {
                Err(error) => private_error(&error, good),
                Ok(_) => panic!("owning identity schema rejects constructed encode case: {label}"),
            }
        }

        fn bad_paths() -> Vec<Value> {
            vec![
                json!(format!("{IDENTITY_CANARY}/relative.sqlite")),
                json!(""),
                json!("."),
                json!("../database.sqlite"),
                Value::Null,
                json!(17),
                json!(true),
                json!([]),
                json!({}),
            ]
        }

        fn bad_hex(length: usize) -> Vec<Value> {
            vec![
                json!(""),
                json!("a".repeat(length - 1)),
                json!("a".repeat(length + 1)),
                json!("g".repeat(length)),
                json!(format!("{IDENTITY_CANARY}{}", "g".repeat(length))),
                json!(format!("{} ", "a".repeat(length - 1))),
                json!("é".repeat(length / 2)),
                Value::Null,
                json!(17),
                json!(true),
                json!([]),
                json!({}),
            ]
        }

        fn bad_numbers() -> Vec<Value> {
            vec![
                json!(-1),
                json!(i64::MIN),
                json!(1.0),
                json!(1.5),
                json!(18446744073709551616.0_f64),
                Value::Null,
                json!(IDENTITY_CANARY),
                json!(true),
                json!([]),
                json!({}),
            ]
        }

        #[test]
        fn identity_genuine_and_owning_domain_controls_preserve_literal_identity() {
            let case = Case::new();
            let good = control(&case);
            let mut uppercase = good.clone();
            uppercase["identity"]["database_identity_json"]["instance_id"] =
                json!("ABCDEF0123456789".repeat(2));
            uppercase["identity"]["launch_nonce"] = json!("ABCDEF0123456789".repeat(4));
            let mut mixed = uppercase.clone();
            mixed["identity"]["database_identity_json"]["instance_id"] =
                json!("aBcDeF0123456789".repeat(2));
            mixed["identity"]["launch_nonce"] = json!("aBcDeF0123456789".repeat(4));
            let mut extended = good.clone();
            extended["identity"]["database_identity_json"]["extension"] =
                json!({"fixture":IDENTITY_CANARY,"nested":[1,null]});
            let mut bounds = good.clone();
            bounds["identity"]["database_identity_json"]["device"] = json!(0);
            bounds["identity"]["database_identity_json"]["inode"] = json!(u64::MAX);
            let mut reverse_bounds = good.clone();
            reverse_bounds["identity"]["database_identity_json"]["device"] = json!(u64::MAX);
            reverse_bounds["identity"]["database_identity_json"]["inode"] = json!(0);
            let mut lexical_absolute = good.clone();
            lexical_absolute["identity"]["database_identity_json"]["canonical_path"] = json!(
                case.fixture
                    ._dir
                    .path()
                    .join("absent-parent/../unallocated.sqlite")
            );
            for value in [
                good,
                uppercase,
                mixed,
                extended,
                bounds,
                reverse_bounds,
                lexical_absolute,
            ] {
                let decoded = decode_input_request(&bytes(&value)).unwrap();
                assert_eq!(
                    decoded.identity.database_identity_json,
                    value["identity"]["database_identity_json"]
                );
                assert_eq!(
                    decoded.identity.launch_nonce,
                    value["identity"]["launch_nonce"].as_str().unwrap()
                );
                assert_frame(&encode_input_request(&decoded).unwrap(), &value);
            }
        }

        #[test]
        fn identity_decode_requires_each_owning_database_identity_field() {
            let case = Case::new();
            let good = control(&case);
            for field in ["canonical_path", "instance_id", "device", "inode"] {
                let mut bad = good.clone();
                bad["identity"]["database_identity_json"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field)
                    .unwrap();
                reject_decode(&good, &bad, field);
            }
        }

        #[test]
        fn identity_decode_requires_an_absolute_string_database_path() {
            let case = Case::new();
            let good = control(&case);
            for (index, value) in bad_paths().into_iter().enumerate() {
                let mut bad = good.clone();
                bad["identity"]["database_identity_json"]["canonical_path"] = value;
                reject_decode(&good, &bad, &format!("canonical_path {index}"));
            }
        }

        #[test]
        fn identity_decode_requires_a_32_character_ascii_hex_instance_id() {
            let case = Case::new();
            let good = control(&case);
            for (index, value) in bad_hex(32).into_iter().enumerate() {
                let mut bad = good.clone();
                bad["identity"]["database_identity_json"]["instance_id"] = value;
                reject_decode(&good, &bad, &format!("instance_id {index}"));
            }
        }

        fn reject_numeric_field(field: &str) {
            let case = Case::new();
            let good = control(&case);
            for (index, value) in bad_numbers().into_iter().enumerate() {
                let mut bad = good.clone();
                bad["identity"]["database_identity_json"][field] = value;
                reject_decode(&good, &bad, &format!("{field} {index}"));
            }
        }

        #[test]
        fn identity_decode_requires_an_unsigned_64_bit_device() {
            reject_numeric_field("device");
        }

        #[test]
        fn identity_decode_requires_an_unsigned_64_bit_inode() {
            reject_numeric_field("inode");
        }

        #[test]
        fn identity_decode_requires_a_64_character_ascii_hex_launch_nonce() {
            let case = Case::new();
            let good = control(&case);
            for (index, value) in bad_hex(64).into_iter().enumerate() {
                let mut bad = good.clone();
                bad["identity"]["launch_nonce"] = value;
                reject_decode(&good, &bad, &format!("launch_nonce {index}"));
            }
        }

        #[test]
        fn identity_encoder_requires_each_owning_database_identity_field() {
            let case = Case::new();
            let good = control(&case);
            for field in ["canonical_path", "instance_id", "device", "inode"] {
                let mut database = good["identity"]["database_identity_json"].clone();
                database.as_object_mut().unwrap().remove(field).unwrap();
                reject_encode(&good, database, &case.fixture.run.launch_nonce, field);
            }
        }

        #[test]
        fn identity_encoder_validates_constructed_database_path_and_instance_id() {
            let case = Case::new();
            let good = control(&case);
            for (field, values) in [
                ("canonical_path", bad_paths()),
                ("instance_id", bad_hex(32)),
            ] {
                for (index, value) in values.into_iter().enumerate() {
                    let mut database = good["identity"]["database_identity_json"].clone();
                    database[field] = value;
                    reject_encode(
                        &good,
                        database,
                        &case.fixture.run.launch_nonce,
                        &format!("{field} {index}"),
                    );
                }
            }
        }

        #[test]
        fn identity_encoder_validates_constructed_unsigned_device_and_inode() {
            let case = Case::new();
            let good = control(&case);
            for field in ["device", "inode"] {
                for (index, value) in bad_numbers().into_iter().enumerate() {
                    let mut database = good["identity"]["database_identity_json"].clone();
                    database[field] = value;
                    reject_encode(
                        &good,
                        database,
                        &case.fixture.run.launch_nonce,
                        &format!("{field} {index}"),
                    );
                }
            }
        }

        #[test]
        fn identity_encoder_validates_a_constructed_launch_nonce() {
            let case = Case::new();
            let good = control(&case);
            for value in bad_hex(64) {
                if let Some(nonce) = value.as_str() {
                    reject_encode(
                        &good,
                        good["identity"]["database_identity_json"].clone(),
                        nonce,
                        "launch_nonce",
                    );
                }
            }
        }
    }
}
