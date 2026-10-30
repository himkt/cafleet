use super::CodingAgent;
use super::catalog::ProfileCatalog;
use crate::error::CafleetError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposerOperation {
    Prompt,
    Exec,
}

impl ComposerOperation {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Prompt => 0,
            Self::Exec => 1,
        }
    }

    pub(super) fn prefix_hex(self) -> &'static str {
        match self {
            Self::Prompt => "",
            Self::Exec => "2120",
        }
    }
}

#[derive(Clone)]
pub(super) struct ComposerSubmission {
    pub(crate) prefix: Vec<u8>,
    pub(crate) submit: Vec<u8>,
    pub(crate) fixture_ids: Vec<String>,
}

pub(crate) struct ComposerPlan {
    backend: CodingAgent,
    native_version: String,
    profile_version: String,
    rule_ids: Vec<String>,
    fixture_ids: Vec<String>,
    operation: ComposerOperation,
    prefix: Vec<u8>,
    submit: Vec<u8>,
}

impl ComposerPlan {
    pub(crate) fn backend(&self) -> CodingAgent {
        self.backend
    }

    pub(crate) fn native_version(&self) -> &str {
        &self.native_version
    }

    pub(crate) fn profile_version(&self) -> &str {
        &self.profile_version
    }

    pub(crate) fn rule_ids(&self) -> &[String] {
        &self.rule_ids
    }

    pub(crate) fn fixture_ids(&self) -> &[String] {
        &self.fixture_ids
    }

    pub(crate) fn encode(&self, literal: &str) -> Result<Vec<u8>, CafleetError> {
        assert!(
            self.prefix.len() == self.operation.prefix_hex().len() / 2,
            "validated composer prefix matches its operation"
        );
        let size = self
            .prefix
            .len()
            .checked_add(literal.len())
            .and_then(|size| size.checked_add(self.submit.len()))
            .filter(|size| *size <= 65536)
            .ok_or_else(|| {
                CafleetError::Value("native composer payload exceeds 65536 bytes".into())
            })?;
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(&self.prefix);
        bytes.extend_from_slice(literal.as_bytes());
        bytes.extend_from_slice(&self.submit);
        Ok(bytes)
    }

    pub(crate) fn stages(&self, literal: &str) -> Result<[Vec<u8>; 2], CafleetError> {
        let mut encoded = self.encode(literal)?;
        let submit = encoded.split_off(encoded.len() - self.submit.len());
        Ok([encoded, submit])
    }
}

impl ProfileCatalog {
    pub(crate) fn composer_plan(
        &self,
        backend: CodingAgent,
        native_version: &str,
        operation: ComposerOperation,
    ) -> Option<ComposerPlan> {
        let profile = self.matching(backend, Some(native_version))?;
        let submission = profile.submissions[operation.index()].as_ref()?;
        let observation = profile
            .composer
            .as_ref()
            .expect("executable composer requires validated observation provenance");
        Some(ComposerPlan {
            backend,
            native_version: native_version.into(),
            profile_version: profile.version.clone(),
            rule_ids: observation.rules.clone(),
            fixture_ids: submission.fixture_ids.clone(),
            operation,
            prefix: submission.prefix.clone(),
            submit: submission.submit.clone(),
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::broker::runtime::Coverage;
    use crate::coding_agent::profiles::catalog::test_support::{
        FIXTURE, PROFILE, VERSION, simple_definition,
    };
    use serde_json::{Value, json};
    pub(crate) const EMPTY: &str = "synthetic-composer_empty";
    pub(crate) const DRAFT: &str = "synthetic-composer_draft";
    pub(crate) const PROMPT_FIXTURE: &str = "synthetic-prompt-plan";
    pub(crate) const EXEC_FIXTURE: &str = "synthetic-exec-plan";
    pub(crate) const VERSION_TWO: &str = "0.0.2-synthetic";

    pub(crate) fn definition(prompt: Option<&str>, exec: Option<&str>) -> Value {
        let mut value = simple_definition("composer_empty", true);
        let draft = simple_definition("composer_draft", true)["profiles"][0]["rules"][0].clone();
        value["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(draft);
        for id in [PROMPT_FIXTURE, EXEC_FIXTURE] {
            let mut fixture = value["fixtures"][0].clone();
            fixture["id"] = json!(id);
            value["fixtures"].as_array_mut().unwrap().push(fixture);
        }
        let prompt = prompt.map(
            |submit| json!({"prefix_hex":"","submit_hex":submit,"fixture_ids":[PROMPT_FIXTURE]}),
        );
        let exec = exec.map(
            |submit| json!({"prefix_hex":"2120","submit_hex":submit,"fixture_ids":[EXEC_FIXTURE]}),
        );
        value["profiles"][0]["capabilities"]["composer"] = json!({
            "rule_ids":[DRAFT,EMPTY],"fixture_ids":[FIXTURE],
            "submission":{"prompt":prompt,"exec":exec}
        });
        value
    }

    pub(crate) fn operation_name(operation: ComposerOperation) -> &'static str {
        match operation {
            ComposerOperation::Prompt => "prompt",
            ComposerOperation::Exec => "exec",
        }
    }

    pub(crate) fn operation_prefix(operation: ComposerOperation) -> &'static [u8] {
        match operation {
            ComposerOperation::Prompt => b"",
            ComposerOperation::Exec => b"! ",
        }
    }

    pub(crate) fn plan(catalog: &ProfileCatalog, operation: ComposerOperation) -> ComposerPlan {
        catalog
            .composer_plan(CodingAgent::Codex, VERSION, operation)
            .expect("valid selected operation has an executable plan")
    }

    pub(crate) fn capabilities(
        catalog: &ProfileCatalog,
        backend: CodingAgent,
        version: Option<&str>,
        observation: Coverage,
        operations: (Coverage, Coverage),
    ) {
        let actual = catalog.capabilities(backend, version);
        assert_eq!(actual.composer, observation);
        assert_eq!(actual.prompt_submission, operations.0);
        assert_eq!(actual.native_exec_submission, operations.1);
    }

    pub(crate) fn metadata(plan: &ComposerPlan, fixtures: &[String]) {
        assert_eq!(plan.backend(), CodingAgent::Codex);
        assert_eq!(plan.native_version(), VERSION);
        assert_eq!(plan.profile_version(), PROFILE);
        assert_eq!(plan.rule_ids(), &[DRAFT.to_owned(), EMPTY.to_owned()]);
        assert_eq!(plan.fixture_ids(), fixtures);
    }

    pub(crate) fn reject(
        good: &Value,
        bad: &Value,
        operation: ComposerOperation,
        label: &str,
    ) -> CafleetError {
        let catalog = ProfileCatalog::from_definitions(good).unwrap();
        let _ = plan(&catalog, operation)
            .encode("same-operation positive control")
            .unwrap();
        match ProfileCatalog::from_definitions(bad) {
            Err(error) => error,
            Ok(_) => panic!("invalid executable definition rejected: {label}"),
        }
    }

    pub(crate) fn expected(operation: ComposerOperation, literal: &str, submit: &[u8]) -> Vec<u8> {
        let mut bytes = operation_prefix(operation).to_vec();
        bytes.extend_from_slice(literal.as_bytes());
        bytes.extend_from_slice(submit);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::broker::runtime::{Coverage, PermissionDecision};
    use crate::coding_agent::profiles::catalog::test_support::synthetic_definitions as definitions;
    use crate::coding_agent::profiles::catalog::test_support::*;
    use serde_json::json;

    #[test]
    fn composer_observation_only_definition_preserves_observation_without_executable_plans() {
        let value = simple_definition("composer_empty", true);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Unknown, Coverage::Unknown),
        );
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            assert!(
                catalog
                    .composer_plan(CodingAgent::Codex, VERSION, operation)
                    .is_none()
            );
        }
        assert_eq!(
            catalog
                .capabilities(CodingAgent::Codex, Some(VERSION))
                .detection,
            Coverage::Unknown
        );
        assert!(
            catalog
                .capabilities(CodingAgent::Codex, Some(VERSION))
                .answer_decisions
                .is_empty()
        );
    }

    #[test]
    fn composer_nullable_operations_and_absent_composer_keep_executable_coverage_unknown() {
        for value in [definition(None, None), definitions()] {
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            let observation = if value["profiles"][0]["capabilities"]["composer"].is_null() {
                Coverage::Unknown
            } else {
                Coverage::Tested
            };
            capabilities(
                &catalog,
                CodingAgent::Codex,
                Some(VERSION),
                observation,
                (Coverage::Unknown, Coverage::Unknown),
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(
                    catalog
                        .composer_plan(CodingAgent::Codex, VERSION, operation)
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn composer_prompt_plan_retains_exact_selected_metadata_and_independent_coverage() {
        let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
        metadata(
            &plan(&catalog, ComposerOperation::Prompt),
            &[PROMPT_FIXTURE.to_owned()],
        );
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Tested, Coverage::Unknown),
        );
        assert!(
            catalog
                .composer_plan(CodingAgent::Codex, VERSION, ComposerOperation::Exec)
                .is_none()
        );
        let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(actual.detection, Coverage::Unknown);
        assert!(actual.answer_decisions.is_empty());
    }

    #[test]
    fn composer_exec_plan_retains_exact_selected_metadata_and_independent_coverage() {
        let catalog = ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
        metadata(
            &plan(&catalog, ComposerOperation::Exec),
            &[EXEC_FIXTURE.to_owned()],
        );
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Unknown, Coverage::Tested),
        );
        assert!(
            catalog
                .composer_plan(CodingAgent::Codex, VERSION, ComposerOperation::Prompt)
                .is_none()
        );
        let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(actual.detection, Coverage::Unknown);
        assert!(actual.answer_decisions.is_empty());
    }

    #[test]
    fn composer_operation_provenance_and_exact_selection_do_not_authorize_other_profiles() {
        let mut value = definition(Some("7e"), Some("6b6c"));
        value["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(definitions()["profiles"][0]["rules"][0].clone());
        value["profiles"][0]["capabilities"]["detection"] =
            json!({"rule_ids":[RULE],"fixture_ids":[FIXTURE]});
        let answers = [("allow", "allow-once", "allow"), ("deny", "deny-once", "deny")].map(|(decision, choice, focus)| {
                json!({"decision":decision,"request_rule_id":RULE,"answer_rule_id":format!("synthetic-{decision}-plan"),
                    "choice_id":choice,"focus_matcher_id":focus,"fixture_ids":[FIXTURE],
                    "stages":[{"bytes_hex":"78","required_rule_id":RULE,"required_choice_id":choice}]})
            });
        value["profiles"][0]["capabilities"]["answers"] = json!(answers);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        let prompt = plan(&catalog, ComposerOperation::Prompt);
        let exec = plan(&catalog, ComposerOperation::Exec);
        metadata(&prompt, &[PROMPT_FIXTURE.to_owned()]);
        metadata(&exec, &[EXEC_FIXTURE.to_owned()]);
        assert_eq!(prompt.encode("literal").unwrap(), b"literal~");
        assert_eq!(exec.encode("literal").unwrap(), b"! literalkl");
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Tested, Coverage::Tested),
        );
        let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(actual.detection, Coverage::Tested);
        assert_eq!(
            actual.answer_decisions,
            [PermissionDecision::Allow, PermissionDecision::Deny]
        );
        capabilities(
            &catalog,
            CodingAgent::Codex,
            None,
            Coverage::Unknown,
            (Coverage::Unknown, Coverage::Unknown),
        );
        let bundled = ProfileCatalog::bundled().unwrap();
        for &backend in CodingAgent::all() {
            let actual = bundled.capabilities(backend, Some(VERSION));
            assert_eq!(
                (actual.prompt_submission, actual.native_exec_submission),
                (Coverage::Unknown, Coverage::Unknown),
                "{backend:?}"
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(bundled.composer_plan(backend, VERSION, operation).is_none());
                if backend != CodingAgent::Codex {
                    assert!(catalog.composer_plan(backend, VERSION, operation).is_none());
                    capabilities(
                        &catalog,
                        backend,
                        Some(VERSION),
                        Coverage::Unknown,
                        (Coverage::Unknown, Coverage::Unknown),
                    );
                }
            }
        }
    }

    #[test]
    fn composer_prompt_preserves_literal_utf8_between_empty_prefix_and_tested_submit_bytes() {
        let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
        let literal = "  literal 世界 '$x' `backticks` \\\"quotes\\\"  ";
        let bytes = plan(&catalog, ComposerOperation::Prompt)
            .encode(literal)
            .unwrap();
        assert_eq!(bytes, expected(ComposerOperation::Prompt, literal, b"~"));
        assert_eq!(&bytes[..literal.len()], literal.as_bytes());
    }

    #[test]
    fn composer_exec_owns_exact_bang_space_prefix_without_changing_the_caller_literal() {
        let catalog = ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
        let literal = "printf '%s' 世界 '$x' `literal`";
        let bytes = plan(&catalog, ComposerOperation::Exec)
            .encode(literal)
            .unwrap();
        assert_eq!(bytes, expected(ComposerOperation::Exec, literal, b"kl"));
        assert_eq!(&bytes[..2], b"! ");
        assert_eq!(&bytes[2..2 + literal.len()], literal.as_bytes());
    }
    #[test]
    fn composer_native_payload_limit_counts_prefix_utf8_literal_and_submit_bytes() {
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            for submit_length in [1, 32] {
                let hex = "6b".repeat(submit_length);
                let value = match operation {
                    ComposerOperation::Prompt => definition(Some(&hex), None),
                    ComposerOperation::Exec => definition(None, Some(&hex)),
                };
                let catalog = ProfileCatalog::from_definitions(&value).unwrap();
                let selected = plan(&catalog, operation);
                let literal_size = 65536 - operation_prefix(operation).len() - submit_length;
                let literal = format!(
                    "{}{}",
                    "界".repeat(literal_size / 3),
                    "x".repeat(literal_size % 3)
                );
                assert_eq!(literal.len(), literal_size);
                let actual = selected.encode(&literal).unwrap();
                assert_eq!(actual.len(), 65536);
                assert_eq!(
                    actual,
                    expected(operation, &literal, &vec![b'k'; submit_length])
                );
                let oversized = format!("{literal}x");
                assert!(
                    selected.encode(&oversized).is_err(),
                    "65537 native payload bytes reject"
                );
            }
        }
    }

    #[test]
    fn composer_plans_withhold_debug_serialization_and_private_bytes_from_errors() {
        assert_no_debug_or_serialize!(ComposerPlan);
        const SUBMIT: &str = "E16_PRIVATE_SUBMIT";
        const LITERAL: &str = "E16_PRIVATE_LITERAL";
        let hex = crate::digest::hex(SUBMIT.as_bytes());
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            let good = match operation {
                ComposerOperation::Prompt => definition(Some(&hex), None),
                ComposerOperation::Exec => definition(None, Some(&hex)),
            };
            let catalog = ProfileCatalog::from_definitions(&good).unwrap();
            let selected = plan(&catalog, operation);
            assert_eq!(
                selected.encode("positive").unwrap(),
                expected(operation, "positive", SUBMIT.as_bytes())
            );
            let mut bad = good.clone();
            bad["profiles"][0]["capabilities"]["composer"]["submission"]
                [operation_name(operation)]["prefix_hex"] = match operation {
                ComposerOperation::Prompt => json!("00"),
                ComposerOperation::Exec => json!(""),
            };
            let definition_error =
                reject(&good, &bad, operation, "private submit with invalid prefix");
            let literal = format!("{LITERAL}{}", "x".repeat(65537));
            let encode_error = match selected.encode(&literal) {
                Err(error) => error,
                Ok(_) => panic!("oversized private literal rejects"),
            };
            for error in [definition_error, encode_error] {
                let diagnostic = format!("{error:?}\n{error}\n{}", error.message());
                for private in [SUBMIT, LITERAL, &hex] {
                    assert!(
                        !diagnostic.contains(private),
                        "plan errors withhold selected bytes and literal payload"
                    );
                }
            }
        }
    }
}
