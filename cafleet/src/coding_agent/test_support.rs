//! Shared fixtures for the coding-agent colocated contract tests (SPEC §6.7).
//!
//! Expected public API pinned by the module test suites:
//!
//! ```text
//! // The spawn-precondition seam (SPEC §9: PATH check + opencode's
//! // preset-existence check against a temp HOME).
//! pub trait SpawnProbe {
//!     fn binary_on_path(&self, name: &str) -> bool;
//!     fn home_dir(&self) -> std::path::PathBuf;
//! }
//!
//! pub trait CodingAgent {
//!     fn name(&self) -> &'static str;
//!     fn binary_name(&self) -> &'static str;
//!     fn validate_model(&self, model: Option<&str>) -> Result<(), CafleetError>;
//!     fn validate_effort(&self, effort: Option<&str>) -> Result<(), CafleetError>;
//!     fn ensure_available(&self, probe: &dyn SpawnProbe) -> Result<(), CafleetError>;
//!     fn build_spawn_argv(&self, prompt: &str, display_name: &str,
//!         model: Option<&str>, effort: Option<&str>) -> Vec<String>;
//! }
//!
//! // Registry: exact-name lookup over exactly three eager singletons.
//! pub fn coding_agent(name: &str) -> Option<&'static dyn CodingAgent>
//! ```
//!
//! `validate_model` / `validate_effort` failures carry the backend's message;
//! the exit-code class is the CLI caller's translation (usage, exit 2 — SPEC
//! §6.7), so these tests assert `message()` only, never the variant.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::SpawnProbe;

pub struct FakeProbe {
    pub binaries: BTreeSet<String>,
    pub home: PathBuf,
}

impl FakeProbe {
    pub fn with_binary(name: &str, home: &std::path::Path) -> Self {
        FakeProbe {
            binaries: BTreeSet::from([name.to_string()]),
            home: home.to_path_buf(),
        }
    }

    pub fn without_binaries(home: &std::path::Path) -> Self {
        FakeProbe {
            binaries: BTreeSet::new(),
            home: home.to_path_buf(),
        }
    }
}

impl SpawnProbe for FakeProbe {
    fn binary_on_path(&self, name: &str) -> bool {
        self.binaries.contains(name)
    }

    fn home_dir(&self) -> PathBuf {
        self.home.clone()
    }
}

pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}
