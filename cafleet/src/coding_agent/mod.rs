//! Coding-agent backends (SPEC §6.7) — the `CodingAgent` surface, the
//! three-entry registry, and the claude/codex/opencode argv builders. The
//! colocated tests pin the contract; the expected API is catalogued in
//! [`test_support`].

pub mod claude;
pub mod codex;
pub mod opencode;
#[cfg(test)]
pub mod test_support;

use crate::error::CafleetError;

/// The spawn-precondition seam: PATH lookups and the home directory, injected
/// so preconditions are testable against a temp HOME (SPEC §9).
pub trait SpawnProbe {
    fn binary_on_path(&self, name: &str) -> bool;
    fn home_dir(&self) -> std::path::PathBuf;
}

pub trait CodingAgent {
    fn name(&self) -> &'static str;
    fn binary_name(&self) -> &'static str;
    fn validate_model(&self, model: Option<&str>) -> Result<(), CafleetError>;
    fn validate_effort(&self, effort: Option<&str>) -> Result<(), CafleetError>;
    fn ensure_available(&self, probe: &dyn SpawnProbe) -> Result<(), CafleetError>;
    fn build_spawn_argv(
        &self,
        prompt: &str,
        display_name: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Vec<String>;
}

/// Exact-name lookup over exactly the three backends — no fuzzy matching, no
/// fallback.
pub fn coding_agent(name: &str) -> Option<&'static dyn CodingAgent> {
    match name {
        "claude" => Some(&claude::Claude),
        "codex" => Some(&codex::Codex),
        "opencode" => Some(&opencode::Opencode),
        _ => None,
    }
}

pub(crate) fn missing_binary(binary_name: &str) -> CafleetError {
    CafleetError::App(format!("binary {binary_name} not found on PATH"))
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::coding_agent;

    #[test]
    fn the_registry_has_exactly_the_three_backends() {
        for name in ["claude", "codex", "opencode"] {
            let backend =
                coding_agent(name).unwrap_or_else(|| panic!("registry must carry '{name}'"));
            assert_eq!(backend.name(), name, "the key equals the backend's name");
        }
    }

    #[test]
    fn lookup_is_exact_with_no_fuzzy_matching_or_fallback() {
        assert!(coding_agent("python").is_none());
        assert!(coding_agent("Claude").is_none());
        assert!(coding_agent("").is_none());
    }
}
