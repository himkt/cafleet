//! Development-only runtime documentation closure and bootstrap maintenance.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

type Files = BTreeMap<PathBuf, String>;

#[derive(Default)]
struct Markdown {
    links: Vec<String>,
    anchors: BTreeSet<String>,
    obsolete: Vec<String>,
}
fn slug(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}
fn parse(text: &str) -> Markdown {
    let mut result = Markdown::default();
    let mut broken = |link: pulldown_cmark::BrokenLink<'_>| {
        if matches!(link.link_type, LinkType::Reference | LinkType::Collapsed) {
            Some((
                format!("unresolved-reference:{}", link.reference).into(),
                "".into(),
            ))
        } else {
            None
        }
    };
    let parser = Parser::new_with_broken_link_callback(
        text,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_HEADING_ATTRIBUTES,
        Some(&mut broken),
    );
    let mut heading: Option<(Option<String>, String)> = None;
    let mut duplicate = BTreeMap::<String, usize>::new();
    let mut code = false;
    let mut link_depth = 0;
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code = true,
            Event::End(TagEnd::CodeBlock) => code = false,
            Event::Start(Tag::Heading { id, .. }) => {
                heading = Some((id.map(|id| id.to_string()), String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                let (id, title) = heading.take().unwrap();
                let base = id.unwrap_or_else(|| slug(&title));
                let count = duplicate.entry(base.clone()).or_default();
                result.anchors.insert(if *count == 0 {
                    base
                } else {
                    format!("{base}-{count}")
                });
                *count += 1;
            }
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                result.links.push(dest_url.to_string());
                link_depth += 1;
            }
            Event::End(TagEnd::Link | TagEnd::Image) => link_depth -= 1,
            Event::Code(value) => {
                if let Some((_, title)) = &mut heading {
                    title.push_str(&value);
                }
                // Link labels and fenced examples are not instructions to open a file.
                if !code
                    && link_depth == 0
                    && (value.starts_with("docs/docs/") || value.as_ref() == "SPEC.md")
                {
                    result.obsolete.push(value.to_string());
                }
            }
            Event::Text(value) if !code => {
                if let Some((_, title)) = &mut heading {
                    title.push_str(&value);
                }
            }
            _ => {}
        }
    }
    result
}
fn normalize(path: &Path) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err(format!("reference escapes root: {}", path.display()));
                }
            }
            _ => return Err(format!("absolute reference: {}", path.display())),
        }
    }
    Ok(out)
}
fn local(source: &Path, url: &str) -> Result<Option<(PathBuf, String)>, String> {
    if url.starts_with("unresolved-reference:") {
        return Err(url.into());
    }
    if url.starts_with("https:") || url.starts_with("http:") || url.starts_with("mailto:") {
        return Ok(None);
    }
    if url.contains('<') || url.contains('>') || url.contains('{') {
        return Err(format!("unresolved symbolic reference: {url}"));
    }
    let (name, anchor) = url.split_once('#').unwrap_or((url, ""));
    let path = if name.is_empty() {
        source.to_path_buf()
    } else {
        normalize(&source.parent().unwrap().join(name))?
    };
    Ok(Some((path, anchor.to_string())))
}
fn validate(files: &Files) -> Result<(), String> {
    for (path, content) in files {
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let doc = parse(content);
        if !doc.obsolete.is_empty() {
            return Err(format!(
                "{}: unconverted inline reference {:?}",
                path.display(),
                doc.obsolete
            ));
        }
        for url in doc.links {
            if let Some((target, anchor)) = local(path, &url)? {
                let target_content = files.get(&target).ok_or_else(|| {
                    format!("{} -> {url}: missing {}", path.display(), target.display())
                })?;
                if !anchor.is_empty() && !parse(target_content).anchors.contains(&anchor) {
                    return Err(format!("{} -> {url}: missing anchor", path.display()));
                }
            }
        }
    }
    Ok(())
}
fn collect(root: &Path, dir: &Path, files: &mut Files) {
    for item in std::fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.file_name().unwrap().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            files.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned(),
            );
        }
    }
}
fn template(home: &Path, base: &Path, agent: &str) -> String {
    let paths = crate::assets::agent_paths(&|_| None, home, agent).unwrap();
    let escape = |p: &Path| {
        p.display()
            .to_string()
            .replace('{', "{{")
            .replace('}', "}}")
    };
    include_str!("../../tests/fixtures/monitor-bootstrap.txt")
        .replace("@SKILL@", &escape(&paths.skills_dir.join("cafleet")))
        .replace("@BASE@", &escape(base))
}
fn bootstrap_block(text: &str, agent: &str) -> String {
    let begin = format!("<!-- BEGIN BOOTSTRAP {agent} -->");
    let end = format!("<!-- END BOOTSTRAP {agent} -->");
    assert_eq!(text.matches(&begin).count(), 1);
    assert_eq!(text.matches(&end).count(), 1);
    let start = text.find(&begin).unwrap() + begin.len();
    let finish = text.find(&end).unwrap();
    assert!(start < finish && !text[start..finish].contains("<!-- BEGIN BOOTSTRAP"));
    let prompt = template(
        Path::new("/home/cafleet-demo"),
        Path::new("/home/cafleet-demo/work/demo"),
        agent,
    );
    let mut out = text.to_string();
    out.replace_range(start..finish, &format!("\n\n```text\n{prompt}```\n\n"));
    out
}
pub(super) fn prepare(root: &Path, updates: &mut BTreeMap<PathBuf, (String, String)>) {
    for (file, agents) in [
        ("quickstart.md", &["claude"][..]),
        ("concepts/coding-agents.md", &["codex", "opencode"][..]),
    ] {
        let path = root.join("docs/docs").join(file);
        let original = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            original.matches("<!-- BEGIN BOOTSTRAP ").count(),
            agents.len()
        );
        assert_eq!(
            original.matches("<!-- END BOOTSTRAP ").count(),
            agents.len()
        );
        let mut rendered = original.clone();
        for agent in agents {
            rendered = bootstrap_block(&rendered, agent);
        }
        updates.insert(path, (original, rendered));
    }
    let files = crate::embedded::SKILLS
        .iter()
        .map(|(path, bytes)| {
            (
                PathBuf::from(path),
                std::str::from_utf8(bytes).unwrap().to_owned(),
            )
        })
        .collect();
    validate(&files).unwrap();
}

pub(super) fn installed(root: &Path) {
    // A single parser fixture covers ordinary/reference links, images, fences and two representative failures.
    let source = "# Title\n[ok](target.md#anchor) [ref][r] ![image](image.png)\n\n[r]: target.md#anchor\n```text\n[ignored](missing.md)\n```\n";
    let mut tiny = Files::from([
        (PathBuf::from("source.md"), source.into()),
        (PathBuf::from("target.md"), "# Anchor\n".into()),
        (PathBuf::from("image.png"), String::new()),
    ]);
    validate(&tiny).unwrap();
    tiny.insert(PathBuf::from("target.md"), "# Wrong\n".into());
    assert!(validate(&tiny).is_err());
    tiny.insert(PathBuf::from("target.md"), "# Anchor\n`SPEC.md`\n".into());
    assert!(validate(&tiny).is_err());
    for agent in crate::assets::TARGET_AGENTS {
        let fixture = tempfile::Builder::new()
            .prefix(".runtime-docs-")
            .tempdir_in(root)
            .unwrap();
        let home = fixture.path().join("home");
        let base = fixture.path().join("work");
        std::fs::create_dir_all(&base).unwrap();
        let paths = crate::assets::agent_paths(&|_| None, &home, agent).unwrap();
        let mut conn = crate::db::connect(&format!(
            "sqlite:///{}",
            fixture.path().join("schema.db").display()
        ))
        .unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let hooks = crate::assets::InstallHooks {
            lock_mode: crate::assets::LockMode::Wait,
            checkpoint: &|_| Ok(()),
        };
        let plan = crate::assets::prepare_install(&paths, agent, super::VERSION).unwrap();
        crate::assets::execute_install(&mut conn, &plan, &hooks).unwrap();
        let mut files = Files::new();
        for name in ["cafleet", "cafleet-design-doc"] {
            collect(&paths.skills_dir, &paths.skills_dir.join(name), &mut files);
        }
        validate(&files).unwrap();
        if let Some((_, preset)) = &paths.preset {
            assert!(preset.is_file());
        }
        let prompt = crate::spawn_prompt::substitute_spawn_placeholders(
            &template(&home, &base, agent),
            43,
            190,
            187,
            agent,
        )
        .unwrap();
        for label in [
            "FLEET ID: 43".to_string(),
            "DIRECTOR MEMBER ID: 187".into(),
            "YOUR MEMBER ID: 190".into(),
            format!("CODING AGENT: {agent}"),
        ] {
            assert!(prompt.contains(&label));
        }
        for file in [
            "roles/monitor.md",
            "reference/coding-agent-overlays.md",
            "SKILL.md",
            "reference/base-dir.md",
        ] {
            let path = paths.skills_dir.join("cafleet").join(file);
            assert!(path.is_file());
            assert!(prompt.contains(path.to_str().unwrap()));
        }
        println!("{agent}: installed closure and bootstrap identity/role paths verified");
    }
}
