use crate::error::CafleetError;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(crate) database_url: String,
    pub(crate) database_url_explicit: bool,
}

impl Settings {
    /// Selects `--database-url` verbatim, or the running version's default database; the default
    /// is the only place the home directory is expanded.
    pub(crate) fn new(database_url: Option<String>) -> Result<Settings, CafleetError> {
        let database_url_explicit = database_url.is_some();
        let database_url = match database_url {
            Some(url) => url,
            None => default_database_url()?,
        };
        Ok(Settings {
            database_url,
            database_url_explicit,
        })
    }
}

pub(crate) fn default_database_url() -> Result<String, CafleetError> {
    let home = home_dir().map_err(|cause| {
        CafleetError::App(format!(
            "{}; cannot derive the default database path",
            cause.message()
        ))
    })?;
    Ok(format!(
        "sqlite:///{}/.local/share/cafleet/cafleet_{}.db",
        home.display(),
        crate::cli::VERSION
    ))
}

#[cfg(test)]
impl Settings {
    pub(crate) fn for_database(path: &std::path::Path) -> Settings {
        Settings::new(Some(format!("sqlite:///{}", path.display()))).unwrap()
    }
}

pub(crate) type EnvLookup<'a> = &'a dyn Fn(&str) -> Result<Option<String>, CafleetError>;

pub(crate) fn process_env(name: &str) -> Result<Option<String>, CafleetError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(CafleetError::App(format!("{name} is not valid UTF-8")))
        }
    }
}

/// opencode discovers skills only at fixed paths, so the skills base ignores
/// every variable.
pub(crate) fn opencode_skills_base(home: &Path) -> PathBuf {
    home.join(".config/opencode")
}

pub(crate) fn home_dir() -> Result<PathBuf, CafleetError> {
    process_env("HOME")?
        .map(PathBuf::from)
        .ok_or_else(|| CafleetError::App("HOME is not set".into()))
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
    match env(var)? {
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
    fn settings_select_the_supplied_database_url_verbatim_or_the_running_versions_default() {
        let default = format!(
            "sqlite:///{}/.local/share/cafleet/cafleet_{}.db",
            std::env::var("HOME").unwrap(),
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            !default.contains('~'),
            "the default URL expands the home directory at startup"
        );
        for (name, settings, url, explicit) in [
            (
                "omitted URL",
                Settings::new(None).unwrap(),
                default.clone(),
                false,
            ),
            (
                "supplied URL without ~ expansion",
                Settings::new(Some("sqlite:///~/custom/path.db".to_string())).unwrap(),
                "sqlite:///~/custom/path.db".to_owned(),
                true,
            ),
            (
                "supplied absolute URL",
                Settings::new(Some("sqlite:///tmp/fleet.db".to_string())).unwrap(),
                "sqlite:///tmp/fleet.db".to_owned(),
                true,
            ),
            (
                "supplied URL naming the default database",
                Settings::new(Some(default_database_url().unwrap())).unwrap(),
                default.clone(),
                true,
            ),
            (
                "database path supplied through for_database",
                Settings::for_database(Path::new("/tmp/fleet.db")),
                "sqlite:////tmp/fleet.db".to_owned(),
                true,
            ),
        ] {
            assert_eq!(settings.database_url, url, "{name}");
            assert_eq!(settings.database_url_explicit, explicit, "{name}");
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

    #[test]
    fn require_absolute_rejects_non_absolute_values() {
        require_absolute("HERMES_HOME", "/srv/hermes").unwrap();
        for value in ["relative", "", "./hermes", "~/hermes"] {
            assert_eq!(
                require_absolute("HERMES_HOME", value)
                    .unwrap_err()
                    .message(),
                format!("HERMES_HOME must be an absolute path (got '{value}')")
            );
        }
    }
}
