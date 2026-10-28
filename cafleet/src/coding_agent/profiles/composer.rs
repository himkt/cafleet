use super::CodingAgent;
use super::catalog::{Fixture, ProfileCatalog};
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
    pub prefix: Vec<u8>,
    pub submit: Vec<u8>,
    pub fixtures: Vec<Fixture>,
}

impl ComposerSubmission {
    pub(super) fn native_provenance(&self) -> bool {
        self.fixtures.iter().all(|fixture| fixture.kind == "native")
    }
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
}

impl ProfileCatalog {
    pub(crate) fn composer_plan(
        &self,
        backend: CodingAgent,
        native_version: &str,
        operation: ComposerOperation,
    ) -> Result<Option<ComposerPlan>, CafleetError> {
        let Some(profile) = self.matching(backend, Some(native_version)) else {
            return Ok(None);
        };
        let Some(submission) = &profile.submissions[operation.index()] else {
            return Ok(None);
        };
        let observation = profile
            .composer
            .as_ref()
            .expect("executable composer requires validated observation provenance");
        Ok(Some(ComposerPlan {
            backend,
            native_version: native_version.into(),
            profile_version: profile.version.clone(),
            rule_ids: observation.rules.clone(),
            fixture_ids: submission
                .fixtures
                .iter()
                .map(|fixture| fixture.id.clone())
                .collect(),
            operation,
            prefix: submission.prefix.clone(),
            submit: submission.submit.clone(),
        }))
    }
}
