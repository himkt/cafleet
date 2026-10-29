use std::collections::BTreeSet;

use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{EventEvidence, invalid, require};
use crate::error::CafleetError;

const SCREEN_FIELDS: [&str; 4] = ["operation", "choices", "native_request_id", "excerpt"];
const EVIDENCE_LIMIT: usize = 12 * 1024;
const EXCERPT_LIMIT: usize = 8 * 1024;

pub struct EvidencePolicy {
    mode: EventEvidence,
    patterns: Vec<Regex>,
    sensitive_rules: BTreeSet<String>,
}

impl EvidencePolicy {
    pub fn metadata() -> Self {
        Self {
            mode: EventEvidence::Metadata,
            patterns: Vec::new(),
            sensitive_rules: BTreeSet::new(),
        }
    }

    pub fn redacted(config: &Value, known_rules: &[&str]) -> Result<Self, CafleetError> {
        let fail = || CafleetError::Usage("invalid event redaction configuration".into());
        let config = config.as_object().ok_or_else(fail)?;
        if config.get("schema_version") != Some(&json!(1)) {
            return Err(fail());
        }
        let patterns = config
            .get("patterns")
            .and_then(Value::as_array)
            .ok_or_else(fail)?;
        if patterns.is_empty() {
            return Err(fail());
        }
        let patterns = patterns
            .iter()
            .map(|pattern| Regex::new(pattern.as_str().ok_or_else(fail)?).map_err(|_| fail()))
            .collect::<Result<Vec<_>, _>>()?;
        let rules = config
            .get("sensitive_rules")
            .and_then(Value::as_array)
            .ok_or_else(fail)?;
        let sensitive_rules = rules
            .iter()
            .map(|rule| {
                let rule = rule.as_str().ok_or_else(fail)?;
                if !known_rules.contains(&rule) {
                    return Err(fail());
                }
                Ok(rule.to_owned())
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(Self {
            mode: EventEvidence::Redacted,
            patterns,
            sensitive_rules,
        })
    }

    pub fn apply(&self, raw: &Value) -> Result<Value, CafleetError> {
        let raw = raw
            .as_object()
            .ok_or_else(|| invalid("evidence", "object required"))?;
        let mut result = Map::new();
        for key in [
            "backend",
            "native_version",
            "profile_version",
            "rule_id",
            "screen_seq",
            "truncated",
        ] {
            let value = raw
                .get(key)
                .ok_or_else(|| invalid(key, "required evidence field"))?;
            result.insert(key.into(), value.clone());
        }
        let rule = result["rule_id"]
            .as_str()
            .ok_or_else(|| invalid("rule_id", "string required"))?;
        let sensitive = raw
            .get("sensitive")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("sensitive", "boolean required"))?;
        let metadata = self.mode == EventEvidence::Metadata
            || sensitive
            || self.sensitive_rules.contains(rule);
        result.insert(
            "evidence_policy".into(),
            json!(if metadata { "metadata" } else { "redacted" }),
        );
        let mut truncated = result["truncated"]
            .as_bool()
            .ok_or_else(|| invalid("truncated", "boolean required"))?;
        for field in SCREEN_FIELDS {
            let value = if metadata {
                Value::Null
            } else {
                let value = raw
                    .get(field)
                    .ok_or_else(|| invalid(field, "required screen field"))?;
                self.transform(value, &mut truncated)?
            };
            result.insert(field.into(), value);
            if field == "operation" {
                let rows = if metadata {
                    Value::Null
                } else {
                    raw.get("operation_rows")
                        .ok_or_else(|| invalid("operation_rows", "required screen field"))?
                        .clone()
                };
                result.insert("operation_rows".into(), rows);
            }
        }
        validate_operation_rows(&result)?;
        if let Some(choices) = result["choices"].as_array_mut()
            && choices.len() > 128
        {
            choices.truncate(128);
            truncated = true;
        }
        result.insert("truncated".into(), json!(truncated));
        let mut value = Value::Object(result);
        while value.to_string().len() > EVIDENCE_LIMIT - 128 {
            let mut longest: Option<(String, Option<usize>, usize)> = None;
            for field in SCREEN_FIELDS {
                match &value[field] {
                    Value::String(text) if text.len() > 1 => {
                        if longest.as_ref().is_none_or(|entry| text.len() > entry.2) {
                            longest = Some((field.into(), None, text.len()));
                        }
                    }
                    Value::Array(choices) => {
                        for (index, choice) in choices.iter().enumerate() {
                            let text = choice
                                .as_str()
                                .ok_or_else(|| invalid(field, "string choices required"))?;
                            if text.len() > 1
                                && longest.as_ref().is_none_or(|entry| text.len() > entry.2)
                            {
                                longest = Some((field.into(), Some(index), text.len()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            let (field, index, length) =
                longest.ok_or_else(|| invalid("evidence", "metadata exceeds evidence bound"))?;
            let target = if let Some(index) = index {
                &mut value[&field][index]
            } else {
                &mut value[&field]
            };
            let text = target.as_str().expect("the longest field was a string");
            *target = json!(truncate_utf8(text, length / 2));
            value["truncated"] = json!(true);
        }
        value["evidence_digest"] = json!(digest(&value));
        validate(&value)?;
        Ok(value)
    }

    pub fn displayed_mode(
        &self,
        rule: &str,
        sensitive: bool,
        mode: Option<&str>,
    ) -> Result<Option<String>, CafleetError> {
        if self.mode == EventEvidence::Metadata || sensitive || self.sensitive_rules.contains(rule)
        {
            return Ok(None);
        }
        let mut truncated = false;
        Ok(self
            .transform(&json!(mode), &mut truncated)?
            .as_str()
            .map(str::to_owned))
    }

    fn transform(&self, value: &Value, truncated: &mut bool) -> Result<Value, CafleetError> {
        match value {
            Value::Null => Ok(Value::Null),
            Value::String(text) => {
                let transformed = if self.patterns.iter().any(|pattern| pattern.is_match(text)) {
                    "[redacted]".into()
                } else {
                    escape_controls(text)
                };
                *truncated |= transformed.len() > EXCERPT_LIMIT;
                Ok(json!(truncate_utf8(&transformed, EXCERPT_LIMIT)))
            }
            Value::Array(values) => values
                .iter()
                .map(|value| {
                    require(value.is_string(), "choices", "string choice required")?;
                    self.transform(value, truncated)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Err(invalid(
                "evidence",
                "nullable text or string array required",
            )),
        }
    }
}

fn truncate_utf8(text: &str, limit: usize) -> String {
    let mut boundary = text.len().min(limit);
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text[..boundary].into()
}

fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character <= '\u{1f}' || character == '\u{7f}' {
                format!("\\x{:02x}", u32::from(character))
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical(&object[key])))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

pub(super) fn digest(value: &Value) -> String {
    let mut value = value.clone();
    value
        .as_object_mut()
        .expect("evidence is an object")
        .remove("evidence_digest");
    Sha256::digest(canonical(&value).to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_operation_rows(object: &Map<String, Value>) -> Result<(), CafleetError> {
    let rows = object
        .get("operation_rows")
        .ok_or_else(|| invalid("operation_rows", "required nullable field"))?;
    let consistent = match object.get("operation") {
        Some(Value::String(_)) => rows.as_u64().is_some_and(|count| count > 0),
        Some(Value::Null) => rows.is_null(),
        _ => false,
    };
    require(
        consistent,
        "operation_rows",
        "a positive row count exactly when an operation is present",
    )
}

pub(crate) fn validate(value: &Value) -> Result<(), CafleetError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("evidence_json", "object required"))?;
    for field in ["backend", "rule_id"] {
        require(
            object
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty() && text.len() <= 256),
            field,
            "bounded evidence identity required",
        )?;
    }
    for field in ["native_version", "profile_version"] {
        require(
            object.get(field).is_some_and(|value| {
                value.is_null() || value.as_str().is_some_and(|text| text.len() <= 256)
            }),
            field,
            "nullable bounded string required",
        )?;
    }
    require(
        object.get("screen_seq").and_then(Value::as_u64).is_some(),
        "screen_seq",
        "nonnegative sequence required",
    )?;
    require(
        object.get("truncated").and_then(Value::as_bool).is_some(),
        "truncated",
        "boolean required",
    )?;
    let policy = object
        .get("evidence_policy")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("evidence_policy", "policy required"))?;
    require(
        matches!(policy, "metadata" | "redacted"),
        "evidence_policy",
        "unknown policy",
    )?;
    require(
        !object.contains_key("semantic_fingerprint") && !object.contains_key("sensitive"),
        "evidence_json",
        "in-memory evidence cannot be persisted",
    )?;
    let allowed = [
        "backend",
        "native_version",
        "profile_version",
        "rule_id",
        "screen_seq",
        "truncated",
        "evidence_policy",
        "operation",
        "operation_rows",
        "choices",
        "native_request_id",
        "excerpt",
        "evidence_digest",
    ];
    require(
        object.keys().all(|key| allowed.contains(&key.as_str())),
        "evidence_json",
        "unknown evidence field",
    )?;
    for field in SCREEN_FIELDS {
        let entry = object
            .get(field)
            .ok_or_else(|| invalid(field, "required nullable field"))?;
        if policy == "metadata" {
            require(entry.is_null(), field, "metadata screen field must be null")?;
        } else if field == "choices" {
            require(
                entry.is_null()
                    || entry
                        .as_array()
                        .is_some_and(|choices| choices.iter().all(Value::is_string)),
                field,
                "nullable string choices required",
            )?;
        } else {
            require(
                entry.is_null() || entry.is_string(),
                field,
                "nullable string required",
            )?;
        }
    }
    validate_operation_rows(object)?;
    require(
        value.to_string().len() <= EVIDENCE_LIMIT,
        "evidence_json",
        "evidence exceeds bound",
    )?;
    require(
        object.get("evidence_digest").and_then(Value::as_str) == Some(digest(value).as_str()),
        "evidence_digest",
        "post-policy digest mismatch",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::{self as storage, *};

    #[test]
    fn metadata_removes_all_screen_fields_and_hashes_only_post_policy_evidence() {
        let policy = evidence::EvidencePolicy::metadata();
        let first = policy.apply(&raw_evidence(SECRET)).unwrap();
        let second = policy
            .apply(&raw_evidence("a different hidden native request"))
            .unwrap();
        for field in ["operation", "choices", "native_request_id", "excerpt"] {
            assert_eq!(first.get(field), Some(&Value::Null), "{field}");
        }
        assert_eq!(first["evidence_digest"], second["evidence_digest"]);
        assert_eq!(first["evidence_digest"].as_str().unwrap().len(), 64);
        assert!(!first.to_string().contains(SECRET));
        assert!(first.get("semantic_fingerprint").is_none());
        assert_eq!(first["evidence_policy"], "metadata");
    }

    const EVIDENCE_KEYS: [&str; 13] = [
        "backend",
        "native_version",
        "profile_version",
        "rule_id",
        "screen_seq",
        "truncated",
        "evidence_policy",
        "operation",
        "operation_rows",
        "choices",
        "native_request_id",
        "excerpt",
        "evidence_digest",
    ];

    fn redacting(sensitive_rules: Value) -> evidence::EvidencePolicy {
        evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":sensitive_rules}),
            &["native-tool-permission"],
        )
        .unwrap()
    }

    fn independent_evidence_digest(value: &Value) -> String {
        use sha2::{Digest, Sha256};
        fn sorted(value: &Value) -> Value {
            match value {
                Value::Object(object) => {
                    let mut keys = object.keys().collect::<Vec<_>>();
                    keys.sort();
                    Value::Object(
                        keys.into_iter()
                            .map(|key| (key.clone(), sorted(&object[key])))
                            .collect(),
                    )
                }
                Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
                _ => value.clone(),
            }
        }
        let mut value = value.clone();
        value.as_object_mut().unwrap().remove("evidence_digest");
        Sha256::digest(sorted(&value).to_string().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn evidence_orders_operation_rows_after_operation_and_nulls_it_with_the_operation() {
        let mut raw = raw_evidence("visible");
        raw["operation"] = json!("run a\nb");
        raw["operation_rows"] = json!(2);
        let metadata = evidence::EvidencePolicy::metadata().apply(&raw).unwrap();
        assert_key_order(&metadata, &EVIDENCE_KEYS);
        assert_eq!(metadata["operation"], Value::Null);
        assert_eq!(metadata["operation_rows"], Value::Null);
        let redacted = redacting(json!([])).apply(&raw).unwrap();
        assert_key_order(&redacted, &EVIDENCE_KEYS);
        assert_eq!(redacted["operation"], "run a\\x0ab");
        assert_eq!(redacted["operation_rows"], 2);
        raw["operation"] = json!(format!("run {SECRET}\nb"));
        let matched = redacting(json!([])).apply(&raw).unwrap();
        assert_eq!(matched["operation"], "[redacted]");
        assert_eq!(
            matched["operation_rows"], 2,
            "a redacted operation keeps its displayed row count"
        );
        let sensitive = redacting(json!(["native-tool-permission"]))
            .apply(&raw)
            .unwrap();
        assert_eq!(sensitive["operation"], Value::Null);
        assert_eq!(sensitive["operation_rows"], Value::Null);
        raw["operation"] = Value::Null;
        raw["operation_rows"] = Value::Null;
        let absent = redacting(json!([])).apply(&raw).unwrap();
        assert_eq!(absent["operation"], Value::Null);
        assert_eq!(absent["operation_rows"], Value::Null);
        for value in [&metadata, &redacted, &matched, &sensitive, &absent] {
            assert_eq!(
                value["evidence_digest"],
                independent_evidence_digest(value),
                "the digest covers operation_rows"
            );
        }
    }

    #[test]
    fn evidence_policy_rejects_a_row_count_that_disagrees_with_the_operation() {
        for (operation, rows) in [
            (json!("run a"), Value::Null),
            (Value::Null, json!(1)),
            (json!("run a"), json!(0)),
            (json!("run a"), json!(-1)),
            (json!("run a"), json!("1")),
            (json!("run a"), json!(1.5)),
        ] {
            let mut raw = raw_evidence("visible");
            raw["operation"] = operation.clone();
            raw["operation_rows"] = rows.clone();
            assert!(
                redacting(json!([])).apply(&raw).is_err(),
                "operation {operation} with rows {rows}"
            );
        }
    }

    #[test]
    fn evidence_validation_binds_operation_rows_to_the_operation() {
        let mut raw = raw_evidence("visible");
        raw["operation"] = json!("run a\nb");
        raw["operation_rows"] = json!(2);
        let valid = redacting(json!([])).apply(&raw).unwrap();
        evidence::validate(&valid).unwrap();
        let redigest = |change: &dyn Fn(&mut Value)| {
            let mut value = valid.clone();
            change(&mut value);
            value["evidence_digest"] = json!(independent_evidence_digest(&value));
            value
        };
        evidence::validate(&redigest(&|value| value["operation_rows"] = json!(3)))
            .expect("a consistent recomputed row count validates");
        let metadata = evidence::EvidencePolicy::metadata().apply(&raw).unwrap();
        evidence::validate(&metadata).unwrap();
        for (label, change) in [
            (
                "missing",
                &(|value: &mut Value| {
                    value.as_object_mut().unwrap().remove("operation_rows");
                }) as &dyn Fn(&mut Value),
            ),
            ("null with an operation", &|value| {
                value["operation_rows"] = Value::Null
            }),
            ("zero", &|value| value["operation_rows"] = json!(0)),
            ("negative", &|value| value["operation_rows"] = json!(-2)),
            ("string", &|value| value["operation_rows"] = json!("2")),
            ("count without an operation", &|value| {
                value["operation"] = Value::Null
            }),
        ] {
            assert!(
                evidence::validate(&redigest(change)).is_err(),
                "redacted evidence with operation_rows {label}"
            );
        }
        let mut forged = metadata.clone();
        forged["operation_rows"] = json!(2);
        forged["evidence_digest"] = json!(independent_evidence_digest(&forged));
        assert!(
            evidence::validate(&forged).is_err(),
            "metadata evidence carries no row count"
        );
    }

    #[test]
    fn matching_redaction_replaces_each_normalized_field_in_full() {
        let policy = evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
            &["native-tool-permission"],
        )
        .unwrap();
        let sanitized = policy.apply(&raw_evidence(SECRET)).unwrap();
        for field in ["operation", "native_request_id", "excerpt"] {
            assert_eq!(sanitized[field], "[redacted]", "{field}");
        }
        assert_eq!(sanitized["choices"], json!(["[redacted]", "deny"]));
        assert_eq!(sanitized["evidence_policy"], "redacted");
        assert!(!sanitized.to_string().contains(SECRET));
        assert!(sanitized.get("semantic_fingerprint").is_none());
    }

    #[test]
    fn sensitive_forms_and_selected_rules_reduce_the_entire_form_to_metadata() {
        for by_profile in [false, true] {
            let sensitive_rules = if by_profile {
                json!([])
            } else {
                json!(["native-tool-permission"])
            };
            let policy = evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":["unmatched"],"sensitive_rules":sensitive_rules}),
            &["native-tool-permission"],
        )
        .unwrap();
            let mut raw = raw_evidence(SECRET);
            raw["sensitive"] = json!(by_profile);
            let sanitized = policy.apply(&raw).unwrap();
            for field in ["operation", "choices", "native_request_id", "excerpt"] {
                assert_eq!(sanitized[field], Value::Null, "{field}");
            }
            assert!(!sanitized.to_string().contains(SECRET));
        }
    }

    #[test]
    fn invalid_redaction_configuration_fails_instead_of_retaining_raw_evidence() {
        for config in [
            json!({"schema_version":2,"patterns":["secret"],"sensitive_rules":[]}),
            json!({"schema_version":1,"patterns":[],"sensitive_rules":[]}),
            json!({"schema_version":1,"patterns":["["],"sensitive_rules":[]}),
            json!({"schema_version":1,"patterns":["secret"],"sensitive_rules":["unknown-rule"]}),
            json!([]),
        ] {
            assert!(
                evidence::EvidencePolicy::redacted(&config, &["native-tool-permission"]).is_err(),
                "{config}"
            );
        }
    }

    #[test]
    fn metadata_and_redacted_transitions_remain_secret_free_through_storage_and_updates() {
        for redacted in [false, true] {
            let mut fixture = Fixture::new();
            if redacted {
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                        [fixture.run.run_id],
                    )
                    .unwrap();
            }
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let policy = if redacted {
                evidence::EvidencePolicy::redacted(
                    &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
                    &["native-tool-permission"],
                )
                .unwrap()
            } else {
                evidence::EvidencePolicy::metadata()
            };
            let transformed = policy.apply(&raw_evidence(SECRET)).unwrap();
            let transition = opening(owner.run_id, 1, 1, transformed.clone());
            assert!(!serde_json::to_string(&transition).unwrap().contains(SECRET));
            let first = storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live,
            )
            .unwrap();
            let update = ObservedTransition {
                event_seq: 2,
                kind: RuntimeEventKind::InteractionObscured,
                evidence_json: transformed,
                ..transition
            };
            storage::persist_transition(&mut fixture.conn, &owner, &update, PersistenceMode::Live)
                .unwrap();
            assert_no_secret(&fixture.conn, SECRET);
            let current = storage::get_event(&fixture.reopen(), first.event_id).unwrap();
            assert!(
                !current
                    .interaction
                    .unwrap()
                    .evidence_json
                    .to_string()
                    .contains(SECRET)
            );
        }
    }

    #[test]
    fn evidence_utf8_and_complete_payload_bounds_record_truncation_after_policy() {
        let policy = evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":["unmatched"],"sensitive_rules":[]}),
            &["native-tool-permission"],
        )
        .unwrap();
        let mut raw = raw_evidence("ordinary");
        raw["excerpt"] = json!(format!("{}\u{1b}[2J\n", "界".repeat(8192)));
        let transformed = policy.apply(&raw).unwrap();
        assert!(transformed["excerpt"].as_str().unwrap().len() <= 8192);
        assert_eq!(transformed["truncated"], true);
        let mut fixture = Fixture::new();
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &opening(owner.run_id, 1, 1, transformed),
            PersistenceMode::Live,
        )
        .unwrap();
        let message = crate::broker::get_message(&fixture.conn, event.message_id.unwrap()).unwrap();
        assert!(message.text.len() <= 16384);
        assert!(serde_json::from_str::<Value>(&message.text).is_ok());
        assert!(!message.text.contains('\u{1b}'));
    }
}
