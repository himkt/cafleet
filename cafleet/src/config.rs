use crate::error::CafleetError;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(crate) database_url: String,
    pub(crate) max_text_len: usize,
    pub(crate) multiplexer: Option<String>,
}

impl Settings {
    /// Read each field from exactly its own `CAFLEET_*` name (no prefix
    /// magic). Bad numerics fail loudly; the default database URL is the only
    /// place `~` (the home directory) is expanded.
    pub(crate) fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Settings, CafleetError> {
        let database_url = match lookup("CAFLEET_DATABASE_URL") {
            Some(value) => value,
            None => {
                let home = home_dir().map_err(|_| {
                    CafleetError::App(
                        "HOME is not set; cannot derive the default database path".into(),
                    )
                })?;
                format!(
                    "sqlite:///{}/.local/share/cafleet/cafleet_v9.db",
                    home.display()
                )
            }
        };
        let max_text_len = match lookup("CAFLEET_MAX_TEXT_LEN") {
            None => 200,
            Some(raw) => raw.parse().map_err(|_| {
                CafleetError::App(format!(
                    "CAFLEET_MAX_TEXT_LEN must be a non-negative integer (got '{raw}')"
                ))
            })?,
        };
        Ok(Settings {
            database_url,
            max_text_len,
            multiplexer: lookup("CAFLEET_MULTIPLEXER"),
        })
    }

    pub(crate) fn from_env() -> Result<Settings, CafleetError> {
        Settings::from_lookup(|name| std::env::var(name).ok())
    }
}

#[cfg(test)]
impl Settings {
    pub(crate) fn for_database(path: &std::path::Path, multiplexer: Option<&str>) -> Settings {
        let database_url = format!("sqlite:///{}", path.display());
        Settings::from_lookup(|name| match name {
            "CAFLEET_DATABASE_URL" => Some(database_url.clone()),
            "CAFLEET_MULTIPLEXER" => multiplexer.map(str::to_owned),
            _ => None,
        })
        .unwrap()
    }
}

pub(crate) type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// opencode discovers skills only at fixed paths, so the skills base ignores
/// every variable.
pub(crate) fn opencode_skills_base(home: &Path) -> PathBuf {
    home.join(".config/opencode")
}

pub(crate) fn home_dir() -> Result<PathBuf, CafleetError> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| CafleetError::App("HOME is not set".into()))
}

pub(crate) fn require_absolute(var: &str, value: &str) -> Result<(), CafleetError> {
    if Path::new(value).is_absolute() {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "{var} must be an absolute path (got '{value}')"
        )))
    }
}

pub(crate) fn resolve(
    var: &str,
    env: EnvLookup,
    default: PathBuf,
) -> Result<PathBuf, CafleetError> {
    match env(var) {
        None => Ok(default),
        Some(value) => {
            require_absolute(var, &value)?;
            Ok(PathBuf::from(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::*;

    #[test]
    fn defaults_when_no_variable_is_set() {
        let s = Settings::from_lookup(|_| None).unwrap();
        assert_eq!(s.max_text_len, 200);
        assert_eq!(s.multiplexer, None);
    }

    #[test]
    fn default_database_url_points_at_expanded_home_cafleet_v9() {
        let s = Settings::from_lookup(|_| None).unwrap();
        assert_eq!(
            s.database_url,
            format!(
                "sqlite:///{}/.local/share/cafleet/cafleet_v9.db",
                std::env::var("HOME").unwrap()
            )
        );
        assert!(
            !s.database_url.contains('~'),
            "the default URL expands the home directory at startup"
        );
    }

    #[test]
    fn each_field_binds_to_its_exact_env_var() {
        let s = Settings::from_lookup(|name| match name {
            "CAFLEET_DATABASE_URL" => Some("sqlite:///srv/db/registry.db".to_string()),
            "CAFLEET_MAX_TEXT_LEN" => Some("50".to_string()),
            "CAFLEET_MULTIPLEXER" => Some("herdr".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(s.database_url, "sqlite:///srv/db/registry.db");
        assert_eq!(s.max_text_len, 50);
        assert_eq!(s.multiplexer, Some("herdr".to_string()));
    }

    #[test]
    fn user_database_url_is_passed_through_verbatim() {
        let s = Settings::from_lookup(|name| match name {
            "CAFLEET_DATABASE_URL" => Some("sqlite:///~/custom/path.db".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            s.database_url, "sqlite:///~/custom/path.db",
            "a user-supplied URL gets no ~ expansion and no rewriting"
        );
    }

    #[test]
    fn a_zero_text_limit_is_a_valid_explicit_setting() {
        let settings =
            Settings::from_lookup(|name| (name == "CAFLEET_MAX_TEXT_LEN").then(|| "0".into()))
                .unwrap();
        assert_eq!(settings.max_text_len, 0);
    }

    #[test]
    fn non_integer_or_negative_max_text_len_fails_loudly() {
        for raw in ["twenty", "-1"] {
            let error = Settings::from_lookup(|name| match name {
                "CAFLEET_MAX_TEXT_LEN" => Some(raw.to_string()),
                _ => None,
            })
            .unwrap_err();
            assert_eq!(
                error.message(),
                format!("CAFLEET_MAX_TEXT_LEN must be a non-negative integer (got '{raw}')")
            );
        }
    }
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn opencode_skills_base_is_pinned_to_the_fixed_discovery_path() {
        assert_eq!(
            opencode_skills_base(Path::new("/home/tester")),
            PathBuf::from("/home/tester/.config/opencode")
        );
    }
}
