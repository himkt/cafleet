use std::path::{Path, PathBuf};

use crate::error::CafleetError;

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
    use std::path::{Path, PathBuf};

    use super::*;

    #[test]
    fn opencode_skills_base_is_pinned_to_the_fixed_discovery_path() {
        assert_eq!(
            opencode_skills_base(Path::new("/home/tester")),
            PathBuf::from("/home/tester/.config/opencode")
        );
    }
}
