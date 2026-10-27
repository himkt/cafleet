use super::*;

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
const SUBMISSION_FIELDS: &[&str] = &["status", "input_id", "stage", "output_seq", "reason"];
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
fn submission(value: &Value) -> Result<SubmissionResult> {
    Object::new(value, SUBMISSION_FIELDS)?;
    let result: SubmissionResult = serde_json::from_value(value.clone())
        .map_err(|_| error("invalid control submission fields"))?;
    if let Some(id) = result.input_id {
        ensure(id > 0, "positive control input ID required")?;
    }
    Ok(result)
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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
        &["allow", "deny"],
    )?;
    validate_order(
        result.missing_evidence.iter().map(MissingEvidence::as_str),
        &[
            "operation",
            "arguments",
            "cwd",
            "environment",
            "targets",
            "effects",
            "choice_scope",
        ],
    )?;
    crate::broker::runtime::evidence::validate(&result.evidence)
        .map_err(|_| error("invalid control inspection evidence"))?;
    Ok(Box::new(result))
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
fn validate_order<'a>(values: impl Iterator<Item = &'a str>, domain: &[&str]) -> Result<()> {
    let mut previous = None;
    for value in values {
        let position = domain
            .iter()
            .position(|item| *item == value)
            .ok_or_else(|| error("invalid control inspection subset"))?;
        ensure(
            previous.is_none_or(|prior| position > prior),
            "control inspection subset order mismatch",
        )?;
        previous = Some(position);
    }
    Ok(())
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
pub(super) fn decode(body: &[u8], request: &InputControlRequest) -> Result<InputControlReply> {
    let value = strict_json::decode(body)?;
    if value.get("error").is_some() {
        let object = Object::new(&value, &["protocol_version", "error"])?;
        version(object.value("protocol_version"))?;
        let cause: String = object.get("error")?;
        return Err(error(format!("runner control: {cause}")));
    }
    let object = Object::new(&value, &["protocol_version", "result"])?;
    version(object.value("protocol_version"))?;
    let result = result(object.value("result"))?;
    validate_binding(&result, request)?;
    Ok(result)
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
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

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed input coordinator and host supply this production consumer"
    )
)]
pub(super) fn encode(
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
