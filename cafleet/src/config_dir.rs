//! Backend config-dir resolution (SPEC §6.3 *Config-dir resolution*): the
//! single owner of the `CLAUDE_CONFIG_DIR` / `CODEX_HOME` /
//! `OPENCODE_CONFIG_DIR` env-var lookups. Each env-resolving function takes
//! an injected env lookup and the home directory, so tests run against
//! fakes; `opencode_skills_base` reads no variable and stays infallible.

use std::path::{Path, PathBuf};

use crate::error::CafleetError;

pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The winning origin of a resolved directory, feeding `doctor`'s `source`
/// column and JSON.
#[derive(Debug)]
pub enum DirSource {
    EnvVar(&'static str),
    Default,
}

#[derive(Debug)]
pub struct ResolvedDir {
    pub path: PathBuf,
    pub source: DirSource,
}

pub fn claude_config_dir(env: EnvLookup, home: &Path) -> Result<ResolvedDir, CafleetError> {
    resolve("CLAUDE_CONFIG_DIR", env, home.join(".claude"))
}

pub fn codex_home(env: EnvLookup, home: &Path) -> Result<ResolvedDir, CafleetError> {
    resolve("CODEX_HOME", env, home.join(".codex"))
}

/// opencode discovers skills only at fixed paths, so the skills base ignores
/// every variable.
pub fn opencode_skills_base(home: &Path) -> PathBuf {
    home.join(".config/opencode")
}

pub fn opencode_preset_base(env: EnvLookup, home: &Path) -> Result<ResolvedDir, CafleetError> {
    resolve("OPENCODE_CONFIG_DIR", env, home.join(".opencode"))
}

fn resolve(
    var: &'static str,
    env: EnvLookup,
    default: PathBuf,
) -> Result<ResolvedDir, CafleetError> {
    match env(var) {
        None => Ok(ResolvedDir {
            path: default,
            source: DirSource::Default,
        }),
        Some(value) if Path::new(&value).is_absolute() => Ok(ResolvedDir {
            path: PathBuf::from(value),
            source: DirSource::EnvVar(var),
        }),
        Some(value) => Err(CafleetError::App(format!(
            "{var} must be an absolute path (got '{value}')"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::error::CafleetError;

    type Resolver = fn(&dyn Fn(&str) -> Option<String>, &Path) -> Result<ResolvedDir, CafleetError>;

    const ENV_RESOLVERS: [(Resolver, &str, &str); 3] = [
        (claude_config_dir, "CLAUDE_CONFIG_DIR", ".claude"),
        (codex_home, "CODEX_HOME", ".codex"),
        (opencode_preset_base, "OPENCODE_CONFIG_DIR", ".opencode"),
    ];

    fn home() -> PathBuf {
        PathBuf::from("/home/tester")
    }

    #[test]
    fn unset_variable_resolves_the_default_with_default_source() {
        for (resolve, var, default_dir) in ENV_RESOLVERS {
            let resolved = resolve(&|_| None, &home()).unwrap();
            assert_eq!(resolved.path, home().join(default_dir), "default for {var}");
            assert!(
                matches!(resolved.source, DirSource::Default),
                "unset {var} must report the Default source"
            );
        }
    }

    #[test]
    fn absolute_value_is_used_verbatim_with_env_var_source() {
        for (resolve, var, _default_dir) in ENV_RESOLVERS {
            let lookup = move |name: &str| (name == var).then(|| "/custom/cfg".to_string());
            let resolved = resolve(&lookup, &home()).unwrap();
            assert_eq!(resolved.path, PathBuf::from("/custom/cfg"));
            assert!(
                matches!(resolved.source, DirSource::EnvVar(name) if name == var),
                "source must carry the winning variable name for {var}"
            );
        }
    }

    #[test]
    fn each_resolver_reads_only_its_own_variable() {
        for (resolve, var, default_dir) in ENV_RESOLVERS {
            let lookup = move |name: &str| (name != var).then(|| "/other/cfg".to_string());
            let resolved = resolve(&lookup, &home()).unwrap();
            assert_eq!(
                resolved.path,
                home().join(default_dir),
                "{var}'s resolver must ignore every other variable"
            );
            assert!(matches!(resolved.source, DirSource::Default));
        }
    }

    #[test]
    fn empty_value_fails_with_the_pinned_error() {
        for (resolve, var, _default_dir) in ENV_RESOLVERS {
            let lookup = move |name: &str| (name == var).then(String::new);
            let err = resolve(&lookup, &home()).unwrap_err();
            assert!(matches!(err, CafleetError::App(_)), "App error for {var}");
            assert_eq!(
                err.message(),
                format!("{var} must be an absolute path (got '')")
            );
        }
    }

    #[test]
    fn relative_or_tilde_value_fails_with_the_pinned_error() {
        for (resolve, var, _default_dir) in ENV_RESOLVERS {
            for value in ["cfg/dir", "~/cfg"] {
                let lookup = move |name: &str| (name == var).then(|| value.to_string());
                let err = resolve(&lookup, &home()).unwrap_err();
                assert!(matches!(err, CafleetError::App(_)), "App error for {var}");
                assert_eq!(
                    err.message(),
                    format!("{var} must be an absolute path (got '{value}')")
                );
            }
        }
    }

    #[test]
    fn opencode_skills_base_is_pinned_to_the_fixed_discovery_path() {
        assert_eq!(
            opencode_skills_base(Path::new("/home/tester")),
            PathBuf::from("/home/tester/.config/opencode")
        );
    }
}
