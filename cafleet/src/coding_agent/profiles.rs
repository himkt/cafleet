mod catalog;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "The approved Batch 2 service adapter consumes this component"
    )
)]
mod detector;
use super::CodingAgent;
use crate::broker::runtime::PermissionDecision;
#[cfg(test)]
use crate::error::CafleetError;
pub use catalog::{NativeCapabilities, ProfileCatalog};
pub(crate) use detector::*;

#[cfg(test)]
mod contract_tests;

#[derive(Debug, Clone)]
pub struct AnswerCapability {
    backend: CodingAgent,
    native_version: String,
    profile_version: String,
    request_rule_id: String,
    answer_rule_id: String,
    decision: PermissionDecision,
    fixture_ids: Vec<String>,
}
impl AnswerCapability {
    pub fn backend(&self) -> CodingAgent {
        self.backend
    }
    pub fn native_version(&self) -> &str {
        &self.native_version
    }
    pub fn profile_version(&self) -> &str {
        &self.profile_version
    }
    pub fn request_rule_id(&self) -> &str {
        &self.request_rule_id
    }
    pub fn answer_rule_id(&self) -> &str {
        &self.answer_rule_id
    }
    pub fn decision(&self) -> PermissionDecision {
        self.decision
    }
    pub fn fixture_ids(&self) -> &[String] {
        &self.fixture_ids
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        backend: CodingAgent,
        native_version: &str,
        profile_version: &str,
        request_rule_id: &str,
        answer_rule_id: &str,
        decision: PermissionDecision,
        fixture_ids: &[String],
    ) -> Result<Self, CafleetError> {
        if [
            native_version,
            profile_version,
            request_rule_id,
            answer_rule_id,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || fixture_ids.is_empty()
            || fixture_ids.iter().any(|value| value.trim().is_empty())
        {
            return Err(CafleetError::Value(
                "answer capability requires complete profile provenance".into(),
            ));
        }
        Ok(Self {
            backend,
            native_version: native_version.into(),
            profile_version: profile_version.into(),
            request_rule_id: request_rule_id.into(),
            answer_rule_id: answer_rule_id.into(),
            decision,
            fixture_ids: fixture_ids.to_vec(),
        })
    }
}
