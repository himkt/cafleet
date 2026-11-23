use crate::coding_agent::CodingAgent;
use crate::config::EnvLookup;
use crate::config::opencode_skills_base;
use crate::error::CafleetError;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub(crate) struct AgentPaths {
    skills_dir: PathBuf,
    presets: Vec<(String, PathBuf)>,
    extension: Option<FileExtension>,
}

/// A backend whose extension is a set of embedded files installed beside one executable record.
struct FileExtension {
    prefix: &'static str,
    dir: PathBuf,
    record: &'static str,
}

pub(crate) const OPENCODE_EXTENSION_PREFIX: &str = "opencode/";
pub(crate) const OPENCODE_EXECUTABLE_RECORD: &str = "cafleet.executable";
pub(crate) const PI_EXTENSION_PREFIX: &str = "pi/";
pub(crate) const PI_EXECUTABLE_RECORD: &str = "cafleet.executable";

pub(crate) fn agent_paths(
    env: EnvLookup<'_>,
    home: &Path,
    agent: CodingAgent,
) -> Result<AgentPaths, CafleetError> {
    let base = agent.config_dir(env, home)?;
    let skills_dir = skills_dir(agent, &base, home);
    let presets = match agent {
        CodingAgent::Claude => vec![],
        CodingAgent::Codex => vec![(
            "codex/cafleet.rules".into(),
            base.join("rules/cafleet.rules"),
        )],
        CodingAgent::Opencode => agent
            .policy_choices()
            .iter()
            .map(|mode| {
                let target = agent
                    .required_preset(&base, mode)
                    .expect("OpenCode requires a preset for every launch policy");
                (format!("opencode/{mode}.md"), target)
            })
            .collect(),
        CodingAgent::Hermes | CodingAgent::Pi => vec![],
    };
    let extension = match agent {
        CodingAgent::Opencode => Some(FileExtension {
            prefix: OPENCODE_EXTENSION_PREFIX,
            dir: opencode_plugins_dir(&base),
            record: OPENCODE_EXECUTABLE_RECORD,
        }),
        CodingAgent::Pi => Some(FileExtension {
            prefix: PI_EXTENSION_PREFIX,
            dir: pi_extensions_dir(&base),
            record: PI_EXECUTABLE_RECORD,
        }),
        CodingAgent::Claude | CodingAgent::Codex | CodingAgent::Hermes => None,
    };
    Ok(AgentPaths {
        skills_dir,
        presets,
        extension,
    })
}

/// Where setup installs `agent`'s skills, given its resolved configuration `base`.
pub(crate) fn skills_dir(agent: CodingAgent, base: &Path, home: &Path) -> PathBuf {
    match agent {
        CodingAgent::Opencode => opencode_skills_base(home).join("skills"),
        _ => base.join("skills"),
    }
}

pub(crate) fn opencode_plugins_dir(preset_base: &Path) -> PathBuf {
    preset_base.join("plugins")
}

pub(crate) fn pi_extensions_dir(pi_base: &Path) -> PathBuf {
    pi_base.join("extensions")
}

/// Every embedded extension file under `prefix`, with its path relative to the extension root.
pub(crate) fn embedded_extension(prefix: &str) -> Vec<(&'static str, &'static [u8])> {
    let files = EXTENSIONS
        .iter()
        .filter_map(|(path, bytes)| path.strip_prefix(prefix).map(|rest| (rest, *bytes)))
        .collect::<Vec<_>>();
    assert!(!files.is_empty(), "the {prefix} extension is embedded");
    files
}

pub(crate) fn install_agent(
    agent: CodingAgent,
    paths: &AgentPaths,
    out: &mut dyn Write,
) -> Result<(), CafleetError> {
    let version = crate::cli::VERSION;
    let mut report = |line: String| crate::cli::output_result(writeln!(out, "{line}"));
    let fail = |error: std::io::Error| {
        CafleetError::App(format!(
            "failed to install skills into {}: {error}",
            paths.skills_dir.display()
        ))
    };
    let roots = SKILLS
        .iter()
        .filter_map(|(path, _)| {
            path.strip_suffix("/SKILL.md")
                .filter(|root| !root.contains('/'))
        })
        .collect::<Vec<_>>();
    assert!(
        !roots.is_empty(),
        "embedded skills have top-level manifests"
    );
    for skill in &roots {
        let prefix = format!("{skill}/");
        remove_existing_target(&paths.skills_dir.join(skill)).map_err(fail)?;
        for (path, bytes) in SKILLS.iter().filter(|(path, _)| path.starts_with(&prefix)) {
            let target = paths.skills_dir.join(path);
            std::fs::create_dir_all(target.parent().expect("skill file has a parent"))
                .map_err(fail)?;
            std::fs::write(target, bytes).map_err(fail)?;
        }
    }
    report(format!(
        "{}: installed {} (v{version}) -> {}",
        agent.name(),
        roots.join(", "),
        paths.skills_dir.display()
    ))?;
    for (source, target) in &paths.presets {
        let fail = |error: std::io::Error| {
            CafleetError::App(format!(
                "failed to install preset into {}: {error}",
                target.display()
            ))
        };
        let bytes = preset(source).expect("embedded preset exists");
        std::fs::create_dir_all(target.parent().expect("preset has a parent")).map_err(fail)?;
        remove_existing_target(target).map_err(fail)?;
        std::fs::write(target, bytes).map_err(fail)?;
        report(format!(
            "{}: installed preset (v{version}) -> {}",
            agent.name(),
            target.display()
        ))?;
    }
    if let Some(extension) = &paths.extension {
        for target in install_file_extension(extension)? {
            report(format!(
                "{}: installed extension (v{version}) -> {}",
                agent.name(),
                target.display()
            ))?;
        }
    }
    Ok(())
}

fn extension_failure(target: &Path, cause: impl std::fmt::Display) -> CafleetError {
    CafleetError::App(format!(
        "failed to install extension into {}: {cause}",
        target.display()
    ))
}

/// Writes each embedded extension file, then the executable record, and returns the files.
fn install_file_extension(extension: &FileExtension) -> Result<Vec<PathBuf>, CafleetError> {
    let mut installed = Vec::new();
    for (relative, bytes) in embedded_extension(extension.prefix) {
        let target = extension.dir.join(relative);
        let fail = |error: std::io::Error| extension_failure(&target, error);
        std::fs::create_dir_all(target.parent().expect("extension file has a parent"))
            .map_err(fail)?;
        remove_existing_target(&target).map_err(fail)?;
        std::fs::write(&target, bytes).map_err(fail)?;
        installed.push(target);
    }
    write_executable_record(&extension.dir.join(extension.record))?;
    Ok(installed)
}

fn write_executable_record(record: &Path) -> Result<(), CafleetError> {
    let running = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            extension_failure(
                record,
                format!("cannot resolve the running cafleet: {error}"),
            )
        })?;
    let fail = |error: std::io::Error| extension_failure(record, error);
    remove_existing_target(record).map_err(fail)?;
    std::fs::write(record, format!("{}\n", running.display())).map_err(fail)
}

/// Why an installed executable record is unusable, or `None` when it names an executable file.
pub(crate) fn executable_record_problem(record: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let text = match std::fs::read_to_string(record) {
        Ok(text) => text,
        Err(error) => return Some(error.to_string()),
    };
    let target = text.trim();
    if target.is_empty() {
        return Some("the record is empty".into());
    }
    match std::fs::metadata(target) {
        Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => None,
        Ok(_) => Some(format!("{target} is not an executable file")),
        Err(error) => Some(format!("{target}: {error}")),
    }
}

/// Every embedded skill file at its install path under `skills_dir`.
pub(crate) fn skill_files(skills_dir: &Path) -> Vec<(PathBuf, &'static [u8])> {
    SKILLS
        .iter()
        .map(|(relative, bytes)| (skills_dir.join(relative), *bytes))
        .collect()
}

/// Why an installed copy of an embedded file is unusable, or `None` when it equals the embedded
/// bytes.
pub(crate) fn installed_file_problem(target: &Path, embedded: &[u8]) -> Option<String> {
    match std::fs::read(target) {
        Ok(bytes) if bytes == embedded => None,
        Ok(_) => Some("differs from the embedded copy".into()),
        Err(error) => Some(error.to_string()),
    }
}

fn remove_existing_target(target: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

include!(concat!(env!("OUT_DIR"), "/embedded_data.rs"));

pub(crate) fn preset(path: &str) -> Option<&'static [u8]> {
    PRESETS
        .iter()
        .find(|(entry, _)| *entry == path)
        .map(|(_, bytes)| *bytes)
}

#[cfg(test)]
mod tests {
    use crate::assets::*;
    use crate::cli::VERSION;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    fn expected_presets(agent: CodingAgent, base: &Path) -> Vec<(&'static str, PathBuf)> {
        match agent {
            CodingAgent::Claude | CodingAgent::Hermes | CodingAgent::Pi => vec![],
            CodingAgent::Codex => vec![("codex/cafleet.rules", base.join("rules/cafleet.rules"))],
            CodingAgent::Opencode => vec![
                ("opencode/cafleet.md", base.join("agents/cafleet.md")),
                (
                    "opencode/cafleet-ask.md",
                    base.join("agents/cafleet-ask.md"),
                ),
            ],
        }
    }

    fn extension_targets(agent: CodingAgent, base: &Path) -> Vec<(&'static [u8], PathBuf)> {
        let (prefix, root) = match agent {
            CodingAgent::Claude | CodingAgent::Codex | CodingAgent::Hermes => return vec![],
            CodingAgent::Opencode => (OPENCODE_EXTENSION_PREFIX, base.join("plugins")),
            CodingAgent::Pi => ("pi/", base.join("extensions")),
        };
        embedded_extension(prefix)
            .into_iter()
            .map(|(relative, bytes)| (bytes, root.join(relative)))
            .collect()
    }

    fn extension_record(agent: CodingAgent, base: &Path) -> Option<PathBuf> {
        match agent {
            CodingAgent::Claude | CodingAgent::Codex | CodingAgent::Hermes => None,
            CodingAgent::Opencode => Some(base.join("plugins/cafleet.executable")),
            CodingAgent::Pi => Some(base.join("extensions/cafleet.executable")),
        }
    }

    fn extension_line(agent: CodingAgent, target: &Path) -> String {
        format!(
            "{}: installed extension (v{VERSION}) -> {}",
            agent.name(),
            target.display()
        )
    }

    fn assert_executable_record(record: &Path) {
        let running = std::env::current_exe().unwrap().canonicalize().unwrap();
        assert!(running.is_absolute());
        assert_ne!(
            std::fs::metadata(&running).unwrap().permissions().mode() & 0o111,
            0
        );
        assert_eq!(
            std::fs::read_to_string(record).unwrap(),
            format!("{}\n", running.display()),
            "{}",
            record.display()
        );
    }

    fn assert_embedded_extensions(targets: &[(&'static [u8], PathBuf)]) {
        for (embedded, target) in targets {
            assert_eq!(
                std::fs::read(target).unwrap(),
                *embedded,
                "{}",
                target.display()
            );
        }
    }

    fn expected_skills_dir(agent: CodingAgent, base: &Path, home: &Path) -> PathBuf {
        match agent {
            CodingAgent::Opencode => home.join(".config/opencode/skills"),
            _ => base.join("skills"),
        }
    }

    fn output_lines(output: Vec<u8>) -> Vec<String> {
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths() {
        let home = tempfile::tempdir().unwrap();
        let external = home.path().join("operator");
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("keep"), "operator bytes").unwrap();
        for agent in CodingAgent::all() {
            let custom = home.path().join(format!("custom-{}", agent.name()));
            let paths = agent_paths(
                &|name| Ok((name == agent.config_variable()).then(|| custom.display().to_string())),
                home.path(),
                *agent,
            )
            .unwrap();
            let skills_dir = expected_skills_dir(*agent, &custom, home.path());
            let presets = expected_presets(*agent, &custom);
            let extensions = extension_targets(*agent, &custom);
            std::fs::create_dir_all(&skills_dir).unwrap();
            symlink(&external, skills_dir.join("cafleet")).unwrap();
            let owned_targets = presets
                .iter()
                .map(|(_, target)| target)
                .chain(extensions.iter().map(|(_, target)| target));
            for target in owned_targets {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                symlink(external.join("keep"), target).unwrap();
            }
            for _ in 0..2 {
                install_agent(*agent, &paths, &mut Vec::<u8>::new()).unwrap();
                for (relative, bytes) in SKILLS {
                    assert_eq!(std::fs::read(skills_dir.join(relative)).unwrap(), *bytes);
                }
                for (source, target) in &presets {
                    assert_eq!(
                        std::fs::read(target).unwrap(),
                        preset(source).unwrap(),
                        "{}",
                        target.display()
                    );
                }
                assert_embedded_extensions(&extensions);
                if let Some(record) = extension_record(*agent, &custom) {
                    assert_executable_record(&record);
                }
                assert_eq!(
                    std::fs::read(external.join("keep")).unwrap(),
                    b"operator bytes"
                );
                assert_eq!(
                    std::fs::read_dir(&external).unwrap().count(),
                    1,
                    "a replaced symlink's referent stays intact"
                );
            }
            if *agent == CodingAgent::Opencode {
                assert!(!custom.join("skills").exists());
            }
        }
    }

    #[test]
    fn installation_prints_skills_then_presets_then_extensions() {
        let home = tempfile::tempdir().unwrap();
        for agent in CodingAgent::all() {
            let custom = home.path().join(format!("custom-{}", agent.name()));
            let paths = agent_paths(
                &|name| Ok((name == agent.config_variable()).then(|| custom.display().to_string())),
                home.path(),
                *agent,
            )
            .unwrap();
            let mut output = Vec::new();
            install_agent(*agent, &paths, &mut output).unwrap();
            let lines = output_lines(output);
            let expected = expected_presets(*agent, &custom)
                .iter()
                .map(|(_, target)| {
                    format!(
                        "{}: installed preset (v{VERSION}) -> {}",
                        agent.name(),
                        target.display()
                    )
                })
                .chain(
                    extension_targets(*agent, &custom)
                        .iter()
                        .map(|(_, target)| extension_line(*agent, target)),
                )
                .collect::<Vec<_>>();
            let skills_suffix = format!(
                " (v{VERSION}) -> {}",
                expected_skills_dir(*agent, &custom, home.path()).display()
            );
            assert!(
                lines[0].starts_with(&format!("{}: installed ", agent.name()))
                    && !lines[0].contains("installed preset")
                    && !lines[0].contains("installed extension")
                    && lines[0].ends_with(&skills_suffix),
                "skills line first: {lines:?}"
            );
            assert_eq!(lines[1..], expected, "{}", agent.name());
        }
    }

    #[test]
    fn opencode_preset_failure_fails_installation() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join("preset-base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("agents"), "blocked parent").unwrap();
        let paths = agent_paths(
            &|name| Ok((name == "OPENCODE_CONFIG_DIR").then(|| base.display().to_string())),
            home.path(),
            CodingAgent::Opencode,
        )
        .unwrap();
        let mut output = Vec::new();
        let failure = install_agent(CodingAgent::Opencode, &paths, &mut output)
            .expect_err("a blocked preset target fails installation");
        assert!(
            failure.message().starts_with(&format!(
                "failed to install preset into {}: ",
                base.join("agents/cafleet.md").display()
            )),
            "{}",
            failure.message()
        );
        let lines = output_lines(output);
        assert_eq!(lines.len(), 1, "only the completed skills line: {lines:?}");
        assert!(
            lines[0].starts_with("opencode: installed ")
                && lines[0].ends_with(&format!(
                    "-> {}",
                    home.path().join(".config/opencode/skills").display()
                )),
            "{lines:?}"
        );
        assert!(!base.join("agents/cafleet-ask.md").exists());
        assert!(
            !base.join("plugins").exists(),
            "extensions install after presets"
        );
    }

    #[test]
    fn opencode_extension_failure_fails_installation() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join("preset-base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("plugins"), "blocked parent").unwrap();
        let paths = agent_paths(
            &|name| Ok((name == "OPENCODE_CONFIG_DIR").then(|| base.display().to_string())),
            home.path(),
            CodingAgent::Opencode,
        )
        .unwrap();
        let mut output = Vec::new();
        let failure = install_agent(CodingAgent::Opencode, &paths, &mut output)
            .expect_err("a blocked extension target fails installation");
        assert_eq!(failure.exit_code(), 1);
        assert!(
            failure.message().starts_with(&format!(
                "failed to install extension into {}: ",
                base.join("plugins")
                    .join(embedded_extension(OPENCODE_EXTENSION_PREFIX)[0].0)
                    .display()
            )),
            "{}",
            failure.message()
        );
        let lines = output_lines(output);
        assert_eq!(
            lines[1..],
            [
                format!(
                    "opencode: installed preset (v{VERSION}) -> {}",
                    base.join("agents/cafleet.md").display()
                ),
                format!(
                    "opencode: installed preset (v{VERSION}) -> {}",
                    base.join("agents/cafleet-ask.md").display()
                ),
            ],
            "no extension line for a failed extension"
        );
        assert_eq!(
            std::fs::read(base.join("plugins")).unwrap(),
            b"blocked parent"
        );
    }

    #[test]
    fn opencode_default_installation_uses_the_native_config_directory_for_every_asset() {
        let home = tempfile::tempdir().unwrap();
        let paths = agent_paths(&|_| Ok(None), home.path(), CodingAgent::Opencode).unwrap();
        let base = home.path().join(".config/opencode");
        install_agent(CodingAgent::Opencode, &paths, &mut Vec::<u8>::new()).unwrap();
        for (source, target) in expected_presets(CodingAgent::Opencode, &base) {
            assert_eq!(
                std::fs::read(&target).unwrap(),
                preset(source).unwrap(),
                "{}",
                target.display()
            );
        }
        assert_embedded_extensions(&extension_targets(CodingAgent::Opencode, &base));
        assert_executable_record(&base.join("plugins/cafleet.executable"));
    }

    mod pi {
        use crate::assets::tests::*;

        struct Installed {
            result: Result<(), CafleetError>,
            lines: Vec<String>,
        }

        fn install(home: &Path, base: Option<&Path>) -> Installed {
            let paths = agent_paths(
                &|name| {
                    Ok(base
                        .filter(|_| name == "PI_CODING_AGENT_DIR")
                        .map(|base| base.display().to_string()))
                },
                home,
                CodingAgent::Pi,
            )
            .unwrap();
            let mut output = Vec::new();
            let result = install_agent(CodingAgent::Pi, &paths, &mut output);
            Installed {
                result,
                lines: output_lines(output),
            }
        }

        fn extension_target(base: &Path) -> PathBuf {
            base.join("extensions/cafleet.ts")
        }

        fn skills_line_matches(line: &str, base: &Path) -> bool {
            line.starts_with("pi: installed ")
                && line.ends_with(&format!(
                    " (v{VERSION}) -> {}",
                    base.join("skills").display()
                ))
        }

        #[test]
        fn pi_default_installation_writes_skills_extension_and_record_under_the_pi_agent_directory()
        {
            let home = tempfile::tempdir().unwrap();
            install(home.path(), None).result.unwrap();
            let base = home.path().join(".pi/agent");
            for (relative, bytes) in SKILLS {
                assert_eq!(
                    std::fs::read(base.join("skills").join(relative)).unwrap(),
                    *bytes
                );
            }
            assert_embedded_extensions(&extension_targets(CodingAgent::Pi, &base));
            assert_eq!(
                std::fs::read(extension_target(&base)).unwrap(),
                EXTENSIONS
                    .iter()
                    .find(|(path, _)| *path == "pi/cafleet.ts")
                    .unwrap()
                    .1
            );
            assert_executable_record(&base.join("extensions/cafleet.executable"));
        }

        #[test]
        fn pi_installation_prints_the_skills_line_then_the_extension_line() {
            let home = tempfile::tempdir().unwrap();
            let base = home.path().join("pi agent");
            let installed = install(home.path(), Some(&base));
            installed.result.unwrap();
            assert_eq!(installed.lines.len(), 2, "{:?}", installed.lines);
            assert!(
                skills_line_matches(&installed.lines[0], &base),
                "{:?}",
                installed.lines
            );
            assert_eq!(
                installed.lines[1],
                format!(
                    "pi: installed extension (v{VERSION}) -> {}",
                    extension_target(&base).display()
                )
            );
        }

        #[test]
        fn pi_rerun_prints_the_same_lines_and_leaves_identical_bytes_and_one_record() {
            let home = tempfile::tempdir().unwrap();
            let base = home.path().join("pi agent");
            let first = install(home.path(), Some(&base));
            first.result.unwrap();
            let extension = std::fs::read(extension_target(&base)).unwrap();
            let record = std::fs::read(base.join("extensions/cafleet.executable")).unwrap();
            let second = install(home.path(), Some(&base));
            second.result.unwrap();
            assert_eq!(second.lines, first.lines);
            assert_eq!(std::fs::read(extension_target(&base)).unwrap(), extension);
            assert_eq!(
                std::fs::read(base.join("extensions/cafleet.executable")).unwrap(),
                record
            );
        }

        #[test]
        fn pi_installation_keeps_operator_extensions_beside_the_cafleet_extension() {
            let home = tempfile::tempdir().unwrap();
            let base = home.path().join("pi agent");
            let extensions = base.join("extensions");
            std::fs::create_dir_all(extensions.join("tools")).unwrap();
            std::fs::write(extensions.join("operator.ts"), "operator extension").unwrap();
            std::fs::write(extensions.join("tools/index.ts"), "operator package").unwrap();
            install(home.path(), Some(&base)).result.unwrap();
            assert_eq!(
                std::fs::read(extensions.join("operator.ts")).unwrap(),
                b"operator extension"
            );
            assert_eq!(
                std::fs::read(extensions.join("tools/index.ts")).unwrap(),
                b"operator package"
            );
        }

        #[test]
        fn pi_extension_failure_fails_installation() {
            let home = tempfile::tempdir().unwrap();
            let base = home.path().join("pi agent");
            std::fs::create_dir_all(&base).unwrap();
            std::fs::write(base.join("extensions"), "blocked parent").unwrap();
            let installed = install(home.path(), Some(&base));
            let failure = installed
                .result
                .expect_err("a blocked extension target fails installation");
            assert!(matches!(failure, CafleetError::App(_)));
            assert_eq!(failure.exit_code(), 1);
            assert!(
                failure.message().starts_with(&format!(
                    "failed to install extension into {}: ",
                    extension_target(&base).display()
                )),
                "{}",
                failure.message()
            );
            assert_eq!(installed.lines.len(), 1, "{:?}", installed.lines);
            assert!(skills_line_matches(&installed.lines[0], &base));
            assert_eq!(
                std::fs::read(base.join("extensions")).unwrap(),
                b"blocked parent"
            );
        }
    }

    mod hermes {
        use crate::assets::tests::*;

        #[test]
        fn hermes_installation_writes_only_the_skills_and_prints_only_the_skills_line() {
            let home = tempfile::tempdir().unwrap();
            let paths = agent_paths(&|_| Ok(None), home.path(), CodingAgent::Hermes).unwrap();
            let base = home.path().join(".hermes");
            for round in 0..2 {
                let mut output = Vec::new();
                install_agent(CodingAgent::Hermes, &paths, &mut output).unwrap();
                let lines = output_lines(output);
                assert_eq!(lines.len(), 1, "round {round}: {lines:?}");
                assert!(
                    lines[0].starts_with("hermes: installed ")
                        && lines[0].ends_with(&format!(
                            " (v{VERSION}) -> {}",
                            base.join("skills").display()
                        )),
                    "round {round}: {lines:?}"
                );
                for (relative, bytes) in SKILLS {
                    assert_eq!(
                        std::fs::read(base.join("skills").join(relative)).unwrap(),
                        *bytes,
                        "round {round}"
                    );
                }
                let entries = std::fs::read_dir(&base)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .collect::<Vec<_>>();
                assert_eq!(
                    entries,
                    ["skills"],
                    "round {round}: only the skills directory"
                );
            }
        }
    }
}
