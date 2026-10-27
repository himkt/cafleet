use super::*;
use crate::runtime::runner::codec::{decode_legacy_control_body, frame_control_body};
use serde::de::DeserializeOwned;
use serde_json::{Map, json};

mod replies;
mod strict_json;

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

fn version(value: &Value) -> Result<()> {
    ensure(
        value.as_u64() == Some(u64::from(CONTROL_PROTOCOL_VERSION)),
        "control frame requires integer protocol_version 1",
    )
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
    version(object.value("protocol_version"))?;
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
            validate_request_token(&input.request_token)?;
            validate_operation_key(&input.operation_key)?;
            validate_permission_reference(&input.mode_evidence_ref)?;
            validate_permission_reference(&input.decision_evidence_ref)?;
            validate_permission_attribution(input.decision_source, input.director_mode)?;
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
        protocol_version: CONTROL_PROTOCOL_VERSION,
        identity,
        expires_at_mono_ns,
        operation,
    })
}

pub(in crate::runtime::runner) fn decode_input_request(body: &[u8]) -> Result<InputControlRequest> {
    decode_request_value(&strict_json::decode(body)?)
}

pub(in crate::runtime::runner) enum ServiceControlRequest {
    Legacy(Value),
    Input(Box<InputControlRequest>),
}
pub(in crate::runtime::runner) fn decode_service_request(
    body: &[u8],
) -> Result<ServiceControlRequest> {
    if strict_json::routes_input(body)? {
        decode_input_request(body).map(|request| ServiceControlRequest::Input(Box::new(request)))
    } else {
        decode_legacy_control_body(body).map(ServiceControlRequest::Legacy)
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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
    let mut value = json!({"protocol_version":request.protocol_version,"kind":kind,"identity":request.identity,"expires_at_mono_ns":request.expires_at_mono_ns});
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
pub(in crate::runtime::runner) fn decode_input_reply(
    body: &[u8],
    request: &InputControlRequest,
) -> Result<InputControlReply> {
    replies::decode(body, request)
}
pub(in crate::runtime::runner) fn encode_input_reply(
    reply: std::result::Result<&InputControlReply, &CafleetError>,
) -> Result<Vec<u8>> {
    replies::encode(reply)
}
