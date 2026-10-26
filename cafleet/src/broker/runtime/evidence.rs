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
        }
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
