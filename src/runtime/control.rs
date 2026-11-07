use crate::broker::runtime as storage;
use crate::broker::runtime::DecisionSource;
use crate::broker::runtime::DeliveryProjection;
use crate::broker::runtime::DeliveryStatus;
use crate::broker::runtime::DirectorMode;
use crate::broker::runtime::InputDisposition;
use crate::broker::runtime::InputKind;
use crate::broker::runtime::InputState;
use crate::broker::runtime::InteractionClass;
use crate::broker::runtime::PermissionDecision;
use crate::broker::runtime::RunRecord;
use crate::broker::stored_enum;
use crate::coding_agent::catalog::AnswerKey;
use crate::coding_agent::catalog::ChoiceScope;
use crate::coding_agent::catalog::valid_choice_id;
use crate::runtime::*;
use rusqlite::Connection;
use serde::Deserializer;
use serde::de::DeserializeOwned;
use serde::de::IgnoredAny;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::Visitor;
use serde_json::Map;
use serde_json::Number;
use std::fmt;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

pub(crate) const CONTROL_PROTOCOL_VERSION: u32 = 1;
pub(crate) const MAX_CONTROL_FRAME: usize = 65536;

pub(super) fn validate_version(frame: &Value) -> Result<()> {
    ensure(
        frame.is_object()
            && frame.get("protocol_version").and_then(Value::as_u64)
                == Some(u64::from(CONTROL_PROTOCOL_VERSION)),
        "control frame requires integer protocol_version 1",
    )
}

pub(crate) fn encode_control_frame(frame: &Value) -> Result<Vec<u8>> {
    validate_version(frame)?;
    let body = serde_json::to_vec(frame).map_err(io_error)?;
    frame_control_body(&body)
}

pub(super) fn frame_control_body(body: &[u8]) -> Result<Vec<u8>> {
    ensure(
        body.len() <= MAX_CONTROL_FRAME,
        "control frame exceeds size limit",
    )?;
    let mut bytes = u32::try_from(body.len())
        .map_err(io_error)?
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(body);
    Ok(bytes)
}

pub(super) fn decode_lifecycle_control_body(body: &[u8]) -> Result<Value> {
    let frame = serde_json::from_slice(body).map_err(|_| error("malformed control JSON body"))?;
    validate_version(&frame)?;
    Ok(frame)
}

pub(super) struct AdmittedControlFrame {
    body: Vec<u8>,
    admitted_at: Instant,
}
impl AdmittedControlFrame {
    pub(super) fn body(&self) -> &[u8] {
        &self.body
    }
    pub(super) fn admitted_at(&self) -> Instant {
        self.admitted_at
    }
}

pub(crate) struct ControlDecoder {
    header: [u8; 4],
    header_len: usize,
    body: Vec<u8>,
    expected: Option<usize>,
    admitted_at: Option<Instant>,
}

impl ControlDecoder {
    pub(crate) fn new() -> Self {
        Self {
            header: [0; 4],
            header_len: 0,
            body: Vec::new(),
            expected: None,
            admitted_at: None,
        }
    }

    pub(super) fn feed_admitted(
        &mut self,
        mut bytes: &[u8],
        observed_at: Instant,
    ) -> Result<Vec<AdmittedControlFrame>> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if self.expected.is_none() {
                if self.header_len == 0 {
                    self.admitted_at = Some(observed_at);
                }
                let count = (4 - self.header_len).min(bytes.len());
                self.header[self.header_len..self.header_len + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_len += count;
                bytes = &bytes[count..];
                if self.header_len < 4 {
                    continue;
                }
                let length = usize::try_from(u32::from_be_bytes(self.header)).map_err(io_error)?;
                ensure(
                    length <= MAX_CONTROL_FRAME,
                    "control frame exceeds size limit",
                )?;
                self.expected = Some(length);
            }
            let length = self
                .expected
                .expect("a complete header declares the body length");
            let count = (length - self.body.len()).min(bytes.len());
            self.body.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.body.len() == length {
                frames.push(AdmittedControlFrame {
                    body: std::mem::take(&mut self.body),
                    admitted_at: self
                        .admitted_at
                        .take()
                        .expect("a complete frame consumed its first header byte"),
                });
                self.header_len = 0;
                self.expected = None;
            }
        }
        Ok(frames)
    }

    pub(crate) fn finish(&self) -> Result<()> {
        ensure(
            self.header_len == 0 && self.expected.is_none() && self.admitted_at.is_none(),
            "truncated control frame",
        )
    }
}

stored_enum!(SubmissionStatus, "submission.status", {
    Accepted => "accepted",
    Queued => "queued",
    Held => "held",
    Rejected => "rejected",
    Uncertain => "uncertain",
});
stored_enum!(SubmissionStage, "submission.stage", {
    ZeroBytes => "zero_bytes",
    Partial => "partial",
    Submitted => "submitted",
    Unknown => "unknown",
});
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmissionResult {
    pub(crate) status: SubmissionStatus,
    pub(crate) input_id: Option<i64>,
    pub(crate) stage: SubmissionStage,
    pub(crate) output_seq: Option<u64>,
    pub(crate) reason: Option<String>,
    #[serde(default)]
    pub(crate) delivery: Option<DeliveryProjection>,
}
impl SubmissionResult {
    pub(crate) fn held(reason: &str) -> Self {
        Self {
            status: SubmissionStatus::Held,
            input_id: None,
            stage: SubmissionStage::ZeroBytes,
            output_seq: None,
            reason: Some(reason.into()),
            delivery: Some(DeliveryProjection {
                status: DeliveryStatus::Held,
                method: None,
                reason: Some(reason.into()),
            }),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PermissionInspectInput {
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    pub(crate) director_member_id: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PermissionRespondInput {
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) expected_run_id: i64,
    pub(crate) expected_owner_epoch: i64,
    pub(crate) request_token: String,
    #[serde(flatten, with = "answer_wire")]
    pub(crate) key: AnswerKey,
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
#[derive(Deserialize)]
pub(crate) struct PermissionResponseResult {
    #[serde(flatten)]
    pub(crate) submission: SubmissionResult,
    pub(crate) member_id: i64,
    pub(crate) run_id: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) interaction_id: i64,
    pub(crate) event_id: i64,
    #[serde(flatten, with = "answer_wire")]
    pub(crate) key: AnswerKey,
    pub(crate) decision_source: DecisionSource,
}

/// The wire's separate `decision` and `choice` fields, exactly one of them non-null.
mod answer_wire {
    use crate::broker::runtime::PermissionDecision;
    use crate::coding_agent::catalog::AnswerKey;
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;

    #[derive(Serialize, Deserialize)]
    struct Answer {
        decision: Option<PermissionDecision>,
        choice: Option<String>,
    }

    pub(super) fn serialize<S: Serializer>(
        key: &AnswerKey,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        Answer {
            decision: key.decision(),
            choice: key.choice().map(str::to_owned),
        }
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<AnswerKey, D::Error> {
        match Answer::deserialize(deserializer)? {
            Answer {
                decision: Some(decision),
                choice: None,
            } => Ok(AnswerKey::Decision(decision)),
            Answer {
                decision: None,
                choice: Some(choice),
            } => Ok(AnswerKey::Choice(choice)),
            _ => Err(serde::de::Error::custom(
                "control answer requires exactly one decision or choice",
            )),
        }
    }
}
// Declaration order is also the required reply order.
stored_enum!(values MissingEvidence, "missing_evidence", {
    Operation => "operation",
    Arguments => "arguments",
    Cwd => "cwd",
    Environment => "environment",
    Targets => "targets",
    Effects => "effects",
    ChoiceScope => "choice_scope",
});
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportedChoice {
    pub(crate) choice_id: String,
    pub(crate) label: String,
    pub(crate) scope: ChoiceScope,
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
    pub(crate) answer_rule_id: Option<String>,
    pub(crate) answer_capability: crate::broker::runtime::Coverage,
    pub(crate) capability_reason: Option<String>,
    pub(crate) supported_decisions: Vec<PermissionDecision>,
    pub(crate) supported_choices: Vec<SupportedChoice>,
    pub(crate) request_token: Option<String>,
    pub(crate) output_seq: u64,
    pub(crate) evidence: Value,
    pub(crate) missing_evidence: Vec<MissingEvidence>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SubmitInput {
    pub(crate) operation_key: String,
    pub(crate) input_kind: InputKind,
    pub(crate) message_id: Option<i64>,
    pub(crate) literal: String,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum ReceiptAction {
    Query {
        operation_key: String,
    },
    Retire {
        input_id: i64,
    },
    Reconcile {
        input_id: i64,
        #[serde(rename = "expected_state")]
        expected: InputState,
        disposition: InputDisposition,
        evidence_ref: String,
    },
}
#[derive(Serialize, Deserialize)]
pub(crate) struct InputControlRequest {
    pub(crate) identity: ControlIdentity,
    pub(crate) expires_at_mono_ns: u64,
    #[serde(flatten)]
    pub(crate) operation: InputControlOperation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

/// A control frame: `protocol_version` beside the flattened request or reply.
#[derive(Serialize, Deserialize)]
struct Frame<T> {
    protocol_version: u32,
    #[serde(flatten)]
    body: T,
}

fn wire<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(io_error)
}

fn public(mut projection: Value, field: &str) -> Value {
    projection[field].take()
}

/// Decodes `value` and requires `encode` to reproduce it exactly, so an unknown, missing or
/// defaulted field is refused like a wrongly typed one.
fn decode_exact<T: DeserializeOwned>(
    value: &Value,
    encode: impl FnOnce(&T) -> Result<Value>,
) -> Result<T> {
    let decoded: T =
        serde_json::from_value(value.clone()).map_err(|_| error("invalid control fields"))?;
    ensure(
        encode(&decoded)? == *value,
        "control object requires its exact fields",
    )?;
    Ok(decoded)
}

fn positive(ids: &[i64]) -> Result<()> {
    ensure(
        ids.iter().all(|id| *id > 0),
        "positive control identity required",
    )
}

fn validate_choice(key: &AnswerKey) -> Result<()> {
    ensure(
        key.choice().is_none_or(valid_choice_id),
        "control choice requires the choice-id shape",
    )
}

impl InputControlRequest {
    fn validate(&self) -> Result<()> {
        let identity = &self.identity;
        crate::broker::runtime::validate_identity(&identity.database_identity_json)
            .map_err(|_| error("invalid control database identity"))?;
        crate::broker::runtime::hex(&identity.launch_nonce, 64, "launch_nonce")
            .map_err(|_| error("invalid control launch nonce"))?;
        positive(&[
            identity.fleet_id,
            identity.member_id,
            identity.director_member_id,
            identity.run_id,
            identity.owner_epoch,
        ])?;
        ensure(
            self.expires_at_mono_ns > 0,
            "positive input expiration required",
        )?;
        match &self.operation {
            InputControlOperation::SubmitPrompt(input)
            | InputControlOperation::SubmitExec(input) => {
                validate_operation_key(&input.operation_key)?;
                ensure(
                    crate::pane_lock::valid_input(&input.literal),
                    "valid single-line input literal required",
                )?;
                let prompt = matches!(self.operation, InputControlOperation::SubmitPrompt(_));
                let valid_kind = match (prompt, input.input_kind, input.message_id) {
                    (true, InputKind::Preview, Some(id)) => id > 0,
                    (true, InputKind::Prompt, None) | (false, InputKind::Exec, None) => true,
                    _ => false,
                };
                ensure(
                    valid_kind,
                    "control input kind and message must match the operation",
                )
            }
            InputControlOperation::PermissionInspect(input) => {
                positive(&[
                    input.interaction_id,
                    input.event_id,
                    input.director_member_id,
                ])?;
                ensure(
                    input.director_member_id == identity.director_member_id,
                    "control Director identity mismatch",
                )
            }
            InputControlOperation::PermissionRespond(input) => {
                validate_choice(&input.key)?;
                ensure(
                    input.director_member_id == identity.director_member_id
                        && input.expected_run_id == identity.run_id
                        && input.expected_owner_epoch == identity.owner_epoch,
                    "control permission identity mismatch",
                )?;
                crate::runtime::input_client::validate_respond(input)
            }
            InputControlOperation::Receipt(ReceiptAction::Query { operation_key }) => {
                validate_operation_key(operation_key)
            }
            InputControlOperation::Receipt(ReceiptAction::Retire { input_id }) => {
                positive(&[*input_id])
            }
            InputControlOperation::Receipt(ReceiptAction::Reconcile {
                input_id,
                evidence_ref,
                ..
            }) => {
                validate_permission_reference(evidence_ref)?;
                positive(&[*input_id])
            }
        }
    }
}

fn decode_input_request(body: &[u8]) -> Result<InputControlRequest> {
    let value = decode_strict_json(body)?;
    validate_version(&value)?;
    let request = decode_exact(&value, wire::<Frame<InputControlRequest>>)?.body;
    request.validate()?;
    Ok(request)
}

/// A lifecycle control request; each variant carries the frame's `protocol_version`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LifecycleRequest {
    Prepare {
        protocol_version: u32,
        identity: ControlIdentity,
        remaining_ms: u64,
    },
    Start {
        protocol_version: u32,
        identity: ControlIdentity,
        pane_id: Option<String>,
    },
    Health {
        protocol_version: u32,
        identity: ControlIdentity,
    },
    Shutdown {
        protocol_version: u32,
        identity: ControlIdentity,
    },
}
impl LifecycleRequest {
    pub(crate) fn health(run: &RunRecord) -> Self {
        Self::Health {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: ControlIdentity::from_run(run),
        }
    }
    pub(crate) fn shutdown(run: &RunRecord) -> Self {
        Self::Shutdown {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: ControlIdentity::from_run(run),
        }
    }
}

pub(in crate::runtime) enum ServiceControlRequest {
    Lifecycle(LifecycleRequest),
    Input(Box<InputControlRequest>),
}
pub(in crate::runtime) fn decode_service_request(body: &[u8]) -> Result<ServiceControlRequest> {
    if routes_input(body)? {
        decode_input_request(body).map(|request| ServiceControlRequest::Input(Box::new(request)))
    } else {
        serde_json::from_value(decode_lifecycle_control_body(body)?)
            .map(ServiceControlRequest::Lifecycle)
            .map_err(|_| error("invalid lifecycle control fields"))
    }
}

fn encode_frame<T: Serialize>(body: T) -> Result<Vec<u8>> {
    let frame = Frame {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        body,
    };
    frame_control_body(&serde_json::to_vec(&frame).map_err(io_error)?)
}

pub(in crate::runtime) fn encode_input_request(request: &InputControlRequest) -> Result<Vec<u8>> {
    request.validate()?;
    encode_frame(request)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyBody<T> {
    Result(T),
    Error(String),
}

/// The `{protocol_version, result | error}` reply envelope.
pub(in crate::runtime) fn encode_reply<T: Serialize>(
    reply: std::result::Result<T, &CafleetError>,
) -> Result<Vec<u8>> {
    encode_frame(match reply {
        Ok(result) => ReplyBody::Result(result),
        Err(cause) => ReplyBody::Error(cause.message().into_owned()),
    })
}

/// Decodes an exact reply envelope; a runner error becomes `runner control: <cause>`.
fn decode_reply<T: Serialize + DeserializeOwned>(body: &[u8]) -> Result<T> {
    let value = decode_strict_json(body)?;
    validate_version(&value)?;
    match decode_exact(&value, wire::<Frame<ReplyBody<T>>>)?.body {
        ReplyBody::Error(cause) => Err(error(format!("runner control: {cause}"))),
        ReplyBody::Result(result) => Ok(result),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ResultWire {
    Submission { submission: Value },
    Inspection { request: Value },
    PermissionResponse { response: Value },
    Receipt { receipt: ReceiptWire },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ReceiptWire {
    Missing { input_id: (), submission: () },
    Present { input_id: i64, submission: Value },
    Retired { input_id: i64, submission: () },
    Settled { input_id: i64, submission: () },
}

fn positive_input(submission: &SubmissionResult) -> Result<()> {
    ensure(
        submission.input_id.is_none_or(|id| id > 0),
        "positive control input ID required",
    )
}

fn validate_submission(submission: &SubmissionResult) -> Result<()> {
    positive_input(submission)?;
    let queued = submission.status == SubmissionStatus::Queued;
    ensure(
        queued
            == submission
                .delivery
                .as_ref()
                .is_some_and(|delivery| delivery.status == DeliveryStatus::Queued),
        "queued submission and queued delivery must agree",
    )?;
    if queued {
        ensure(
            submission.stage == SubmissionStage::ZeroBytes && submission.input_id.is_some(),
            "queued submission requires zero bytes and a durable input ID",
        )?;
    }
    Ok(())
}

fn validate_inspection(inspection: &PermissionInspection) -> Result<()> {
    positive(&[
        inspection.event_id,
        inspection.message_id,
        inspection.member_id,
        inspection.fleet_id,
        inspection.director_member_id,
        inspection.run_id,
        inspection.owner_epoch,
        inspection.interaction_id,
        inspection.episode_seq,
    ])?;
    if let Some(token) = &inspection.request_token {
        validate_request_token(token)?;
    }
    validate_order(
        inspection
            .supported_decisions
            .iter()
            .map(|decision| decision.as_str()),
        PermissionDecision::VALUES,
    )?;
    validate_order(
        inspection
            .missing_evidence
            .iter()
            .map(|missing| missing.as_str()),
        MissingEvidence::VALUES,
    )?;
    let mut choices = std::collections::BTreeSet::new();
    ensure(
        inspection
            .supported_choices
            .iter()
            .all(|choice| valid_choice_id(&choice.choice_id) && choices.insert(&choice.choice_id)),
        "control inspection choices require unique shaped choice ids",
    )?;
    ensure(
        inspection.supported_choices.is_empty()
            || (inspection.supported_decisions.is_empty()
                && inspection.classification == Some(InteractionClass::Question)),
        "control inspection choices belong only to a question",
    )?;
    crate::broker::runtime::evidence::validate(&inspection.evidence)
        .map_err(|_| error("invalid control inspection evidence"))
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

fn validate_reply(reply: &InputControlReply) -> Result<()> {
    match reply {
        InputControlReply::Submission(submission) => validate_submission(submission),
        InputControlReply::Inspection(inspection) => validate_inspection(inspection),
        InputControlReply::PermissionResponse(response) => {
            positive_input(&response.submission)?;
            positive(&[
                response.member_id,
                response.run_id,
                response.owner_epoch,
                response.interaction_id,
                response.event_id,
            ])?;
            validate_choice(&response.key)
        }
        InputControlReply::Receipt(ReceiptReply::Missing) => Ok(()),
        InputControlReply::Receipt(ReceiptReply::Present {
            input_id,
            submission,
        }) => {
            positive(&[*input_id])?;
            validate_submission(submission)?;
            ensure(
                submission.input_id == Some(*input_id),
                "receipt input identity mismatch",
            )
        }
        InputControlReply::Receipt(
            ReceiptReply::Retired { input_id } | ReceiptReply::Settled { input_id },
        ) => positive(&[*input_id]),
    }
}

fn decode_result(result: ResultWire) -> Result<InputControlReply> {
    let submission = |value: &Value| decode_exact(value, wire::<SubmissionResult>);
    Ok(match result {
        ResultWire::Submission { submission: value } => {
            InputControlReply::Submission(submission(&value)?)
        }
        ResultWire::Inspection { request } => {
            InputControlReply::Inspection(Box::new(decode_exact(&request, |inspection| {
                Ok(public(
                    crate::output::json::permission_inspection(inspection),
                    "request",
                ))
            })?))
        }
        ResultWire::PermissionResponse { response } => {
            InputControlReply::PermissionResponse(decode_exact(&response, |response| {
                Ok(public(
                    crate::output::json::permission_response_result(response),
                    "response",
                ))
            })?)
        }
        ResultWire::Receipt { receipt } => InputControlReply::Receipt(match receipt {
            ReceiptWire::Missing { .. } => ReceiptReply::Missing,
            ReceiptWire::Present {
                input_id,
                submission: value,
            } => ReceiptReply::Present {
                input_id,
                submission: submission(&value)?,
            },
            ReceiptWire::Retired { input_id, .. } => ReceiptReply::Retired { input_id },
            ReceiptWire::Settled { input_id, .. } => ReceiptReply::Settled { input_id },
        }),
    })
}

fn validate_binding(reply: &InputControlReply, request: &InputControlRequest) -> Result<()> {
    let identity = &request.identity;
    let matches = match (&request.operation, reply) {
        (
            InputControlOperation::SubmitPrompt(_) | InputControlOperation::SubmitExec(_),
            InputControlReply::Submission(submission),
        ) => submission.delivery.is_some(),
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
                && result.key == input.key
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

pub(in crate::runtime) fn decode_input_reply(
    body: &[u8],
    request: &InputControlRequest,
) -> Result<InputControlReply> {
    let reply = decode_result(decode_reply(body)?)?;
    validate_reply(&reply)?;
    validate_binding(&reply, request)?;
    Ok(reply)
}

fn encode_result(reply: &InputControlReply) -> Result<ResultWire> {
    Ok(match reply {
        InputControlReply::Submission(submission) => ResultWire::Submission {
            submission: wire(submission)?,
        },
        InputControlReply::Inspection(inspection) => ResultWire::Inspection {
            request: public(
                crate::output::json::permission_inspection(inspection),
                "request",
            ),
        },
        InputControlReply::PermissionResponse(response) => ResultWire::PermissionResponse {
            response: public(
                crate::output::json::permission_response_result(response),
                "response",
            ),
        },
        InputControlReply::Receipt(receipt) => ResultWire::Receipt {
            receipt: match receipt {
                ReceiptReply::Missing => ReceiptWire::Missing {
                    input_id: (),
                    submission: (),
                },
                ReceiptReply::Present {
                    input_id,
                    submission,
                } => ReceiptWire::Present {
                    input_id: *input_id,
                    submission: wire(submission)?,
                },
                ReceiptReply::Retired { input_id } => ReceiptWire::Retired {
                    input_id: *input_id,
                    submission: (),
                },
                ReceiptReply::Settled { input_id } => ReceiptWire::Settled {
                    input_id: *input_id,
                    submission: (),
                },
            },
        },
    })
}

pub(in crate::runtime) fn encode_input_reply(
    reply: std::result::Result<&InputControlReply, &CafleetError>,
) -> Result<Vec<u8>> {
    let result = match reply {
        Ok(reply) => {
            validate_reply(reply)?;
            Ok(encode_result(reply)?)
        }
        Err(cause) => Err(cause),
    };
    encode_reply(result)
}

pub(super) fn pause(deadline: Instant) -> Result<()> {
    check_deadline(Instant::now(), deadline)?;
    nap_until(deadline);
    Ok(())
}
pub(super) struct ControlClient {
    pub(crate) stream: UnixStream,
    decoder: ControlDecoder,
}

#[derive(Clone, Copy)]
enum EndpointPhase {
    Starting,
    Ready,
}

impl ControlClient {
    pub(super) fn connect(path: &Path, deadline: Instant) -> Result<Self> {
        Self::connect_in_phase(path, deadline, EndpointPhase::Ready)
    }

    pub(super) fn connect_starting(path: &Path, deadline: Instant) -> Result<Self> {
        Self::connect_in_phase(path, deadline, EndpointPhase::Starting)
    }

    fn connect_in_phase(path: &Path, deadline: Instant, phase: EndpointPhase) -> Result<Self> {
        use nix::sys::socket::AddressFamily;
        use nix::sys::socket::SockFlag;
        use nix::sys::socket::SockType;
        use nix::sys::socket::UnixAddr;
        use nix::sys::socket::connect;
        use nix::sys::socket::socket;
        loop {
            check_deadline(Instant::now(), deadline)?;
            let meta = match std::fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(cause)
                    if cause.kind() == ErrorKind::NotFound
                        && matches!(phase, EndpointPhase::Starting) =>
                {
                    pause(deadline)?;
                    continue;
                }
                Err(cause) => return Err(io_error(cause)),
            };
            ensure(
                meta.file_type().is_socket() && meta.uid() == nix::unistd::geteuid().as_raw(),
                "private control socket identity required",
            )?;
            if meta.mode() & 0o777 != 0o600 {
                if matches!(phase, EndpointPhase::Starting) {
                    pause(deadline)?;
                    continue;
                }
                return Err(error("private control socket identity required"));
            }
            let fd = socket(
                AddressFamily::Unix,
                SockType::Stream,
                SockFlag::empty(),
                None,
            )
            .map_err(io_error)?;
            nix::fcntl::fcntl(
                &fd,
                nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
            )
            .map_err(io_error)?;
            nix::fcntl::fcntl(
                &fd,
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )
            .map_err(io_error)?;
            match connect(fd.as_raw_fd(), &UnixAddr::new(path).map_err(io_error)?) {
                Ok(()) | Err(nix::errno::Errno::EINPROGRESS) => {
                    return Ok(Self {
                        stream: UnixStream::from(fd),
                        decoder: ControlDecoder::new(),
                    });
                }
                Err(nix::errno::Errno::EAGAIN) => pause(deadline)?,
                Err(nix::errno::Errno::ECONNREFUSED | nix::errno::Errno::ENOENT)
                    if matches!(phase, EndpointPhase::Starting) =>
                {
                    pause(deadline)?;
                }
                Err(cause) => return Err(io_error(cause)),
            }
        }
    }
    pub(super) fn exchange(
        &mut self,
        request: &LifecycleRequest,
        deadline: Instant,
    ) -> Result<Value> {
        let bytes = encode_control_frame(&wire(request)?)?;
        let frame = self.exchange_bytes(&bytes, deadline)?;
        decode_reply(frame.body())
    }
    pub(super) fn exchange_input(
        &mut self,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        check_deadline(Instant::now(), deadline)?;
        let bytes = encode_input_request(request)?;
        let frame = self.exchange_bytes(&bytes, deadline)?;
        let reply = decode_input_reply(frame.body(), request)?;
        check_deadline(Instant::now(), deadline)?;
        Ok(reply)
    }
    fn exchange_bytes(&mut self, bytes: &[u8], deadline: Instant) -> Result<AdmittedControlFrame> {
        let mut offset = 0;
        while offset < bytes.len() {
            check_deadline(Instant::now(), deadline)?;
            match self.stream.write(&bytes[offset..]) {
                Ok(0) => return Err(error("control endpoint disconnected during write")),
                Ok(count) => offset += count,
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    pause(deadline)?
                }
                Err(cause) => return Err(io_error(cause)),
            }
        }
        loop {
            check_deadline(Instant::now(), deadline)?;
            let mut bytes = [0; 4096];
            match self.stream.read(&mut bytes) {
                Ok(0) => {
                    self.decoder.finish()?;
                    return Err(error("control endpoint disconnected before reply"));
                }
                Ok(count) => {
                    let observed_at = Instant::now();
                    let frames = self.decoder.feed_admitted(&bytes[..count], observed_at)?;
                    if !frames.is_empty() {
                        ensure(frames.len() == 1, "unexpected multiple control replies")?;
                        self.decoder.finish()?;
                        return Ok(frames.into_iter().next().expect("one control reply"));
                    }
                }
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    pause(deadline)?
                }
                Err(cause) => return Err(io_error(cause)),
            }
        }
    }
}
pub(super) fn request(
    run: &RunRecord,
    request: LifecycleRequest,
    deadline: Instant,
) -> Result<Value> {
    let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
    ControlClient::connect(&paths.control_socket, deadline)?.exchange(&request, deadline)
}

pub(crate) fn request_shutdown(
    conn: &mut rusqlite::Connection,
    run: &RunRecord,
    deadline: Instant,
) -> Result<ShutdownOutcome> {
    validate_run_paths(conn, run)?;
    let result = request(run, LifecycleRequest::shutdown(run), deadline)?;
    let outcome: ShutdownOutcome = serde_json::from_value(result).map_err(io_error)?;
    let current = storage::get_run(conn, run.run_id)?;
    ensure(
        current.owner_epoch == run.owner_epoch,
        "shutdown ownership changed",
    )?;
    Ok(outcome)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HealthSnapshot {
    pub(crate) identity: ControlIdentity,
    pub(crate) runner: ProcessIdentity,
    pub(crate) child: Option<ProcessIdentity>,
    pub(crate) output_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) observation: Option<ObservationHealth>,
}
impl HealthSnapshot {
    pub(crate) fn decode(value: Value, run: &RunRecord) -> Result<Self> {
        let result: Self = serde_json::from_value(value).map_err(io_error)?;
        result.validate(run)?;
        Ok(result)
    }
    pub(crate) fn validate(&self, run: &RunRecord) -> Result<()> {
        self.identity.validate(run)?;
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        ensure(
            run.runner_pid == Some(self.runner.pid)
                && run.runner_start_identity.as_deref()
                    == Some(self.runner.start_identity.as_str())
                && run.owner_pid == self.runner.pid
                && run.owner_start_identity == self.runner.start_identity
                && self.runner.matches()?,
            "health runner generation mismatch",
        )?;
        match &self.child {
            Some(child) => ensure(
                run.child_pid == Some(child.pid)
                    && run.child_start_identity.as_deref() == Some(child.start_identity.as_str())
                    && child.matches()?,
                "health child generation mismatch",
            ),
            None => ensure(
                run.child_pid.is_none(),
                "health omitted native child identity",
            ),
        }
    }
}
pub(crate) fn probe_health(run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot> {
    let reply = request(run, LifecycleRequest::health(run), deadline)?;
    HealthSnapshot::decode(reply, run)
}
/// A doctor row's status, ordered by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CheckStatus {
    Ok,
    Info,
    Warn,
    Error,
}

impl CheckStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberState {
    Running,
    NotResponding,
    LostRuntime,
    InterruptedLaunch,
    AgentExited,
    DeletedWithLeftoverRuntime,
    CheckFailed,
    NotChecked,
    Ended,
}

impl MemberState {
    /// Summary order of the states doctor prints; `Ended` is hidden from the summary.
    pub(crate) const SUMMARY_ORDER: [Self; 8] = [
        Self::Running,
        Self::NotResponding,
        Self::LostRuntime,
        Self::InterruptedLaunch,
        Self::AgentExited,
        Self::DeletedWithLeftoverRuntime,
        Self::CheckFailed,
        Self::NotChecked,
    ];

    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NotResponding => "not_responding",
            Self::LostRuntime => "lost_runtime",
            Self::InterruptedLaunch => "interrupted_launch",
            Self::AgentExited => "agent_exited",
            Self::DeletedWithLeftoverRuntime => "deleted_with_leftover_runtime",
            Self::CheckFailed => "check_failed",
            Self::NotChecked => "not_checked",
            Self::Ended => "ended",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NotResponding => "not responding",
            Self::LostRuntime => "lost runtime",
            Self::InterruptedLaunch => "interrupted launch",
            Self::AgentExited => "agent exited",
            Self::DeletedWithLeftoverRuntime => "deleted with leftover runtime",
            Self::CheckFailed => "could not be checked",
            Self::NotChecked => "not checked (time limit)",
            Self::Ended => "ended",
        }
    }

    pub(crate) fn status(self) -> CheckStatus {
        match self {
            Self::Running => CheckStatus::Ok,
            Self::NotChecked | Self::Ended => CheckStatus::Info,
            Self::CheckFailed => CheckStatus::Error,
            Self::NotResponding
            | Self::LostRuntime
            | Self::InterruptedLaunch
            | Self::AgentExited
            | Self::DeletedWithLeftoverRuntime => CheckStatus::Warn,
        }
    }

    /// Next commands with `ID` standing for the member ID and `DIRECTOR` for its fleet's
    /// Director ID.
    pub(crate) fn next_commands(self) -> &'static [&'static str] {
        const RECOVER: &str = "cafleet member recover ID --from-member-id DIRECTOR";
        match self {
            Self::Running | Self::Ended => &[],
            Self::NotResponding | Self::InterruptedLaunch | Self::DeletedWithLeftoverRuntime => {
                &[RECOVER]
            }
            Self::LostRuntime => &[RECOVER, "cafleet member delete ID"],
            Self::AgentExited => &["cafleet member delete ID"],
            Self::CheckFailed => &["cafleet doctor --verbose"],
            Self::NotChecked => &["cafleet member show ID"],
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum OwnerLockProbe {
    Held,
    Free,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) enum HealthProbe {
    Answered,
    Skipped,
    Failed(String),
}

pub(crate) enum RuntimeProbes {
    InvalidPaths(String),
    Observed {
        descriptor_leftover: bool,
        lock: OwnerLockProbe,
        health: HealthProbe,
    },
}

impl RuntimeProbes {
    pub(crate) fn classify(self) -> RuntimeDiagnosis {
        let (descriptor_leftover, lock, health) = match self {
            Self::InvalidPaths(cause) => {
                return RuntimeDiagnosis {
                    state: MemberState::InterruptedLaunch,
                    diagnostics: vec![format!("runtime files are invalid: {cause}")],
                };
            }
            Self::Observed {
                descriptor_leftover,
                lock,
                health,
            } => (descriptor_leftover, lock, health),
        };
        let held = match lock {
            OwnerLockProbe::Held => true,
            OwnerLockProbe::Free => false,
            OwnerLockProbe::Failed(cause) => {
                return RuntimeDiagnosis {
                    state: MemberState::CheckFailed,
                    diagnostics: vec![cause],
                };
            }
        };
        let mut diagnostics = Vec::new();
        if descriptor_leftover {
            diagnostics.push("launch descriptor left over".to_owned());
        }
        diagnostics.push(
            if held {
                "owner lock is held"
            } else {
                "owner lock is free"
            }
            .to_owned(),
        );
        let health_state = match health {
            HealthProbe::Answered => {
                diagnostics.push("runtime answered the health probe".to_owned());
                MemberState::Running
            }
            HealthProbe::Skipped => MemberState::NotChecked,
            HealthProbe::Failed(cause) => {
                diagnostics.push(format!("runtime did not answer the health probe: {cause}"));
                if held {
                    MemberState::NotResponding
                } else {
                    MemberState::LostRuntime
                }
            }
        };
        let state = if descriptor_leftover {
            MemberState::InterruptedLaunch
        } else {
            health_state
        };
        RuntimeDiagnosis { state, diagnostics }
    }
}

pub(crate) struct RuntimeDiagnosis {
    pub(crate) state: MemberState,
    pub(crate) diagnostics: Vec<String>,
}
/// Diagnoses `run_id` through its own read-only connection to the registry file at `source`.
pub(crate) fn diagnose_runtime_at(
    source: &str,
    run_id: i64,
    deadline: Instant,
) -> Result<RuntimeDiagnosis> {
    let mut observed = crate::db::connect_read_only(&format!("sqlite:///{source}"))?;
    with_database_deadline(&mut observed, deadline, |conn| {
        diagnose_runtime_observed(conn, run_id, deadline)
    })
}
fn diagnose_runtime_observed(
    conn: &Connection,
    run_id: i64,
    deadline: Instant,
) -> Result<RuntimeDiagnosis> {
    check_deadline(Instant::now(), deadline)?;
    let run = storage::get_run(conn, run_id)?;
    let probes = match validate_run_paths(conn, &run) {
        Ok(paths) => {
            let descriptor_leftover = paths.descriptor.try_exists().map_err(io_error)?;
            let lock = match RunLock::acquire(conn, &run.owner()) {
                Ok(Some(_)) => OwnerLockProbe::Free,
                Ok(None) => OwnerLockProbe::Held,
                Err(cause) => OwnerLockProbe::Failed(cause.message().into_owned()),
            };
            let health = if Instant::now() < deadline {
                match probe_health(&run, deadline) {
                    Ok(_) => HealthProbe::Answered,
                    Err(cause) => HealthProbe::Failed(cause.to_string()),
                }
            } else {
                HealthProbe::Skipped
            };
            RuntimeProbes::Observed {
                descriptor_leftover,
                lock,
                health,
            }
        }
        Err(cause) => RuntimeProbes::InvalidPaths(cause.to_string()),
    };
    Ok(probes.classify())
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::fs::PermissionsExt;

    use std::os::unix::net::UnixListener;

    use std::path::Path;

    use nix::sys::socket::AddressFamily;
    use nix::sys::socket::Backlog;
    use nix::sys::socket::SockFlag;
    use nix::sys::socket::SockType;
    use nix::sys::socket::UnixAddr;
    use nix::sys::socket::bind;
    use nix::sys::socket::listen;
    use nix::sys::socket::socket;

    use crate::runtime::control::*;
    use crate::runtime::test_support::Fixture;

    pub(in crate::runtime) struct Endpoint {
        pub(in crate::runtime) fixture: Fixture,
        artifacts: RunArtifacts,
        pub(in crate::runtime) descriptor: LaunchDescriptor,
    }

    impl Endpoint {
        pub(in crate::runtime) fn new() -> Self {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let descriptor = LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap();
            Self {
                fixture,
                artifacts,
                descriptor,
            }
        }

        pub(in crate::runtime) fn path(&self) -> &Path {
            &self.artifacts.paths.control_socket
        }

        pub(in crate::runtime) fn prepared(&self) -> Prepared {
            let mut identity = ControlIdentity::from_run(&self.fixture.run);
            identity.owner_epoch += 1;
            Prepared {
                identity,
                runner: ProcessIdentity::current().unwrap(),
            }
        }
    }

    pub(in crate::runtime) fn bound_socket(
        path: &Path,
        mode: u32,
        listening: bool,
    ) -> UnixListener {
        let fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .unwrap();
        bind(fd.as_raw_fd(), &UnixAddr::new(path).unwrap()).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        let listener = UnixListener::from(fd);
        listener.set_nonblocking(true).unwrap();
        assert!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_socket()
        );
        if listening {
            listen(&listener, Backlog::new(8).unwrap()).unwrap();
        }
        listener
    }

    #[derive(Clone, Copy)]
    pub(in crate::runtime) enum StartupInterval {
        Bound,
        BeforePrivateMode,
    }

    /// A `u32` big-endian length prefix followed by `body`, built without the production framer.
    pub(crate) fn independent_frame(body: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    }

    pub(crate) fn feed_values(decoder: &mut ControlDecoder, bytes: &[u8]) -> Result<Vec<Value>> {
        decoder
            .feed_admitted(bytes, Instant::now())?
            .into_iter()
            .map(|frame| decode_lifecycle_control_body(frame.body()))
            .collect()
    }

    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::Instant;

    use rusqlite::params;

    pub(crate) fn activate(fixture: &mut Fixture) -> (RunArtifacts, RunLock) {
        let artifacts = fixture.artifacts();
        let held = transfer_setting(
            fixture,
            "status='running',runner_pid=?1,runner_start_identity=?2,child_pid=?1,child_start_identity=?2",
        );
        (artifacts, held)
    }

    /// Transfers the run to this process, then applies the `member_runs` SET list `columns`,
    /// where `?1` is this process's pid and `?2` its start identity.
    pub(crate) fn transfer_setting(fixture: &mut Fixture, columns: &str) -> RunLock {
        let held = RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .unwrap();
        let process = ProcessIdentity::current().unwrap();
        let next = held
            .transfer_to_runner(&mut fixture.conn, &fixture.run.owner(), process.clone())
            .unwrap();
        fixture
            .conn
            .execute(
                &format!("UPDATE member_runs SET {columns} WHERE run_id=?3"),
                params![process.pid, process.start_identity, next.run_id],
            )
            .unwrap();
        fixture.run = storage::get_run(&fixture.conn, next.run_id).unwrap();
        held
    }

    pub(crate) struct SilentServer {
        pub(crate) stop: Arc<AtomicBool>,
        pub(crate) thread: Option<std::thread::JoinHandle<()>>,
    }

    impl SilentServer {
        pub(crate) fn start(path: &std::path::Path) -> Self {
            let listener = UnixListener::bind(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let observed = stop.clone();
            let thread = std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(2);
                let mut connections = Vec::new();
                while !observed.load(Ordering::SeqCst) && Instant::now() < end {
                    match listener.accept() {
                        Ok((connection, _)) => connections.push(connection),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => panic!("fixture socket accept failed: {error}"),
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            Self {
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for SilentServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    use serde_json::json;

    /// A tested, visible permission inspection with every identity field set.
    pub(crate) fn inspection() -> PermissionInspection {
        serde_json::from_value(json!({
            "event_id":11,"message_id":14,"member_id":3,"fleet_id":2,"director_member_id":1,
            "run_id":7,"owner_epoch":2,"interaction_id":12,"episode_seq":1,
            "classification":"permission_request","visibility":"visible",
            "backend":"opencode","native_version":"2.1.4","answer_rule_id":"opencode-shell-allow-enter",
            "answer_capability":"tested","capability_reason":null,"supported_decisions":["allow","deny"],
            "supported_choices":[],
            "request_token":"a".repeat(64),"output_seq":5,"evidence":{"rule_id":"opencode-shell-permission"},
            "missing_evidence":["arguments","cwd"]
        }))
        .expect("the fixture inspection matches the reply shape")
    }

    /// A tested, visible question inspection offering two dismissal choices.
    pub(crate) fn question_inspection() -> PermissionInspection {
        serde_json::from_value(json!({
            "event_id":11,"message_id":14,"member_id":3,"fleet_id":2,"director_member_id":1,
            "run_id":7,"owner_epoch":2,"interaction_id":12,"episode_seq":1,
            "classification":"question","visibility":"visible",
            "backend":"claude","native_version":"2.1.312","answer_rule_id":null,
            "answer_capability":"tested","capability_reason":null,"supported_decisions":[],
            "supported_choices":[
                {"choice_id":"not-now","label":"Not now","scope":"persistent"},
                {"choice_id":"dont-show-again","label":"Don't show again","scope":"persistent"}
            ],
            "request_token":"a".repeat(64),"output_seq":5,"evidence":{"rule_id":"claude-auto-startup"},
            "missing_evidence":[]
        }))
        .expect("the fixture question inspection matches the reply shape")
    }
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::catalog::AnswerKey;
    use crate::runtime::control::test_support::feed_values;
    use crate::runtime::control::test_support::independent_frame as frame;
    use crate::runtime::control::*;

    #[test]
    fn check_statuses_order_by_severity_and_render_their_wire_names() {
        let mut statuses = [
            CheckStatus::Error,
            CheckStatus::Ok,
            CheckStatus::Warn,
            CheckStatus::Info,
        ];
        statuses.sort();
        assert_eq!(
            statuses,
            [
                CheckStatus::Ok,
                CheckStatus::Info,
                CheckStatus::Warn,
                CheckStatus::Error
            ]
        );
        assert_eq!(
            statuses.map(CheckStatus::as_str),
            ["ok", "info", "warn", "error"]
        );
        assert_eq!(MemberState::Running.status(), CheckStatus::Ok);
        assert_eq!(MemberState::CheckFailed.status(), CheckStatus::Error);
        assert_eq!(MemberState::LostRuntime.status(), CheckStatus::Warn);
        assert_eq!(MemberState::Ended.status(), CheckStatus::Info);
    }

    mod input_frames_tests {
        use crate::runtime::control::tests::*;
        use std::time::Duration;
        use std::time::Instant;

        fn later(origin: Instant, millis: u64) -> Instant {
            origin.checked_add(Duration::from_millis(millis)).unwrap()
        }

        #[test]
        fn frames_every_split_and_single_byte_feed_retains_first_consumption_time() {
            let body = br#"{"protocol_version":1,"kind":"health","text":"literal \u4e16\u754c"}"#;
            let value: Value = serde_json::from_slice(body).unwrap();
            let bytes = frame(body);
            let t = Instant::now();
            for cut in 0..bytes.len() {
                let mut decoder = ControlDecoder::new();
                assert!(decoder.feed_admitted(&bytes[..cut], t).unwrap().is_empty());
                let frames = decoder.feed_admitted(&bytes[cut..], later(t, 100)).unwrap();
                assert_eq!(frames.len(), 1, "split {cut}");
                assert_eq!(frames[0].body(), body, "split {cut}");
                assert_eq!(
                    decode_lifecycle_control_body(frames[0].body()).unwrap(),
                    value,
                    "split {cut}"
                );
                assert_eq!(
                    frames[0].admitted_at(),
                    if cut == 0 { later(t, 100) } else { t },
                    "split {cut}"
                );
                decoder.finish().unwrap();
            }
            let mut decoder = ControlDecoder::new();
            let mut frames = Vec::new();
            for (index, byte) in bytes.iter().enumerate() {
                frames.extend(
                    decoder
                        .feed_admitted(&[*byte], later(t, u64::try_from(index).unwrap()))
                        .unwrap(),
                );
            }
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].body(), body);
            assert_eq!(frames[0].admitted_at(), t);
            decoder.finish().unwrap();
        }

        #[test]
        fn frames_empty_feeds_leave_admission_unset_and_preserve_partial_frame_time() {
            let t = Instant::now();
            let body = br#"{"protocol_version":1,"kind":"health"}"#;
            let bytes = frame(body);
            let mut decoder = ControlDecoder::new();
            assert!(decoder.feed_admitted(&[], t).unwrap().is_empty());
            decoder.finish().unwrap();
            assert!(
                decoder
                    .feed_admitted(&bytes[..1], later(t, 10))
                    .unwrap()
                    .is_empty()
            );
            assert!(decoder.finish().is_err());
            assert!(decoder.feed_admitted(&[], later(t, 20)).unwrap().is_empty());
            let frames = decoder.feed_admitted(&bytes[1..], later(t, 30)).unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].body(), body);
            assert_eq!(frames[0].admitted_at(), later(t, 10));
            decoder.finish().unwrap();
        }

        #[test]
        fn frames_partial_complete_and_new_tail_keep_each_first_header_time() {
            let bodies = [b"first".as_slice(), b"second", b"third"];
            let encoded = bodies.map(frame);
            let t = Instant::now();
            let mut decoder = ControlDecoder::new();
            assert!(
                decoder
                    .feed_admitted(&encoded[0][..2], t)
                    .unwrap()
                    .is_empty()
            );
            let mut next = encoded[0][2..].to_vec();
            next.extend_from_slice(&encoded[1]);
            next.extend_from_slice(&encoded[2][..1]);
            let frames = decoder.feed_admitted(&next, later(t, 10)).unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!(frames[0].body(), bodies[0]);
            assert_eq!(frames[0].admitted_at(), t);
            assert_eq!(frames[1].body(), bodies[1]);
            assert_eq!(frames[1].admitted_at(), later(t, 10));
            assert!(decoder.finish().is_err());
            let tail = decoder
                .feed_admitted(&encoded[2][1..], later(t, 20))
                .unwrap();
            assert_eq!(tail.len(), 1);
            assert_eq!(tail[0].body(), bodies[2]);
            assert_eq!(tail[0].admitted_at(), later(t, 10));
            decoder.finish().unwrap();
        }

        #[test]
        fn frames_inclusive_body_limit_and_header_only_oversize_rejection() {
            let t = Instant::now();
            let body = vec![b'x'; 65536];
            let mut decoder = ControlDecoder::new();
            let frames = decoder.feed_admitted(&frame(&body), t).unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].body(), body);
            assert_eq!(frames[0].admitted_at(), t);
            decoder.finish().unwrap();
            for size in [65537u32, u32::MAX] {
                let mut decoder = ControlDecoder::new();
                let header = size.to_be_bytes();
                for byte in &header[..3] {
                    assert!(decoder.feed_admitted(&[*byte], t).unwrap().is_empty());
                }
                assert!(
                    decoder.feed_admitted(&header[3..], later(t, 10)).is_err(),
                    "declared {size}"
                );
            }
        }

        #[test]
        fn frames_eof_rejects_each_partial_boundary_and_empty_body_reaches_consumer() {
            let t = Instant::now();
            let body = br#"{"protocol_version":1,"kind":"health"}"#;
            let bytes = frame(body);
            ControlDecoder::new().finish().unwrap();
            for cut in 1..bytes.len() {
                let mut decoder = ControlDecoder::new();
                assert!(decoder.feed_admitted(&bytes[..cut], t).unwrap().is_empty());
                assert!(decoder.finish().is_err(), "prefix {cut}");
            }
            let mut decoder = ControlDecoder::new();
            let frames = decoder.feed_admitted(&frame(&[]), t).unwrap();
            assert_eq!(frames.len(), 1);
            assert!(frames[0].body().is_empty());
            assert_eq!(frames[0].admitted_at(), t);
            decoder.finish().unwrap();
            assert!(feed_values(&mut ControlDecoder::new(), &frame(&[])).is_err());
        }

        #[test]
        fn frames_raw_bytes_preserve_whitespace_and_invalid_json_for_consumer_validation() {
            let t = Instant::now();
            for (body, valid) in [
            (
                b"{\n \"protocol_version\":1,\"kind\":\"health\",\"text\":\"literal \\u00e9\"\n}"
                    .as_slice(),
                true,
            ),
            (b"{".as_slice(), false),
            (b"\xff".as_slice(), false),
            (b"null".as_slice(), false),
        ] {
            let mut decoder = ControlDecoder::new();
            let frames = decoder.feed_admitted(&frame(body), t).unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].body(), body);
            assert_eq!(frames[0].admitted_at(), t);
            decoder.finish().unwrap();
            assert_eq!(
                feed_values(&mut ControlDecoder::new(), &frame(body)).is_ok(),
                valid
            );
        }
        }
    }
    mod control_codec_tests {
        use crate::runtime::control::test_support::transfer_setting;
        use crate::runtime::control::tests::*;
        use crate::runtime::test_support::Fixture;
        use crate::runtime::test_support::read_fixture_bytes;
        use crate::runtime::test_support::write_fixture_bytes;
        use serde_json::Value;
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        use std::time::Duration;
        use std::time::Instant;

        #[test]
        fn runner_review_codec_encoding_pins_big_endian_length_and_required_version() {
            for value in [
                json!({"protocol_version":1,"kind":"health","identity":{},"text":"literal\n世界"}),
                json!({"protocol_version":1,"result":{"text":"literal\nreply"}}),
                json!({"protocol_version":1,"error":"literal\nerror"}),
            ] {
                let body = serde_json::to_vec(&value).unwrap();
                assert_eq!(encode_control_frame(&value).unwrap(), frame(&body));
            }
        }

        #[test]
        fn runner_review_codec_keeps_json_whitespace_and_multiple_frames_independent() {
            let request = b"{\n  \"protocol_version\": 1,\n  \"kind\": \"health\"\n}";
            let reply = json!({"protocol_version":1,"result":"line one\nline two"});
            let mut bytes = frame(request);
            bytes.extend(frame(&serde_json::to_vec(&reply).unwrap()));
            let mut decoder = ControlDecoder::new();
            assert_eq!(
                feed_values(&mut decoder, &bytes).unwrap(),
                [serde_json::from_slice::<Value>(request).unwrap(), reply]
            );
            decoder.finish().unwrap();
        }

        #[test]
        fn runner_review_codec_requires_integer_supported_versions_in_requests_and_replies() {
            for envelope in [
                json!({"kind":"health"}),
                json!({"result":null}),
                json!({"error":"failure"}),
            ] {
                for version in [
                    None,
                    Some(Value::Null),
                    Some(json!("1")),
                    Some(json!(1.0)),
                    Some(json!(2)),
                    Some(json!(-1)),
                ] {
                    let mut value = envelope.clone();
                    if let Some(version) = version {
                        value["protocol_version"] = version;
                    }
                    assert!(encode_control_frame(&value).is_err(), "{value}");
                    let mut decoder = ControlDecoder::new();
                    assert!(
                        feed_values(&mut decoder, &frame(&serde_json::to_vec(&value).unwrap()))
                            .is_err(),
                        "{value}"
                    );
                }
            }
        }

        #[test]
        fn runner_review_codec_rejects_empty_malformed_and_nonobject_bodies() {
            for body in [b"".as_slice(), b"{", b"\xff", b"[]", b"null", b"1"] {
                let mut decoder = ControlDecoder::new();
                assert!(feed_values(&mut decoder, &frame(body)).is_err(), "{body:?}");
            }
            for value in [Value::Null, json!([]), json!(1)] {
                assert!(encode_control_frame(&value).is_err());
            }
        }

        #[test]
        fn runner_review_codec_accepts_the_inclusive_body_cap_without_an_aggregate_frame_cap() {
            let mut value = json!({"protocol_version":1,"result":""});
            let overhead = serde_json::to_vec(&value).unwrap().len();
            value["result"] = json!("x".repeat(65536 - overhead));
            let body = serde_json::to_vec(&value).unwrap();
            assert_eq!(body.len(), 65536);
            let encoded = encode_control_frame(&value).unwrap();
            assert_eq!(encoded.len(), 65540);
            assert_eq!(&encoded[..4], &65536u32.to_be_bytes());
            let t = Instant::now();
            let mut decoder = ControlDecoder::new();
            let frames = decoder.feed_admitted(&frame(&body).repeat(3), t).unwrap();
            assert_eq!(frames.len(), 3);
            for observed in frames {
                assert_eq!(observed.admitted_at(), t);
                assert_eq!(
                    decode_lifecycle_control_body(observed.body()).unwrap(),
                    value
                );
            }
            decoder.finish().unwrap();
            value["result"] = json!("x".repeat(65537 - overhead));
            assert!(encode_control_frame(&value).is_err());
        }

        #[test]
        fn runner_review_actual_health_client_uses_framing_and_validates_reply_versions() {
            for version in [
                Some(json!(1)),
                None,
                Some(json!(2)),
                Some(json!("1")),
                Some(json!(1.0)),
            ] {
                let mut fixture = Fixture::new();
                let artifacts = fixture.artifacts();
                let _held =
                    transfer_setting(&mut fixture, "runner_pid=?1,runner_start_identity=?2");
                let process = ProcessIdentity::current().unwrap();
                let run = fixture.run.clone();
                let listener = UnixListener::bind(&artifacts.paths.control_socket).unwrap();
                std::fs::set_permissions(
                    &artifacts.paths.control_socket,
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                listener.set_nonblocking(true).unwrap();
                let mut reply = json!({"result": {"identity":ControlIdentity::from_run(&run),"runner":process,"child":null,"output_seq":7}});
                if let Some(version) = &version {
                    reply["protocol_version"] = version.clone();
                }
                let expected_identity = ControlIdentity::from_run(&run);
                let valid = version == Some(json!(1));
                let deadline = Instant::now() + Duration::from_secs(2);
                std::thread::scope(|scope| {
                    let server = scope.spawn(move || {
                        let mut stream = loop {
                            match listener.accept() {
                                Ok((stream, _)) => break stream,
                                Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {}
                                Err(cause) => panic!("fixture socket accept failed: {cause}"),
                            }
                            assert!(Instant::now() < deadline, "fixture socket accept timed out");
                            std::thread::sleep(
                                Duration::from_millis(2)
                                    .min(deadline.saturating_duration_since(Instant::now())),
                            );
                        };
                        stream.set_nonblocking(true).unwrap();
                        let flags = nix::fcntl::OFlag::from_bits_retain(
                            nix::fcntl::fcntl(&stream, nix::fcntl::FcntlArg::F_GETFL).unwrap(),
                        );
                        assert!(flags.contains(nix::fcntl::OFlag::O_NONBLOCK));
                        let read_deadline = deadline.min(Instant::now() + Duration::from_secs(1));
                        let mut header = [0; 4];
                        read_fixture_bytes(&mut stream, &mut header, read_deadline);
                        let length = usize::try_from(u32::from_be_bytes(header)).unwrap();
                        assert!(
                            length > 0 && length <= 65536,
                            "actual client sent invalid length {length}"
                        );
                        let mut body = vec![0; length];
                        read_fixture_bytes(&mut stream, &mut body, read_deadline);
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(request["protocol_version"], 1);
                        assert_eq!(request["kind"], "health");
                        assert_eq!(
                            request["identity"],
                            serde_json::to_value(expected_identity).unwrap()
                        );
                        for byte in frame(&serde_json::to_vec(&reply).unwrap()) {
                            write_fixture_bytes(&mut stream, &[byte], deadline);
                        }
                    });
                    let observed = probe_health(&run, deadline);
                    assert_eq!(observed.is_ok(), valid, "{observed:?}");
                    if let Ok(health) = observed {
                        assert_eq!(health.output_seq, 7);
                    }
                    server.join().unwrap();
                });
            }
        }
    }

    use std::time::Duration;

    mod input_client_tests {
        use crate::broker::runtime::InputKind;
        use crate::runtime::control::InputControlOperation;
        use crate::runtime::control::InputControlReply;
        use crate::runtime::control::InputControlRequest;
        use crate::runtime::control::SubmissionStage;
        use crate::runtime::control::SubmissionStatus;
        use crate::runtime::control::SubmitInput;
        use crate::runtime::control::tests::*;
        use crate::runtime::test_support::Fixture;
        use serde_json::json;
        use std::net::Shutdown;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;

        type PeerResult<T> = std::result::Result<T, String>;
        use crate::runtime::control::test_support::independent_frame as frame;
        fn request(fixture: &Fixture) -> InputControlRequest {
            InputControlRequest {
                identity: ControlIdentity::from_run(&fixture.run),
                expires_at_mono_ns: 50_000_000_000,
                operation: InputControlOperation::SubmitPrompt(SubmitInput {
                    operation_key: "fixture:typed-client".into(),
                    input_kind: InputKind::Prompt,
                    message_id: None,
                    literal: "literal 世界 '$x'".into(),
                }),
            }
        }
        fn expected(fixture: &Fixture) -> Value {
            let run = &fixture.run;
            json!({"protocol_version":1,"kind":"submit_prompt","identity":{"database_identity_json":run.database_identity_json,"fleet_id":run.fleet_id,"member_id":run.member_id,"director_member_id":run.director_member_id,"run_id":run.run_id,"owner_epoch":run.owner_epoch,"launch_nonce":run.launch_nonce},"expires_at_mono_ns":50_000_000_000u64,"operation_key":"fixture:typed-client","input_kind":"prompt","message_id":null,"literal":"literal 世界 '$x'"})
        }
        fn reply(status: &str, stage: &str) -> Vec<u8> {
            let reason = if status == "accepted" {
                Value::Null
            } else {
                json!("fixture uncertainty")
            };
            let delivery = if status == "accepted" {
                json!({"status":"submitted","method":"profile","reason":null})
            } else {
                json!({"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"})
            };
            frame(
            &serde_json::to_vec(
                &json!({"protocol_version":1,"result":{"kind":"submission","submission":{
                    "status":status,"input_id":13,"stage":stage,"output_seq":null,"reason":reason,"delivery":delivery
                }}}),
            )
            .unwrap(),
        )
        }
        fn pair() -> (ControlClient, UnixStream) {
            let (stream, peer) = UnixStream::pair().unwrap();
            stream.set_nonblocking(true).unwrap();
            peer.set_nonblocking(true).unwrap();
            (
                ControlClient {
                    stream,
                    decoder: ControlDecoder::new(),
                },
                peer,
            )
        }
        fn active(stop: &AtomicBool, deadline: Instant) -> PeerResult<()> {
            if stop.load(Ordering::Acquire) {
                return Err("owned peer cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("owned peer deadline expired".into());
            }
            Ok(())
        }
        fn write_peer(
            stream: &mut UnixStream,
            mut bytes: &[u8],
            stop: &AtomicBool,
            deadline: Instant,
        ) -> PeerResult<()> {
            while !bytes.is_empty() {
                active(stop, deadline)?;
                match stream.write(bytes) {
                    Ok(0) => return Err("peer write made no progress".into()),
                    Ok(count) => bytes = &bytes[count..],
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) =>
                    {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(format!("peer write: {error}")),
                }
            }
            Ok(())
        }
        fn read_peer(
            stream: &mut UnixStream,
            mut bytes: &mut [u8],
            stop: &AtomicBool,
            deadline: Instant,
        ) -> PeerResult<()> {
            while !bytes.is_empty() {
                active(stop, deadline)?;
                match stream.read(bytes) {
                    Ok(0) => return Err("peer received truncated request".into()),
                    Ok(count) => bytes = &mut bytes[count..],
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) =>
                    {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(format!("peer read: {error}")),
                }
            }
            Ok(())
        }
        fn read_request(
            stream: &mut UnixStream,
            stop: &AtomicBool,
            deadline: Instant,
        ) -> PeerResult<Value> {
            let mut header = [0; 4];
            read_peer(stream, &mut header, stop, deadline)?;
            let size =
                usize::try_from(u32::from_be_bytes(header)).map_err(|error| error.to_string())?;
            if size > 65536 {
                return Err("peer received oversized request".into());
            }
            let mut body = vec![0; size];
            read_peer(stream, &mut body, stop, deadline)?;
            serde_json::from_slice(&body).map_err(|error| error.to_string())
        }
        struct Observed {
            request: Value,
            at: Instant,
            fragments_at: Option<(Instant, Instant)>,
            drained_after_return_at: Option<Instant>,
            extra_connections: usize,
            extra_request_bytes: usize,
        }
        fn describe(result: &Result<InputControlReply>) -> String {
            match result {
                Err(error) => format!("Err({error:?})"),
                Ok(InputControlReply::Submission(value)) => format!(
                    "Ok(Submission status={:?}, input_id={:?}, stage={:?}, output_seq={:?}, reason={:?})",
                    value.status, value.input_id, value.stage, value.output_seq, value.reason
                ),
                Ok(InputControlReply::Inspection(_)) => "Ok(Inspection)".into(),
                Ok(InputControlReply::PermissionResponse(_)) => "Ok(PermissionResponse)".into(),
                Ok(InputControlReply::Receipt(_)) => "Ok(Receipt)".into(),
            }
        }
        /// Runs `work` as a scoped peer while `exchange` runs here, then stops and joins the peer.
        fn exchange_with_peer(
            work: impl FnOnce(&AtomicBool, Instant) -> PeerResult<Observed> + Send,
            exchange: impl FnOnce() -> Result<InputControlReply>,
        ) -> (Result<InputControlReply>, Observed) {
            let stop = AtomicBool::new(false);
            let until = Instant::now() + Duration::from_secs(2);
            std::thread::scope(|scope| {
                let peer = scope.spawn(|| work(&stop, until));
                let result = exchange();
                stop.store(true, Ordering::Release);
                let observed =
                    peer.join()
                        .expect("owned peer panicked")
                        .unwrap_or_else(|cleanup| {
                            panic!(
                                "primary exchange result: {}; owned peer: {cleanup}",
                                describe(&result)
                            )
                        });
                (result, observed)
            })
        }
        fn accepted(result: Result<InputControlReply>) {
            match result.unwrap() {
                InputControlReply::Submission(submission) => {
                    assert_eq!(submission.status, SubmissionStatus::Accepted);
                    assert_eq!(submission.input_id, Some(13));
                    assert_eq!(submission.stage, SubmissionStage::Submitted);
                    assert_eq!(submission.output_seq, None);
                    assert_eq!(submission.reason, None);
                }
                _ => panic!("actual typed exchange returns submission"),
            }
        }
        fn preloaded(fixture: &Fixture, response: &[u8]) -> (Result<InputControlReply>, Value) {
            let (mut client, mut peer) = pair();
            let until = Instant::now() + Duration::from_secs(1);
            let stop = AtomicBool::new(false);
            write_peer(&mut peer, response, &stop, until).unwrap();
            let result = client.exchange_input(&request(fixture), until);
            let actual = read_request(&mut peer, &stop, until).unwrap();
            (result, actual)
        }
        #[test]
        fn typed_exchange_sends_an_independent_flat_request_with_the_original_expiry() {
            let fixture = Fixture::new();
            let (result, actual) = preloaded(&fixture, &reply("accepted", "submitted"));
            accepted(result);
            assert_eq!(actual, expected(&fixture));
        }
        #[test]
        fn exchange_validates_fragmented_duplicate_and_wrong_kind_replies() {
            let fixture = Fixture::new();
            let (mut client, mut stream) = pair();
            let response = reply("accepted", "submitted");
            let (result, observed) = exchange_with_peer(
                move |stop, until| {
                    let request = read_request(&mut stream, stop, until)?;
                    let at = Instant::now();
                    for chunk in response.chunks(3) {
                        write_peer(&mut stream, chunk, stop, until)?;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Ok(Observed {
                        request,
                        at,
                        fragments_at: None,
                        drained_after_return_at: None,
                        extra_connections: 0,
                        extra_request_bytes: 0,
                    })
                },
                || {
                    client
                        .exchange_input(&request(&fixture), Instant::now() + Duration::from_secs(1))
                },
            );
            accepted(result);
            assert_eq!(observed.request, expected(&fixture));
            let duplicate=frame(br#"{"protocol_version":1,"result":{"kind":"submission","submission":{"status":"accepted","input_id":13,"input_id":13,"stage":"submitted","output_seq":null,"reason":null,"delivery":{"status":"submitted","method":"profile","reason":null}}}}"#);
            let wrong=frame(br#"{"protocol_version":1,"result":{"kind":"receipt","receipt":{"status":"missing","input_id":null,"submission":null}}}"#);
            for response in [duplicate, wrong] {
                let (result, actual) = preloaded(&fixture, &response);
                assert!(result.is_err());
                assert_eq!(actual, expected(&fixture));
            }
        }
        #[test]
        fn exchange_rejects_extra_reply_trailing_partial_and_premature_eof() {
            let fixture = Fixture::new();
            let good = reply("accepted", "submitted");
            let mut extra = good.clone();
            extra.extend_from_slice(&good);
            let mut partial = good.clone();
            partial.extend_from_slice(&good[..2]);
            for response in [extra, partial] {
                let (result, actual) = preloaded(&fixture, &response);
                assert!(result.is_err());
                assert_eq!(actual, expected(&fixture));
            }
            let (mut client, mut peer) = pair();
            let until = Instant::now() + Duration::from_secs(1);
            let stop = AtomicBool::new(false);
            write_peer(&mut peer, &good[..good.len() - 1], &stop, until).unwrap();
            peer.shutdown(Shutdown::Write).unwrap();
            let result = client.exchange_input(&request(&fixture), until);
            let actual = read_request(&mut peer, &stop, until).unwrap();
            assert!(result.is_err());
            assert_eq!(actual, expected(&fixture));
        }
        #[test]
        fn expired_and_stalled_exchanges_keep_the_original_deadline() {
            let fixture = Fixture::new();
            let (mut client, mut peer) = pair();
            let expired = Instant::now()
                .checked_sub(Duration::from_millis(1))
                .unwrap();
            assert!(client.exchange_input(&request(&fixture), expired).is_err());
            let mut byte = [0];
            let observation_deadline = Instant::now() + Duration::from_millis(100);
            loop {
                assert!(Instant::now() < observation_deadline);
                match peer.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => panic!("an expired invocation sends zero request bytes"),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => panic!("observe expired exchange bytes: {error}"),
                }
            }
            let (mut client, mut stream) = pair();
            let budget = Duration::from_millis(800);
            let fragment_delay = Duration::from_millis(350);
            let return_tolerance = Duration::from_millis(100);
            let started = Instant::now();
            let deadline = started + budget;
            let mut returned = None;
            let (result, observed) = exchange_with_peer(
                move |stop, until| {
                    let request = read_request(&mut stream, stop, until)?;
                    let at = Instant::now();
                    write_peer(&mut stream, &[0, 0], stop, until)?;
                    let first_fragment_at = Instant::now();
                    let next_fragment_at = first_fragment_at + fragment_delay;
                    while Instant::now() < next_fragment_at {
                        active(stop, until)?;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    write_peer(&mut stream, &[0, 20], stop, until)?;
                    let later_fragment_at = Instant::now();
                    while !stop.load(Ordering::Acquire) && Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Ok(Observed {
                        request,
                        at,
                        fragments_at: Some((first_fragment_at, later_fragment_at)),
                        drained_after_return_at: None,
                        extra_connections: 0,
                        extra_request_bytes: 0,
                    })
                },
                || {
                    let result = client.exchange_input(&request(&fixture), deadline);
                    returned = Some(Instant::now());
                    result
                },
            );
            let returned = returned.expect("the exchange records its return");
            let (first_fragment_at, later_fragment_at) = observed.fragments_at.unwrap();
            assert!(result.is_err());
            assert_eq!(observed.request, expected(&fixture));
            assert!(
                observed.at < deadline,
                "peer observed the actual request before the tested expiry"
            );
            assert!(observed.at <= first_fragment_at);
            assert!(first_fragment_at < later_fragment_at);
            assert!(later_fragment_at < deadline);
            assert!(later_fragment_at.duration_since(first_fragment_at) >= fragment_delay);
            assert!(later_fragment_at.duration_since(started) >= fragment_delay);
            assert!(return_tolerance < later_fragment_at.duration_since(first_fragment_at));
            assert!(returned >= deadline);
            assert!(
                returned < deadline + return_tolerance,
                "actual exchange shares its original budget"
            );
            assert!(returned < later_fragment_at + budget);
        }
        #[test]
        fn disconnect_after_the_request_has_no_reconnect_or_second_request() {
            let fixture = Fixture::new();
            let directory = crate::runtime::test_support::short_tempdir();
            let path = directory.path().join("control.sock");
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let exchange_returned = Arc::new(AtomicBool::new(false));
            let returned_signal = exchange_returned.clone();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut returned_at = None;
            let (result, observed) = exchange_with_peer(
                move |stop, until| {
                    let (mut stream, _) = loop {
                        active(stop, until)?;
                        match listener.accept() {
                            Ok(value) => break value,
                            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                                std::thread::yield_now()
                            }
                            Err(error) => return Err(error.to_string()),
                        }
                    };
                    stream
                        .set_nonblocking(true)
                        .map_err(|error| error.to_string())?;
                    let request = read_request(&mut stream, stop, until)?;
                    let at = Instant::now();
                    let mut trailing = [0; 4096];
                    let mut extra_request_bytes = 0;
                    stream
                        .shutdown(Shutdown::Write)
                        .map_err(|error| error.to_string())?;
                    let mut extra_connections = 0;
                    let drained_after_return_at = loop {
                        if Instant::now() >= until {
                            return Err("disconnect observer expired before final drain".into());
                        }
                        let after_return = exchange_returned.load(Ordering::Acquire);
                        loop {
                            if Instant::now() >= until {
                                return Err("disconnect request drain exceeded its bound".into());
                            }
                            match stream.read(&mut trailing) {
                                Ok(0) => break,
                                Ok(count) => extra_request_bytes += count,
                                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                                Err(error) => return Err(error.to_string()),
                            }
                        }
                        loop {
                            if Instant::now() >= until {
                                return Err("disconnect listener drain exceeded its bound".into());
                            }
                            match listener.accept() {
                                Ok((extra, _)) => {
                                    extra_connections += 1;
                                    drop(extra);
                                }
                                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                                Err(error) => return Err(error.to_string()),
                            }
                        }
                        if after_return {
                            break Instant::now();
                        }
                        if stop.load(Ordering::Acquire)
                            && !exchange_returned.load(Ordering::Acquire)
                        {
                            return Err(
                                "disconnect observer cancelled before actual exchange return"
                                    .into(),
                            );
                        }
                        std::thread::yield_now();
                    };
                    Ok(Observed {
                        request,
                        at,
                        fragments_at: None,
                        drained_after_return_at: Some(drained_after_return_at),
                        extra_connections,
                        extra_request_bytes,
                    })
                },
                || {
                    let mut client = ControlClient::connect(&path, deadline).unwrap();
                    let result = client.exchange_input(&request(&fixture), deadline);
                    returned_at = Some(Instant::now());
                    returned_signal.store(true, Ordering::Release);
                    result
                },
            );
            assert!(result.is_err());
            assert_eq!(observed.request, expected(&fixture));
            assert_eq!(observed.extra_connections, 0);
            assert_eq!(observed.extra_request_bytes, 0);
            assert!(
                observed.drained_after_return_at.unwrap()
                    >= returned_at.expect("the exchange records its return")
            );
        }

        #[test]
        fn textual_error_remains_error_and_structured_uncertainty_remains_result() {
            let fixture = Fixture::new();
            let error = frame(br#"{"protocol_version":1,"error":"fixture failure"}"#);
            let (result, actual) = preloaded(&fixture, &error);
            assert_eq!(actual, expected(&fixture));
            match result {
                Err(CafleetError::App(message)) => {
                    assert_eq!(message, "runner control: fixture failure")
                }
                Err(other) => panic!("ordinary application error expected: {other}"),
                Ok(_) => panic!("textual server failure is an error"),
            }
            for stage in ["partial", "unknown"] {
                let (result, actual) = preloaded(&fixture, &reply("uncertain", stage));
                assert_eq!(actual, expected(&fixture));
                match result.unwrap() {
                    InputControlReply::Submission(submission) => {
                        assert_eq!(submission.status, SubmissionStatus::Uncertain);
                        assert_eq!(submission.stage.as_str(), stage);
                        assert_eq!(submission.input_id, Some(13));
                    }
                    _ => panic!("structured uncertainty remains a submission result"),
                }
            }
        }
    }

    mod launch_readiness_tests {
        use std::io::ErrorKind;

        use std::time::Duration;
        use std::time::Instant;

        use crate::runtime::control::*;

        use crate::runtime::control::test_support::Endpoint;
        use crate::runtime::control::test_support::StartupInterval;
        use crate::runtime::control::test_support::bound_socket;
        use crate::runtime::control::test_support::transfer_setting;

        #[test]
        fn runner_readiness_post_start_health_preserves_strict_endpoint_failures() {
            for interval in [StartupInterval::Bound, StartupInterval::BeforePrivateMode] {
                let mut endpoint = Endpoint::new();
                let _held = transfer_setting(
                    &mut endpoint.fixture,
                    "status='running',child_pid=?1,child_start_identity=?2",
                );
                let listener = match interval {
                    StartupInterval::Bound => bound_socket(endpoint.path(), 0o600, false),
                    StartupInterval::BeforePrivateMode => {
                        bound_socket(endpoint.path(), 0o644, true)
                    }
                };
                let started = Instant::now();
                let result = probe_health(&endpoint.fixture.run, started + Duration::from_secs(1));
                assert!(result.is_err(), "{result:?}");
                assert!(started.elapsed() < Duration::from_millis(500));
                if matches!(interval, StartupInterval::BeforePrivateMode) {
                    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
                }
            }
        }
    }

    use std::time::Instant;

    use crate::runtime::control::test_support::*;

    use crate::runtime::test_support::Fixture;
    use serde_json::json;

    #[test]
    fn runner_health_real_silent_endpoint_is_bounded_and_probe_is_read_only() {
        let mut fixture = Fixture::new();
        let (artifacts, _held) = activate(&mut fixture);
        let _server = SilentServer::start(&artifacts.paths.control_socket);
        let before = std::fs::read(fixture.conn.path().unwrap()).unwrap();
        let start = Instant::now();
        assert!(probe_health(&fixture.run, start + Duration::from_millis(80)).is_err());
        assert!(start.elapsed() < Duration::from_millis(750));
        assert_eq!(std::fs::read(fixture.conn.path().unwrap()).unwrap(), before);
    }

    mod observation_contract {
        use crate::runtime::control::tests::*;

        fn wire(run: &RunRecord) -> serde_json::Value {
            json!({"identity":ControlIdentity::from_run(run),"runner":{"pid":run.runner_pid.unwrap(),"start_identity":run.runner_start_identity.as_ref().unwrap()},"child":{"pid":run.child_pid.unwrap(),"start_identity":run.child_start_identity.as_ref().unwrap()},"output_seq":17,"observation":{"visibility":"unknown","eligibility":{"state":"hold","reason":"unknown"},"pending_count":0,"pending_evidence":false,"gap_count":0,"degraded":false,"diagnostic":null}})
        }

        #[test]
        fn health_decode_without_observation_validates_identity_without_granting_composer_proof() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            let mut value = wire(&fixture.run);
            value.as_object_mut().unwrap().remove("observation");
            let decoded = HealthSnapshot::decode(value, &fixture.run).unwrap();
            assert!(decoded.observation.is_none());
        }

        #[test]
        fn health_decode_accepts_bounded_hold_and_checked_empty_composer_states() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for visibility in ["visible", "obscured", "not_visible", "unknown"] {
                let mut value = wire(&fixture.run);
                value["observation"]["visibility"] = json!(visibility);
                value["observation"]["pending_count"] = json!(130);
                value["observation"]["pending_evidence"] = json!(true);
                value["observation"]["degraded"] = json!(true);
                value["observation"]["diagnostic"] = json!("sink_unavailable");
                let decoded = HealthSnapshot::decode(value, &fixture.run).unwrap();
                decoded.observation.unwrap().validate().unwrap();
            }
            let mut value = wire(&fixture.run);
            value["observation"]["visibility"] = json!("not_visible");
            value["observation"]["eligibility"] = json!({"state":"composer_empty"});
            assert!(HealthSnapshot::decode(value, &fixture.run).is_ok());
        }

        #[test]
        fn health_decode_rejects_unknown_wire_fields_enums_unbounded_and_private_diagnostics() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for field in [
                "unknown_field",
                "visibility",
                "state",
                "reason",
                "pending",
                "negative",
                "float",
                "raw_diagnostic",
                "long_diagnostic",
                "unicode_diagnostic",
            ] {
                let mut value = wire(&fixture.run);
                let status = &mut value["observation"];
                match field {
                    "unknown_field" => status["screen"] = json!("private screen"),
                    "visibility" => status["visibility"] = json!("invented"),
                    "state" => status["eligibility"]["state"] = json!("allow"),
                    "reason" => status["eligibility"]["reason"] = json!("invented"),
                    "pending" => status["pending_count"] = json!(131),
                    "negative" => status["gap_count"] = json!(-1),
                    "float" => status["pending_count"] = json!(0.5),
                    "raw_diagnostic" => {
                        status["diagnostic"] = json!("private exception configuration canary")
                    }
                    "long_diagnostic" => status["diagnostic"] = json!("x".repeat(129)),
                    _ => status["diagnostic"] = json!("世界"),
                }
                assert!(
                    HealthSnapshot::decode(value, &fixture.run).is_err(),
                    "{field}"
                );
            }
        }

        #[test]
        fn health_decode_requires_hold_for_visibility_pressure_and_degradation() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for field in [
                "visible",
                "obscured",
                "unknown",
                "pending_count",
                "pending_evidence",
                "degraded",
            ] {
                let mut value = wire(&fixture.run);
                value["observation"]["visibility"] = json!("not_visible");
                value["observation"]["eligibility"] = json!({"state":"composer_empty"});
                match field {
                    "pending_count" => value["observation"][field] = json!(1),
                    "pending_evidence" | "degraded" => value["observation"][field] = json!(true),
                    _ => value["observation"]["visibility"] = json!(field),
                }
                assert!(
                    HealthSnapshot::decode(value, &fixture.run).is_err(),
                    "{field}"
                );
            }
        }

        #[test]
        fn health_decode_preserves_process_generation_and_control_identity_checks() {
            let mut fixture = Fixture::new();
            let (_artifacts, _lock) = activate(&mut fixture);
            for field in ["epoch", "nonce", "runner", "child"] {
                let mut value = wire(&fixture.run);
                match field {
                    "epoch" => {
                        value["identity"]["owner_epoch"] = json!(fixture.run.owner_epoch + 1)
                    }
                    "nonce" => value["identity"]["launch_nonce"] = json!("b".repeat(64)),
                    "runner" => value["runner"]["start_identity"] = json!("stale generation"),
                    _ => value["child"]["start_identity"] = json!("stale generation"),
                }
                assert!(
                    HealthSnapshot::decode(value, &fixture.run).is_err(),
                    "{field}"
                );
            }
        }
    }

    mod recovery_entry {
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        use std::path::PathBuf;

        use crate::runtime::control::tests::*;

        const SPOOL_CANARY: &[u8] = b"private-recovery-diagnosis-canary:not-a-complete-valid-frame";

        fn registry_snapshot(conn: &Connection) -> Vec<crate::db::test_support::TableSnapshot> {
            crate::db::test_support::table_snapshot(conn, |_| true)
        }

        #[derive(Debug, PartialEq)]
        struct FileSnapshot {
            path: PathBuf,
            device: u64,
            inode: u64,
            mode: u32,
            bytes: Vec<u8>,
        }

        #[derive(Debug, PartialEq)]
        struct OwnedArtifacts {
            directories: Vec<(PathBuf, u64, u64, u32)>,
            files: Vec<FileSnapshot>,
        }

        fn owned_snapshot(
            fixture: &Fixture,
            artifacts: &RunArtifacts,
            held: &RunLock,
        ) -> OwnedArtifacts {
            held.validate(&fixture.conn, &fixture.run.owner()).unwrap();
            let directories = [&artifacts.paths.namespace_dir, &artifacts.paths.run_dir]
                .into_iter()
                .map(|path| {
                    let metadata = std::fs::symlink_metadata(path).unwrap();
                    assert!(metadata.is_dir());
                    (
                        path.clone(),
                        metadata.dev(),
                        metadata.ino(),
                        metadata.mode(),
                    )
                })
                .collect();
            let files = [
                &artifacts.paths.namespace_manifest,
                &artifacts.paths.owner_lock,
                &artifacts.paths.outbox,
                &artifacts.paths.descriptor,
            ]
            .into_iter()
            .map(|path| {
                let metadata = std::fs::symlink_metadata(path).unwrap();
                assert!(metadata.is_file());
                FileSnapshot {
                    path: path.clone(),
                    device: metadata.dev(),
                    inode: metadata.ino(),
                    mode: metadata.mode(),
                    bytes: std::fs::read(path).unwrap(),
                }
            })
            .collect();
            OwnedArtifacts { directories, files }
        }

        fn seed_owned_spool(fixture: &Fixture, artifacts: &RunArtifacts, held: &RunLock) {
            held.validate(&fixture.conn, &fixture.run.owner()).unwrap();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&artifacts.paths.outbox)
                .unwrap();
            file.write_all(SPOOL_CANARY).unwrap();
            file.sync_all().unwrap();
            artifacts
                .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
                .unwrap();
        }

        fn diagnose_without_content_effects(fixture: &Fixture, expected_lock: &str) {
            let started = Instant::now();
            let diagnosis = crate::runtime::control::diagnose_runtime_at(
                fixture.conn.path().unwrap(),
                fixture.run.run_id,
                started + Duration::from_millis(80),
            )
            .unwrap();
            assert!(started.elapsed() < Duration::from_millis(750));
            assert_eq!(diagnosis.state, MemberState::InterruptedLaunch);
            assert_eq!(
                diagnosis.diagnostics[..2],
                ["launch descriptor left over", expected_lock]
            );
            let text = diagnosis.diagnostics.join("\n");
            assert!(!text.contains(std::str::from_utf8(SPOOL_CANARY).unwrap()));
        }

        #[test]
        fn runner_recovery_entry_doctor_preserves_held_owner_corrupt_spool_rows_and_identities() {
            let mut fixture = Fixture::new();
            let (artifacts, held) = activate(&mut fixture);
            seed_owned_spool(&fixture, &artifacts, &held);
            let before_rows = registry_snapshot(&fixture.conn);
            let before_artifacts = owned_snapshot(&fixture, &artifacts, &held);
            diagnose_without_content_effects(&fixture, "owner lock is held");
            assert_eq!(registry_snapshot(&fixture.conn), before_rows);
            assert_eq!(
                owned_snapshot(&fixture, &artifacts, &held),
                before_artifacts
            );
        }

        #[test]
        fn runner_recovery_entry_doctor_preserves_available_owner_corrupt_spool_without_takeover() {
            let mut fixture = Fixture::new();
            let (artifacts, held) = activate(&mut fixture);
            seed_owned_spool(&fixture, &artifacts, &held);
            let before_rows = registry_snapshot(&fixture.conn);
            let before_artifacts = owned_snapshot(&fixture, &artifacts, &held);
            drop(held);
            diagnose_without_content_effects(&fixture, "owner lock is free");
            let held = RunLock::acquire_released(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .expect("diagnosis releases its probe and preserves the original owner identity");
            assert_eq!(registry_snapshot(&fixture.conn), before_rows);
            assert_eq!(
                owned_snapshot(&fixture, &artifacts, &held),
                before_artifacts
            );
        }
    }

    use serde_json::Value;

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
                    "expected_run_id":run.run_id,"expected_owner_epoch":run.owner_epoch,"request_token":"a".repeat(64),"decision":"allow","choice":null,
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
                "backend":"codex","native_version":run.native_version,"rule_id":"fixture-wire-evidence","screen_seq":2,
                "operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false
            })).unwrap();
            json!({"event_id":11,"message_id":14,"member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,
                "run_id":run.run_id,"owner_epoch":run.owner_epoch,"interaction_id":12,"episode_seq":1,
                "classification":if supported {json!("permission_request")} else {Value::Null},"visibility":if supported {"visible"} else {"unknown"},
                "backend":"codex","native_version":run.native_version,
                "answer_rule_id":if supported {json!("fixture-rule")} else {Value::Null},
                "answer_capability":if supported {"tested"} else {"unknown"},"capability_reason":if supported {Value::Null} else {json!("fixture unsupported")},
                "supported_decisions":if supported {json!(["allow","deny"])} else {json!([])},"supported_choices":[],
                "request_token":if supported {json!("a".repeat(64))} else {Value::Null},
                "output_seq":2,"evidence":evidence,
                "missing_evidence":["operation","arguments","cwd","environment","targets","effects","choice_scope"]})
        }
        fn question_inspection(&self, supported: bool) -> Value {
            let mut value = self.inspection(supported);
            value["classification"] = json!("question");
            value["visibility"] = json!("visible");
            value["supported_decisions"] = json!([]);
            value["missing_evidence"] = json!([]);
            if supported {
                value["answer_rule_id"] = json!("fixture-question-decline");
                value["supported_choices"] = json!([
                    {"choice_id":"decline","label":"No, keep manual mode","scope":"persistent"},
                    {"choice_id":"later","label":"Ask later","scope":"session"}
                ]);
            } else {
                value["capability_reason"] = json!("question has no tested answer");
            }
            value
        }
        fn choice_request(&self, choice: &str) -> Value {
            let mut value = self.request("permission_respond", None);
            value["decision"] = Value::Null;
            value["choice"] = json!(choice);
            value
        }
        fn response(&self) -> Value {
            let run = &self.fixture.run;
            json!({"status":"accepted","input_id":13,"member_id":run.member_id,"run_id":run.run_id,"owner_epoch":run.owner_epoch,
                "interaction_id":12,"event_id":11,"decision":"allow","choice":null,"decision_source":"director-auto","stage":"submitted","output_seq":2,"reason":null})
        }
        fn choice_response(&self, choice: &str) -> Value {
            let mut value = self.response();
            value["decision"] = Value::Null;
            value["choice"] = json!(choice);
            value
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
            _ => {
                json!({"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"})
            }
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
                    assert_eq!(input.key, AnswerKey::Decision(PermissionDecision::Allow));
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
        let identity = case.request("submit_prompt", None)["identity"].clone();
        for (kind, field) in [
            ("health", None),
            ("prepare", Some(("remaining_ms", json!(5)))),
            ("start", Some(("pane_id", json!("%1")))),
            ("shutdown", None),
        ] {
            let mut value = json!({"protocol_version":1,"kind":kind,"identity":identity});
            if let Some((name, field)) = field {
                value[name] = field;
            }
            match decode_service_request(&bytes(&value)).unwrap() {
                ServiceControlRequest::Lifecycle(actual) => {
                    assert_eq!(serde_json::to_value(&actual).unwrap(), value, "{kind}")
                }
                ServiceControlRequest::Input(_) => panic!("{kind} routes to the lifecycle"),
            }
            value["fixture_unknown"] = json!(true);
            assert!(decode_service_request(&bytes(&value)).is_err(), "{kind}");
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
    fn exact_reencoding_refuses_defaulted_flattened_float_and_duplicated_fields() {
        let case = Case::new();
        let prompt = case.request("submit_prompt", None);
        let respond = case.request("permission_respond", None);
        let retire = case.request("receipt", Some("retire"));

        let mut float_expiry = prompt.clone();
        float_expiry["expires_at_mono_ns"] = json!(50_000_000_000.0);
        rejection(&prompt, &bytes(&float_expiry));
        let mut absent_choice = respond.clone();
        absent_choice.as_object_mut().unwrap().remove("choice");
        rejection(&respond, &bytes(&absent_choice));
        rejection(&respond, &duplicate(&respond, "decision"));

        let accepted = reply(
            "submission",
            "submission",
            submission("accepted", "submitted", Some(13)),
        );
        let mut defaulted_delivery = accepted.clone();
        defaulted_delivery["result"]["submission"]
            .as_object_mut()
            .unwrap()
            .remove("delivery");
        reject_reply(&accepted, &bytes(&defaulted_delivery), &prompt);
        let mut float_input = accepted.clone();
        float_input["result"]["submission"]["input_id"] = json!(13.0);
        reject_reply(&accepted, &bytes(&float_input), &prompt);

        let response = reply("permission_response", "response", case.response());
        let mut flattened_delivery = response.clone();
        flattened_delivery["result"]["response"]["delivery"] =
            submission("accepted", "submitted", Some(13))["delivery"].clone();
        reject_reply(&response, &bytes(&flattened_delivery), &respond);
        let mut absent_response_choice = response.clone();
        absent_response_choice["result"]["response"]
            .as_object_mut()
            .unwrap()
            .remove("choice");
        reject_reply(&response, &bytes(&absent_response_choice), &respond);

        let retired = reply(
            "receipt",
            "receipt",
            json!({"status":"retired","input_id":13,"submission":null}),
        );
        let mut absent_unit = retired.clone();
        absent_unit["result"]["receipt"]
            .as_object_mut()
            .unwrap()
            .remove("submission");
        reject_reply(&retired, &bytes(&absent_unit), &retire);
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
            let good = reply("inspection", "request", case.inspection(supported));
            let actual = decoded_reply(&good, &request);
            assert!(matches!(actual, InputControlReply::Inspection(_)));
            assert_frame(&encode_input_reply(Ok(&actual)).unwrap(), &good);
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
        assert_eq!(actual.key, AnswerKey::Decision(PermissionDecision::Allow));
        assert_eq!(actual.decision_source, DecisionSource::DirectorAuto);
        let mut nested = good.clone();
        nested["result"]["response"]["submission"] = submission("accepted", "submitted", Some(13));
        reject_reply(&good, &bytes(&nested), &request);
    }

    #[test]
    fn request_permission_respond_carries_exactly_one_of_a_decision_and_a_choice() {
        let case = Case::new();
        let decision = case.request("permission_respond", None);
        match decoded(&decision).operation {
            InputControlOperation::PermissionRespond(input) => {
                assert_eq!(input.key, AnswerKey::Decision(PermissionDecision::Allow))
            }
            _ => panic!("permission response operation"),
        }
        for choice in [
            "decline",
            "dont-show-again",
            "not-now",
            "a",
            &"a".repeat(64),
        ] {
            let good = case.choice_request(choice);
            match decoded(&good).operation {
                InputControlOperation::PermissionRespond(input) => {
                    assert_eq!(input.key, AnswerKey::Choice(choice.into()), "{choice}")
                }
                _ => panic!("permission response operation"),
            }
            assert_frame(&encode_input_request(&decoded(&good)).unwrap(), &good);
        }
        let good = case.choice_request("decline");
        let mut both = good.clone();
        both["decision"] = json!("deny");
        rejection(&good, &bytes(&both));
        let mut neither = good.clone();
        neither["choice"] = Value::Null;
        rejection(&good, &bytes(&neither));
        for invalid in [
            json!("Decline"),
            json!("ask_later"),
            json!("-decline"),
            json!("decline-"),
            json!("dont--show"),
            json!("dont show"),
            json!(""),
            json!("a".repeat(65)),
            json!(1),
            json!(["decline"]),
        ] {
            let mut bad = good.clone();
            bad["choice"] = invalid;
            rejection(&good, &bytes(&bad));
        }
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("choice");
        rejection(&good, &bytes(&missing));
        rejection(&good, &duplicate(&good, "choice"));
    }

    #[test]
    fn reply_question_inspection_carries_its_supported_choices_and_no_decisions() {
        let case = Case::new();
        let request = case.request("permission_inspect", None);
        for supported in [true, false] {
            let payload = case.question_inspection(supported);
            let good = reply("inspection", "request", payload.clone());
            let actual = match decoded_reply(&good, &request) {
                InputControlReply::Inspection(actual) => actual,
                _ => panic!("inspection result has exact kind"),
            };
            assert_eq!(
                serde_json::to_value(actual.classification).unwrap(),
                json!("question")
            );
            assert!(actual.supported_decisions.is_empty());
            assert_eq!(
                serde_json::to_value(&actual.supported_choices).unwrap(),
                payload["supported_choices"]
            );
            assert_eq!(
                actual.capability_reason.as_deref(),
                payload["capability_reason"].as_str()
            );
            assert_eq!(
                actual.request_token.as_deref(),
                payload["request_token"].as_str()
            );
            assert!(actual.missing_evidence.is_empty());
            let encoded = encode_input_reply(Ok(&InputControlReply::Inspection(actual))).unwrap();
            assert_frame(&encoded, &good);
            assert_eq!(
                serde_json::from_slice::<Value>(body(&encoded)).unwrap()["result"]["request"]
                    ["supported_choices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|choice| choice.as_object().unwrap().keys().cloned().collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                if supported {
                    vec![vec!["choice_id", "label", "scope"]; 2]
                } else {
                    Vec::<Vec<&str>>::new()
                }
            );
        }
        let good = reply("inspection", "request", case.question_inspection(true));
        for (case_name, choices) in [
            (
                "unknown scope",
                json!([{"choice_id":"decline","label":"Keep","scope":"forever"}]),
            ),
            (
                "missing label",
                json!([{"choice_id":"decline","scope":"persistent"}]),
            ),
            (
                "extra field",
                json!([{"choice_id":"decline","label":"Keep","scope":"persistent","stage":"0d"}]),
            ),
            (
                "choice id outside its shape",
                json!([{"choice_id":"Decline","label":"Keep","scope":"persistent"}]),
            ),
            (
                "duplicate choice",
                json!([
                    {"choice_id":"decline","label":"Keep","scope":"persistent"},
                    {"choice_id":"decline","label":"Keep","scope":"persistent"}
                ]),
            ),
            ("non-array", json!({"choice_id":"decline"})),
        ] {
            let mut bad = good.clone();
            bad["result"]["request"]["supported_choices"] = choices;
            assert!(
                decode_input_reply(&bytes(&bad), &decoded(&request)).is_err(),
                "{case_name}"
            );
        }
        let mut both = good.clone();
        both["result"]["request"]["supported_decisions"] = json!(["allow"]);
        reject_reply(&good, &bytes(&both), &request);
    }

    #[test]
    fn reply_choice_response_names_the_requested_choice_after_a_null_decision() {
        let case = Case::new();
        let request = case.choice_request("decline");
        let good = reply(
            "permission_response",
            "response",
            case.choice_response("decline"),
        );
        let actual = match decoded_reply(&good, &request) {
            InputControlReply::PermissionResponse(actual) => actual,
            _ => panic!("response result has exact kind"),
        };
        assert_eq!(actual.key, AnswerKey::Choice("decline".into()));
        let encoded =
            encode_input_reply(Ok(&InputControlReply::PermissionResponse(actual))).unwrap();
        assert_frame(&encoded, &good);
        for (field, value) in [
            ("choice", json!("later")),
            ("choice", Value::Null),
            ("decision", json!("allow")),
        ] {
            let mut bad = good.clone();
            bad["result"]["response"][field] = value;
            reject_reply(&good, &bytes(&bad), &request);
        }
        let decision_request = case.request("permission_respond", None);
        let decision_good = reply("permission_response", "response", case.response());
        let mut chose = decision_good.clone();
        chose["result"]["response"]["choice"] = json!("decline");
        reject_reply(&decision_good, &bytes(&chose), &decision_request);
    }

    #[test]
    fn reply_rejected_answer_kind_mismatches_decode_with_their_reasons() {
        let case = Case::new();
        for (request, mut response, reason) in [
            (
                case.choice_request("decline"),
                case.choice_response("decline"),
                "choice answers apply only to question interactions",
            ),
            (
                case.request("permission_respond", None),
                case.response(),
                "decision answers apply only to permission requests",
            ),
            (
                case.choice_request("decline"),
                case.choice_response("decline"),
                "choice decline has no tested answer for this question",
            ),
        ] {
            response["status"] = json!("rejected");
            response["input_id"] = Value::Null;
            response["stage"] = json!("zero_bytes");
            response["output_seq"] = Value::Null;
            response["reason"] = json!(reason);
            let good = reply("permission_response", "response", response);
            let actual = match decoded_reply(&good, &request) {
                InputControlReply::PermissionResponse(actual) => actual,
                _ => panic!("response result has exact kind"),
            };
            assert_eq!(actual.submission.status, SubmissionStatus::Rejected);
            assert_eq!(actual.submission.stage, SubmissionStage::ZeroBytes);
            assert_eq!(actual.submission.reason.as_deref(), Some(reason));
        }
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
                assert_eq!(result_keys(&encoded, "request"),"event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq,evidence,missing_evidence".split(',').collect::<Vec<_>>());
            }
            if request["kind"] == "permission_respond" {
                assert_eq!(result_keys(&encoded, "response"),"status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,decision_source,stage,output_seq,reason".split(',').collect::<Vec<_>>());
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
            delivery: Some(crate::broker::runtime::DeliveryProjection {
                status: crate::broker::runtime::DeliveryStatus::Uncertain,
                method: Some(crate::broker::runtime::DeliveryMethod::Fallback),
                reason: Some("submission_plan_unavailable".into()),
            }),
        });
        assert!(encode_input_reply(Ok(&actual)).is_err());
    }

    mod identity {
        use crate::runtime::control::tests::*;

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
            for (field, values) in [
                ("canonical_path", bad_paths()),
                ("instance_id", bad_hex(32)),
            ] {
                for (index, value) in values.into_iter().enumerate() {
                    let mut bad = good.clone();
                    bad["identity"]["database_identity_json"][field] = value;
                    reject_decode(&good, &bad, &format!("{field} {index}"));
                }
            }
        }

        #[test]
        fn identity_decode_requires_an_unsigned_64_bit_device() {
            let case = Case::new();
            let good = control(&case);
            for field in ["device", "inode"] {
                for (index, value) in bad_numbers().into_iter().enumerate() {
                    let mut bad = good.clone();
                    bad["identity"]["database_identity_json"][field] = value;
                    reject_decode(&good, &bad, &format!("{field} {index}"));
                }
            }
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

    mod runtime_diagnosis {
        use crate::runtime::control::*;

        fn observed(
            descriptor_leftover: bool,
            lock: OwnerLockProbe,
            health: HealthProbe,
        ) -> RuntimeDiagnosis {
            RuntimeProbes::Observed {
                descriptor_leftover,
                lock,
                health,
            }
            .classify()
        }

        fn refused() -> HealthProbe {
            HealthProbe::Failed("member runtime: ECONNREFUSED: Connection refused".into())
        }

        fn lock_failure() -> OwnerLockProbe {
            OwnerLockProbe::Failed("owner lock identity changed".into())
        }

        #[test]
        fn a_leftover_descriptor_is_an_interrupted_launch_before_any_health_outcome() {
            const REFUSED: &str = "runtime did not answer the health probe: member runtime: ECONNREFUSED: Connection refused";
            let cases: Vec<(RuntimeDiagnosis, MemberState, &[&str])> = vec![
                (
                    RuntimeProbes::InvalidPaths("namespace manifest is missing".into()).classify(),
                    MemberState::InterruptedLaunch,
                    &["runtime files are invalid: namespace manifest is missing"],
                ),
                (
                    observed(true, OwnerLockProbe::Held, HealthProbe::Answered),
                    MemberState::InterruptedLaunch,
                    &[
                        "launch descriptor left over",
                        "owner lock is held",
                        "runtime answered the health probe",
                    ],
                ),
                (
                    observed(true, OwnerLockProbe::Free, HealthProbe::Skipped),
                    MemberState::InterruptedLaunch,
                    &["launch descriptor left over", "owner lock is free"],
                ),
                (
                    observed(false, OwnerLockProbe::Held, HealthProbe::Answered),
                    MemberState::Running,
                    &["owner lock is held", "runtime answered the health probe"],
                ),
                (
                    observed(false, OwnerLockProbe::Free, HealthProbe::Answered),
                    MemberState::Running,
                    &["owner lock is free", "runtime answered the health probe"],
                ),
                (
                    observed(false, OwnerLockProbe::Held, refused()),
                    MemberState::NotResponding,
                    &["owner lock is held", REFUSED],
                ),
                (
                    observed(false, OwnerLockProbe::Free, refused()),
                    MemberState::LostRuntime,
                    &["owner lock is free", REFUSED],
                ),
                (
                    observed(false, OwnerLockProbe::Held, HealthProbe::Skipped),
                    MemberState::NotChecked,
                    &["owner lock is held"],
                ),
                (
                    observed(false, OwnerLockProbe::Free, HealthProbe::Skipped),
                    MemberState::NotChecked,
                    &["owner lock is free"],
                ),
                (
                    observed(false, lock_failure(), HealthProbe::Answered),
                    MemberState::CheckFailed,
                    &["owner lock identity changed"],
                ),
                (
                    observed(true, lock_failure(), refused()),
                    MemberState::CheckFailed,
                    &["owner lock identity changed"],
                ),
            ];
            for (diagnosis, state, phrases) in cases {
                assert_eq!(diagnosis.state, state, "{:?}", diagnosis.diagnostics);
                assert_eq!(diagnosis.diagnostics, phrases);
            }
        }
    }
}
