mod structural_contracts {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Component, Path, PathBuf};
    use std::sync::{Arc, LazyLock};

    const BACKENDS: &str = "skills/cafleet/reference/coding-agents.md";
    const CORE: &str = "skills/cafleet/SKILL.md";
    const DIRECTOR: &str = "skills/cafleet/roles/director.md";
    const MONITOR: &str = "skills/cafleet/roles/monitor.md";
    const SUPERVISION: &str = "skills/cafleet/reference/supervision.md";
    const ROUTING: &str = "skills/cafleet/reference/prompt-routing.md";
    const CREATE: &str = "skills/cafleet-design-doc/create/create.md";
    const EXECUTE: &str = "skills/cafleet-design-doc/execute/execute.md";
    const INTERVIEW: &str = "skills/cafleet-design-doc/interview/interview.md";
    const DRAFTER: &str = "skills/cafleet-design-doc/create/roles/drafter.md";
    const COORDINATION: &str = "skills/cafleet-design-doc/reference/coordination.md";
    const MANUAL: &str = "docs/docs/how-to/mixed-backend-team.md";
    const AUTHOR: &str = ".claude/skills/skill-author/SKILL.md";
    const BASH: &str = ".claude/rules/bash-tool.md";
    const RUNTIME: [&str; 7] = [
        "decision_surface",
        "permission_flags",
        "bg_run",
        "bg_stop",
        "pane_title",
        "skill_loader",
        "effort_levels",
    ];
    const DEFAULTS: [&str; 2] = ["reviewer_model", "monitor_model"];
    const IDENTITIES: [&str; 4] = [
        "fleet_id",
        "member_id",
        "director_member_id",
        "coding_agent",
    ];

    static LINKS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\[[^\]\n]+\]\(([^)\n]+)\)").unwrap());
    static INLINE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\x60([^\x60\n]+)\x60").unwrap());
    static TOKENS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap());
    static LOCATOR: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r":\d+(-\d+)?$").unwrap());
    static INVENTORY: LazyLock<Inventory> = LazyLock::new(Inventory::load);

    struct Inventory {
        sources: BTreeMap<String, Arc<str>>,
        canonical_reads: usize,
        bytes_read: usize,
        directory_visits: usize,
    }

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    impl Inventory {
        fn load() -> Self {
            fn walk(
                path: &Path,
                paths: &mut Vec<PathBuf>,
                ancestors: &mut BTreeSet<PathBuf>,
                visits: &mut usize,
            ) {
                let canonical = path
                    .canonicalize()
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(
                    ancestors.insert(canonical.clone()),
                    "directory alias cycle at {}",
                    path.display()
                );
                *visits += 1;
                for entry in
                    std::fs::read_dir(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(&path, paths, ancestors, visits);
                    } else if path
                        .extension()
                        .is_some_and(|ext| ext == "md" || ext == "mdx" || ext == "json")
                    {
                        paths.push(path);
                    }
                }
                assert!(ancestors.remove(&canonical));
            }
            let root = root();
            let mut paths = Vec::new();
            let mut directory_visits = 0;
            for tree in ["skills", "docs/docs", ".claude/skills/clean-docs"] {
                walk(
                    &root.join(tree),
                    &mut paths,
                    &mut BTreeSet::new(),
                    &mut directory_visits,
                );
            }
            paths.retain(|path| {
                !path.starts_with(root.join(".claude/skills/clean-docs"))
                    || path.components().any(|part| part.as_os_str() == "roles")
            });
            paths.extend([root.join(AUTHOR), root.join(BASH), root.join("SPEC.md")]);
            paths.sort();
            paths.dedup();
            let mut canonical = BTreeMap::<PathBuf, Arc<str>>::new();
            let mut sources = BTreeMap::new();
            let mut bytes_read = 0;
            for path in paths {
                let physical = path
                    .canonicalize()
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let content = canonical
                    .entry(physical)
                    .or_insert_with(|| {
                        let text = std::fs::read_to_string(&path)
                            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                        bytes_read += text.len();
                        Arc::from(text)
                    })
                    .clone();
                let logical = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                assert!(sources.insert(logical, content).is_none());
            }
            Self {
                sources,
                canonical_reads: canonical.len(),
                bytes_read,
                directory_visits,
            }
        }

        fn document(&self, path: &str) -> Region<'_> {
            Region {
                path: path.to_string(),
                text: self
                    .sources
                    .get(path)
                    .unwrap_or_else(|| panic!("{path}:1: required source missing from inventory")),
                line: 1,
                section: "document".into(),
            }
        }

        fn link(&self, source: &Region<'_>, line: usize, target: &str) -> Result<(), String> {
            if target.contains("://") || target.starts_with("mailto:") {
                return Err(source.error(
                    line,
                    &format!("required local owner link is external: {target}"),
                ));
            }
            let (file, anchor) = target
                .split_once('#')
                .map_or((target, None), |(file, anchor)| (file, Some(anchor)));
            let path = if file.is_empty() {
                source.path.clone()
            } else {
                let joined = Path::new(&source.path).parent().unwrap().join(file);
                let mut logical = PathBuf::new();
                for component in joined.components() {
                    match component {
                        Component::Normal(part) => logical.push(part),
                        Component::CurDir => {}
                        Component::ParentDir if logical.pop() => {}
                        _ => {
                            return Err(source.error(
                                line,
                                &format!("required target escapes repository: {target}"),
                            ));
                        }
                    }
                }
                logical.to_str().unwrap().to_string()
            };
            if !root().join(&path).is_file() {
                return Err(
                    source.error(line, &format!("missing target {target} (resolved {path})"))
                );
            }
            if let Some(anchor) = anchor {
                let text = self.sources.get(&path).ok_or_else(|| {
                    source.error(
                        line,
                        &format!("required anchor source {path} is missing from inventory"),
                    )
                })?;
                let destination = Region {
                    path: path.clone(),
                    text,
                    line: 1,
                    section: "document".into(),
                };
                for anchor in if anchor == "<name>" {
                    vec!["claude", "codex", "opencode"]
                } else {
                    vec![anchor]
                } {
                    destination
                        .section(anchor)
                        .map_err(|e| source.error(line, &format!("target {target}: {e}")))?;
                }
            }
            Ok(())
        }

        fn require_link(&self, region: &Region<'_>, target: &str) -> Result<(), String> {
            let found = region
                .text
                .lines()
                .enumerate()
                .find_map(|(offset, line)| {
                    LINKS
                        .captures_iter(line)
                        .any(|capture| &capture[1] == target)
                        .then_some(region.line + offset)
                })
                .ok_or_else(|| {
                    region.error(region.line, &format!("required link {target} missing"))
                })?;
            self.link(region, found, target)
        }
    }

    #[derive(Clone, Debug)]
    struct Region<'a> {
        path: String,
        text: &'a str,
        line: usize,
        section: String,
    }

    fn heading_id(title: &str) -> Result<String, String> {
        if let Some((_, id)) = title.rsplit_once(" {#") {
            return id
                .strip_suffix('}')
                .filter(|id| {
                    !id.is_empty()
                        && id
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                })
                .map(str::to_string)
                .ok_or_else(|| format!("unsupported explicit heading ID: {title}"));
        }
        if title.contains(['[', ']', '<', '>', '{', '}']) {
            return Err(format!(
                "unsupported heading syntax; add an explicit {{#id}}: {title}"
            ));
        }
        Ok(title
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
            .map(|c| if c.is_whitespace() { '-' } else { c })
            .collect())
    }

    impl<'a> Region<'a> {
        fn error(&self, line: usize, detail: &str) -> String {
            format!("{}:{line}: section {}: {detail}", self.path, self.section)
        }

        fn section(&self, anchor: &str) -> Result<Region<'a>, String> {
            let mut offset = 0;
            let mut fence = false;
            let mut selected = None;
            let mut unsupported = Vec::new();
            for (index, line) in self.text.split_inclusive('\n').enumerate() {
                if line.trim_start().starts_with("~~~") || line.trim_start().starts_with("```") {
                    fence = !fence;
                }
                if !fence {
                    let depth = line.chars().take_while(|c| *c == '#').count();
                    if (1..=6).contains(&depth) && line.as_bytes().get(depth) == Some(&b' ') {
                        if let Some((begin, start_line, parent_depth)) = selected {
                            if depth <= parent_depth {
                                return Ok(Region {
                                    path: self.path.clone(),
                                    text: &self.text[begin..offset],
                                    line: start_line,
                                    section: format!("{} > {anchor}", self.section),
                                });
                            }
                        } else {
                            match heading_id(line[depth + 1..].trim()) {
                                Ok(id) if id == anchor => {
                                    selected = Some((offset, self.line + index, depth))
                                }
                                Err(error) => {
                                    unsupported.push(format!("{}: {error}", self.line + index))
                                }
                                _ => {}
                            }
                        }
                    }
                }
                offset += line.len();
            }
            if let Some((begin, line, _)) = selected {
                Ok(Region {
                    path: self.path.clone(),
                    text: &self.text[begin..],
                    line,
                    section: format!("{} > {anchor}", self.section),
                })
            } else {
                Err(self.error(
                    self.line,
                    &format!("missing anchor #{anchor}; {}", unsupported.join("; ")),
                ))
            }
        }

        fn require(&self, tokens: &[&str]) -> Result<(), String> {
            for token in tokens {
                if !self.text.contains(token) {
                    return Err(self.error(
                        self.line,
                        &format!("required interface token {token:?} missing"),
                    ));
                }
            }
            Ok(())
        }

        fn ordered(&self, commands: &[&str]) -> Result<(), String> {
            let mut remaining = self.text;
            for command in commands {
                let start = remaining.find(command).ok_or_else(|| {
                    self.error(self.line, &format!("missing next command {command:?}"))
                })?;
                remaining = &remaining[start + command.len()..];
            }
            Ok(())
        }

        fn rows(&self) -> Result<Vec<(usize, Vec<&'a str>)>, String> {
            let rows = self
                .text
                .lines()
                .enumerate()
                .skip_while(|(_, line)| !line.trim_start().starts_with('|'))
                .take_while(|(_, line)| line.trim_start().starts_with('|'))
                .map(|(index, line)| {
                    (
                        self.line + index,
                        line.trim()
                            .trim_matches('|')
                            .split('|')
                            .map(str::trim)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let Some((_, header)) = rows.first() else {
                return Err(self.error(self.line, "required table missing"));
            };
            for (line, row) in &rows {
                if row.len() != header.len() || row.iter().any(|cell| cell.is_empty()) {
                    return Err(self.error(*line, "malformed table row"));
                }
            }
            if rows.len() < 2
                || !rows[1]
                    .1
                    .iter()
                    .all(|cell| cell.chars().all(|c| c == '-' || c == ':'))
            {
                return Err(self.error(self.line, "table separator missing"));
            }
            Ok(rows)
        }

        fn bindings(&self, expected: &[&str]) -> Result<BTreeMap<String, &'a str>, String> {
            let rows = self.rows()?;
            if rows[0].1 != ["Placeholder", "Value"] {
                return Err(self.error(rows[0].0, "expected Placeholder / Value table"));
            }
            let mut assignments = BTreeMap::new();
            for (line, row) in rows.iter().skip(2) {
                let key = row[0]
                    .trim_matches('`')
                    .strip_prefix('{')
                    .and_then(|key| key.strip_suffix('}'))
                    .ok_or_else(|| self.error(*line, "malformed binding key"))?;
                if !expected.contains(&key) || assignments.insert(key.to_string(), row[1]).is_some()
                {
                    return Err(self.error(*line, &format!("unknown or duplicate binding {key}")));
                }
            }
            for key in expected {
                if !assignments.contains_key(*key) {
                    return Err(self.error(self.line, &format!("missing binding {key}")));
                }
            }
            Ok(assignments)
        }

        fn operational_tokens(&self, allowed: &[&str]) -> Result<(), String> {
            for (index, line) in self.text.lines().enumerate() {
                for token in TOKENS.captures_iter(line) {
                    if !allowed.contains(&&token[1]) {
                        return Err(self.error(
                            self.line + index,
                            &format!("unknown operational placeholder {}", &token[0]),
                        ));
                    }
                }
            }
            Ok(())
        }
    }

    fn required_backend(region: &Region<'_>) -> Result<String, String> {
        let required = region.section("required-reading")?;
        let rows = required.rows()?;
        let first = rows
            .iter()
            .skip(2)
            .filter(|(_, row)| row[0] == "1")
            .collect::<Vec<_>>();
        if first.len() != 1 || rows[2].1[0] != "1" {
            return Err(required.error(required.line, "Required reading must start with one row 1"));
        }
        let (line, cells) = first[0];
        let target = cells.iter().find_map(|cell| {
            LINKS
                .captures_iter(cell)
                .find(|capture| {
                    capture[1]
                        .split('#')
                        .next()
                        .unwrap()
                        .ends_with("coding-agents.md")
                })
                .map(|capture| capture[1].to_string())
        });
        target.ok_or_else(|| required.error(*line, "row 1 must link to coding-agents.md"))
    }

    fn prompt<'a>(region: &Region<'a>) -> Result<Region<'a>, String> {
        let opening = "```text\n";
        let mut offset = 0;
        while let Some(start) = region.text[offset..].find(opening) {
            let begin = offset + start + opening.len();
            let end = region.text[begin..]
                .find("\n```")
                .ok_or_else(|| region.error(region.line, "unclosed text example"))?
                + begin;
            if region.text[begin..end].contains("ROLE DEFINITION:") {
                return Ok(Region {
                    path: region.path.clone(),
                    text: &region.text[begin..end],
                    line: region.line
                        + region.text[..begin].bytes().filter(|c| *c == b'\n').count(),
                    section: "spawn template".into(),
                });
            }
            offset = end + 4;
        }
        Err(region.error(region.line, "required role prompt example missing"))
    }

    #[test]
    fn source_inventory_shares_content_and_preserves_logical_aliases() {
        let inventory = &*INVENTORY;
        assert!(Arc::ptr_eq(
            &inventory.sources["docs/docs/spec/cli-options.md"],
            &inventory.sources["skills/cafleet/reference/runtime/spec/cli-options.md"]
        ));
        assert!(
            inventory
                .sources
                .keys()
                .any(|path| path.starts_with(".claude/skills/clean-docs/")
                    && path.contains("/roles/"))
        );
        for path in [AUTHOR, BASH] {
            inventory.document(path);
        }
        assert!(inventory.canonical_reads < inventory.sources.len());
        eprintln!(
            "docs-inventory logical={} canonical_reads={} bytes_read={} directory_visits={}",
            inventory.sources.len(),
            inventory.canonical_reads,
            inventory.bytes_read,
            inventory.directory_visits
        );
    }

    #[test]
    fn required_reads_resolve_inside_their_own_sections() {
        let inventory = &*INVENTORY;
        for path in [
            "create/roles/drafter.md",
            "create/roles/reviewer.md",
            "execute/roles/programmer.md",
            "execute/roles/tester.md",
            "execute/roles/verifier.md",
            "execute/roles/reviewer.md",
            "interview/roles/analyzer.md",
        ] {
            inventory.document(&format!("skills/cafleet-design-doc/{path}"));
        }
        for path in inventory.sources.keys().filter(|path| {
            path.starts_with("skills/")
                || (path.starts_with(".claude/skills/clean-docs/") && path.contains("/roles/"))
        }) {
            let document = inventory.document(path);
            if !path.contains("/roles/") && document.section("required-reading").is_err() {
                continue;
            }
            let target = required_backend(&document).unwrap();
            let required = document.section("required-reading").unwrap();
            inventory.require_link(&required, &target).unwrap();
            let resolved = root()
                .join(Path::new(path).parent().unwrap())
                .join(target.split('#').next().unwrap())
                .canonicalize()
                .unwrap();
            assert_eq!(
                resolved,
                root().join(BACKENDS).canonicalize().unwrap(),
                "{path}"
            );
            if path.contains("/roles/") {
                let base = LINKS
                    .captures_iter(required.text)
                    .find(|capture| {
                        capture[1]
                            .split('#')
                            .next()
                            .unwrap()
                            .ends_with("base-dir.md")
                    })
                    .unwrap_or_else(|| {
                        panic!("{path}:{}: Required reading needs BASE", required.line)
                    });
                inventory.require_link(&required, &base[1]).unwrap();
            }
            if path.starts_with("skills/cafleet-design-doc/") && path.contains("/roles/") {
                let coordination = LINKS
                    .captures_iter(required.text)
                    .find(|capture| capture[1].ends_with("coordination.md"))
                    .unwrap_or_else(|| {
                        panic!(
                            "{path}:{}: Required reading needs coordination",
                            required.line
                        )
                    });
                inventory.require_link(&required, &coordination[1]).unwrap();
            }
        }
        for (path, target) in [
            (DIRECTOR, "../reference/supervision.md"),
            (DIRECTOR, "../reference/supervision.md#recovery"),
            (DIRECTOR, "../reference/supervision.md#shutdown"),
            (DIRECTOR, "../reference/prompt-routing.md"),
            (CORE, "reference/supervision.md#recovery"),
            (CORE, "reference/supervision.md#shutdown"),
            ("skills/cafleet/roles/member.md", "../reference/base-dir.md"),
            (
                "skills/cafleet/roles/member.md",
                "../reference/prompt-routing.md",
            ),
            (CREATE, "../reference/guidelines.md#file-layout"),
            (EXECUTE, "../reference/guidelines.md#file-layout"),
            (INTERVIEW, "../reference/guidelines.md#file-layout"),
        ] {
            inventory
                .require_link(
                    &inventory
                        .document(path)
                        .section("required-reading")
                        .unwrap(),
                    target,
                )
                .unwrap();
        }
    }

    #[test]
    fn backend_bindings_have_unique_values_and_local_model_defaults() {
        let document = INVENTORY.document(BACKENDS);
        for backend in ["claude", "codex", "opencode"] {
            let section = document.section(backend).unwrap();
            section
                .section("runtime-bindings")
                .unwrap()
                .bindings(&RUNTIME)
                .unwrap();
            let defaults = section
                .section("role-defaults")
                .unwrap()
                .bindings(&DEFAULTS)
                .unwrap();
            let catalog = section.section("model-catalog").unwrap();
            let rows = catalog.rows().unwrap();
            let columns = rows[0]
                .1
                .iter()
                .enumerate()
                .filter(|(_, key)| matches!(**key, "Model" | "Alias"))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            assert!(
                !columns.is_empty(),
                "{}",
                catalog.error(catalog.line, "model catalog needs Model/Alias")
            );
            for (key, value) in defaults {
                let value = value.trim_matches('`');
                assert!(
                    value != "—"
                        && rows.iter().skip(2).any(|(_, row)| columns
                            .iter()
                            .any(|&index| row[index].trim_matches('`') == value)),
                    "{}",
                    catalog.error(
                        catalog.line,
                        &format!(
                            "{backend} {key} default {value} is missing from its local catalog"
                        )
                    )
                );
            }
        }
    }

    #[test]
    fn backend_worked_launches_and_stop_bindings_preserve_execution_modes() {
        let document = INVENTORY.document(BACKENDS);
        for (backend, launch, stop) in [
            ("claude", "run_in_background: true", "TaskStop"),
            (
                "codex",
                "cafleet monitor <fleet-id>",
                "retained managed execution session",
            ),
            (
                "opencode",
                "cafleet monitor <fleet-id> &",
                "recorded background process",
            ),
        ] {
            let section = document.section(backend).unwrap();
            let worked = section.section("worked-resolution").unwrap();
            worked
                .require(&[
                    launch,
                    "cafleet monitor <fleet-id>",
                    "monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)",
                    "monitor live",
                ])
                .unwrap();
            let runtime = section
                .section("runtime-bindings")
                .unwrap()
                .bindings(&RUNTIME)
                .unwrap();
            if backend == "codex" {
                assert!(
                    runtime["bg_run"].contains("Codex-managed execution"),
                    "{}",
                    section.error(section.line, "bg_run must bind Codex-managed execution")
                );
            }
            assert!(
                runtime["bg_stop"].contains(stop),
                "{}",
                section.error(section.line, &format!("bg_stop must name {stop}"))
            );
            let cues = section.section("pane-state-capture-cues").unwrap();
            let rows = cues.rows().unwrap();
            let actual = rows
                .iter()
                .skip(2)
                .map(|(_, row)| row[0].trim_matches('`'))
                .collect::<BTreeSet<_>>();
            assert_eq!(
                actual,
                BTreeSet::from(["awaiting_user", "finished", "working", "stall_candidate"]),
                "{}",
                cues.error(cues.line, "capture-state keys must name all four states")
            );
            assert_eq!(
                rows.len() - 2,
                actual.len(),
                "{}",
                cues.error(cues.line, "duplicate state")
            );
        }
    }

    #[test]
    fn workflow_routes_and_complete_payload_receivers_stay_at_their_owners() {
        for (path, section, tokens) in [
            (
                CREATE,
                "step-6-finalize--clean-up-director",
                vec!["ready (doc) — user approved; finalize", "addressed (doc)"],
            ),
            (
                DRAFTER,
                "workflow",
                vec![
                    "ready (doc) — user approved; finalize",
                    "ready (doc)",
                    "addressed (doc)",
                ],
            ),
            (
                CREATE,
                "step-3-internal-quality-loop-director",
                vec![
                    "complete (doc)",
                    "addressed (doc)",
                    "QUALITY_REVIEW_ONLY=true",
                    "ready (doc)",
                    "approved (doc)",
                ],
            ),
            (DRAFTER, "resume-mode", vec!["addressed (doc)"]),
            (
                EXECUTE,
                "revision-loop-comment-marker-based-feedback",
                vec!["COMMENT(user-relay)", "ready (doc)", "approved (doc)"],
            ),
        ] {
            INVENTORY
                .document(path)
                .section(section)
                .unwrap()
                .require(&tokens)
                .unwrap();
        }
        for (path, section) in [
            (CREATE, "step-2-clarification-phase-director"),
            (INTERVIEW, "2e-wait-for-the-analyzers-question-list"),
            (EXECUTE, "coordination-protocol"),
        ] {
            let region = INVENTORY.document(path).section(section).unwrap();
            region
                .require(&["cafleet message poll <director-member-id> --json"])
                .unwrap();
            assert!(
                region.text.contains("ACK")
                    || region.text.contains("cafleet message ack <message-id>"),
                "{}",
                region.error(
                    region.line,
                    "complete payload receiver must acknowledge consumed delivery"
                )
            );
            INVENTORY
                .require_link(&region, "../reference/coordination.md#payload-exemptions")
                .unwrap();
        }
        INVENTORY
            .document(COORDINATION)
            .section("payload-exemptions")
            .unwrap()
            .require(&[
                "cafleet message poll <member-id> --json",
                "cafleet message show <message-id> --json",
                "ACK",
                "--file -",
            ])
            .unwrap();
        INVENTORY
            .document(CORE)
            .section("acknowledge-ack")
            .unwrap()
            .require(&["cafleet message ack <message-id>"])
            .unwrap();
        for path in [CREATE, EXECUTE, INTERVIEW] {
            INVENTORY
                .require_link(&INVENTORY.document(path), "../reference/coordination.md")
                .unwrap();
        }
    }

    #[test]
    fn spawn_templates_and_manual_commands_carry_consistent_identities() {
        let skeleton = INVENTORY
            .document(DIRECTOR)
            .section("canonical-spawn-prompt-skeleton")
            .unwrap();
        let skeleton = prompt(&skeleton).unwrap();
        skeleton.require(&["ROLE DEFINITION: Open [INSERT abs path to roles/", "BASE: [INSERT abs BASE path]",
            "cafleet message send --from-member-id {member_id} --to-member-id {director_member_id}"]).unwrap();
        let manual = INVENTORY
            .document(MANUAL)
            .section("manual-lifecycle")
            .unwrap();
        let installed = prompt(&manual).unwrap();
        for template in [&skeleton, &installed] {
            template
                .require(&[
                    "FLEET ID: {fleet_id}",
                    "DIRECTOR MEMBER ID: {director_member_id}",
                    "YOUR MEMBER ID: {member_id}",
                    "CODING AGENT: {coding_agent}",
                ])
                .unwrap();
            template.operational_tokens(&IDENTITIES).unwrap();
        }
        for suffix in [
            "/skills/cafleet/roles/monitor.md",
            "/skills/cafleet/reference/coding-agents.md",
            "/skills/cafleet/SKILL.md",
            "/skills/cafleet/reference/base-dir.md",
        ] {
            let path = installed
                .text
                .split_whitespace()
                .find(|part| part.ends_with(suffix))
                .unwrap_or_else(|| {
                    panic!(
                        "{}",
                        installed.error(
                            installed.line,
                            &format!("absolute installed path ending {suffix} required")
                        )
                    )
                });
            assert!(Path::new(path).is_absolute(), "{path}");
        }
        manual
            .require(&[
                "cafleet doctor --json",
                "--monitor-file",
                "--monitor-model",
                "1 director=2 monitor=3",
                "cafleet member create --fleet-id 1",
                "member ID is `4`",
                "cafleet member capture 4",
                "cafleet message send --from-member-id 2 --to-member-id 4",
                "cafleet message poll 2 --json",
                "cafleet message ack 10",
            ])
            .unwrap();
        manual
            .ordered(&[
                "cafleet member delete 3",
                "cafleet member delete 4",
                "cafleet member list 1",
                "cafleet fleet delete 1",
                "cafleet fleet list",
            ])
            .unwrap();
        INVENTORY
            .require_link(&manual, "../spec/cli-options.md#config-dir-resolution")
            .unwrap();
    }

    #[test]
    fn message_and_shell_examples_retain_commands_and_protocol_links() {
        let send = INVENTORY.document(CORE).section("send-unicast").unwrap();
        send.require(&[
            "cafleet message send --from-member-id",
            "--to-member-id",
            "--file",
            "--json",
            "Message <id> was persisted",
        ])
        .unwrap();
        for target in [
            "reference/runtime/spec/cli-options.md#json-output",
            "reference/runtime/spec/multiplexer-backends.md#push-notifications",
            "reference/supervision.md#recovery",
            "reference/runtime/spec/cli-options.md#message-send",
        ] {
            INVENTORY.require_link(&send, target).unwrap();
        }
        let broadcast = INVENTORY.document(CORE).section("broadcast").unwrap();
        broadcast
            .require(&[
                "cafleet message broadcast --from-member-id",
                "--file",
                "--json",
                "origin_message_id",
            ])
            .unwrap();
        for target in [
            "reference/runtime/spec/data-model.md#broadcast-grouping",
            "reference/runtime/spec/message-envelope.md",
            "reference/runtime/spec/cli-options.md#message-broadcast",
        ] {
            INVENTORY.require_link(&broadcast, target).unwrap();
        }
        let dispatch = INVENTORY
            .document(ROUTING)
            .section("director-side-dispatch")
            .unwrap();
        dispatch
            .ordered(&[
                "cafleet member prompt <member-id> --shell",
                "cafleet member ping <member-id>",
                "cafleet message ack <message-id>",
            ])
            .unwrap();
        for target in [
            "../roles/member.md#command-execution",
            "../roles/director.md#member-prompt",
            "../roles/director.md#member-ping-manual-inbox-poll",
        ] {
            INVENTORY
                .require_link(&INVENTORY.document(ROUTING), target)
                .unwrap();
        }
        let isolation = INVENTORY
            .document(CORE)
            .section("one-shot-command-isolation")
            .unwrap();
        isolation
            .require(&[
                "NAME=value",
                "--file <path>",
                "Operation not permitted",
                "Permission denied",
            ])
            .unwrap();
        INVENTORY
            .require_link(&isolation, "reference/supervision.md")
            .unwrap();
        INVENTORY
            .document(DIRECTOR)
            .section("member-ping-manual-inbox-poll")
            .unwrap()
            .require(&["cafleet member ping", "cafleet message poll <member-id>"])
            .unwrap();
        INVENTORY
            .document(DIRECTOR)
            .section("member-capture")
            .unwrap()
            .require(&["cafleet member capture"])
            .unwrap();
        let bash = INVENTORY
            .document(BASH)
            .section("the-owning-protocols")
            .unwrap();
        bash.require(&[
            "cafleet member ping",
            "cafleet member prompt --shell",
            "cafleet message poll <your-member-id>",
            "skills/cafleet/roles/member.md",
            "skills/cafleet/reference/prompt-routing.md",
        ])
        .unwrap();
        INVENTORY.document("docs/docs/spec/multiplexer-backends.md")
            .section("the-monitor-wake-and-the-fixed-direct-ping").unwrap().require(&[
                "[cafleet] tick: fleet <fleet-id>", "coding_agent=<agent>; unacked=<n>",
                "Follow your monitor role protocol. Resume your work if something was still running.",
                "cafleet message poll <member-id> — then\nresume your work if something was still running."
            ]).unwrap();
    }

    #[test]
    fn monitor_summaries_link_to_runtime_and_supervision_owners() {
        for (path, targets) in [
            (
                CORE,
                vec![
                    "roles/monitor.md",
                    "reference/supervision.md",
                    "reference/coding-agents.md",
                ],
            ),
            (
                DIRECTOR,
                vec!["monitor.md", "../reference/supervision.md#shutdown"],
            ),
            (
                MONITOR,
                vec!["../reference/coding-agents.md", "../SKILL.md"],
            ),
            (SUPERVISION, vec!["../roles/monitor.md", "coding-agents.md"]),
            ("skills/cafleet/roles/member.md", vec!["monitor.md"]),
            (
                AUTHOR,
                vec![
                    "../../../skills/cafleet/reference/coding-agents.md",
                    "../../../skills/cafleet/reference/base-dir.md",
                    "../../../skills/cafleet/reference/supervision.md#spawn-protocol",
                    "../../../skills/cafleet/roles/director.md#canonical-spawn-prompt-skeleton",
                    "../../../skills/cafleet/roles/director.md#member-create--scratch-and-audit-files",
                    "../../../skills/cafleet-design-doc/reference/coordination.md",
                    "../../../skills/cafleet/reference/supervision.md#recovery",
                    "../../../skills/cafleet/reference/supervision.md#shutdown",
                ],
            ),
            (
                "docs/docs/concepts/monitoring.md",
                vec![
                    "storage.md#duplicate-monitor-recovery",
                    "../spec/multiplexer-backends.md",
                    "../spec/cli-options.md#monitor-resource-cleanup",
                    "../spec/cli-options.md#cafleet-monitor-scan",
                ],
            ),
            (
                "docs/docs/concepts/storage.md",
                vec![
                    "../spec/cli-options.md#cafleet-setup",
                    "../spec/data-model.md#members",
                    "../spec/cli-options.md#stale-assets-guard",
                ],
            ),
            (
                "docs/docs/spec/webui-api.md",
                vec![
                    "../concepts/monitoring.md#cadence-and-tick-precision",
                    "data-model.md#monitor_runtime",
                ],
            ),
            (
                "docs/docs/spec/cli-options.md",
                vec![
                    "../concepts/monitoring.md",
                    "../concepts/monitoring.md#cadence-and-tick-precision",
                    "coding-agent-backends.md#spawn-argv",
                ],
            ),
            (
                "docs/docs/spec/data-model.md",
                vec![
                    "../concepts/monitoring.md#cadence-and-tick-precision",
                    "webui-api.md#get-apimonitor--fleet-monitor-runtime",
                ],
            ),
        ] {
            let document = INVENTORY.document(path);
            for target in targets {
                INVENTORY.require_link(&document, target).unwrap();
            }
        }
        INVENTORY
            .document(SUPERVISION)
            .section("spawn-protocol")
            .unwrap()
            .require(&["cafleet fleet create", "--monitor-file", "--monitor-model"])
            .unwrap();
        INVENTORY
            .document(SUPERVISION)
            .section("recovery")
            .unwrap()
            .require(&[
                "cafleet doctor",
                "cafleet member capture",
                "member delete",
                "member create",
            ])
            .unwrap();
        INVENTORY
            .document(AUTHOR)
            .section("resolve-and-bootstrap")
            .unwrap()
            .require(&["cafleet doctor", "--monitor-file", "--monitor-model"])
            .unwrap();
        INVENTORY
            .document("docs/docs/concepts/storage.md")
            .section("duplicate-monitor-recovery")
            .unwrap()
            .require(&[
                "member delete <surplus-id>",
                "CAFLEET_DATABASE_URL",
                "setup",
            ])
            .unwrap();
        let spec = INVENTORY.document("SPEC.md");
        INVENTORY
            .require_link(&spec, "#6-module-specifications")
            .unwrap();
        spec.section("member-ping")
            .unwrap()
            .require(&["send_poll_trigger", "{member_id, pane_id, skipped}"])
            .unwrap();
        spec.section("member-capture")
            .unwrap()
            .require(&["content_sha256", "--lines", "--ansi"])
            .unwrap();
        spec.section("monitor")
            .unwrap()
            .require(&[
                "cafleet monitor FLEET_ID [--tick N] [--interval N]",
                "CAFLEET_MONITOR_WAKE_INTERVAL",
                "monitor live",
            ])
            .unwrap();
        spec.section("66-monitor-heartbeat-loop")
            .unwrap()
            .section("public-surface")
            .unwrap()
            .require(&[
                "wake_due(last_wake_at, started_at, wake_interval_seconds, now)",
                "run_monitor_loop(fleet_id, tick_seconds, wake_interval_seconds)",
            ])
            .unwrap();
    }

    #[test]
    fn public_configuration_and_navigation_resolve_current_pages() {
        let quickstart = INVENTORY.document("docs/docs/quickstart.md");
        quickstart
            .section("install")
            .unwrap()
            .require(&["brew install himkt/tap/cafleet", "cafleet setup"])
            .unwrap();
        quickstart
            .section("configure")
            .unwrap()
            .require(&[
                "CLAUDE_CONFIG_DIR",
                "CODEX_HOME",
                "OPENCODE_CONFIG_DIR",
                ".config/opencode/skills",
            ])
            .unwrap();
        INVENTORY
            .require_link(&quickstart, "how-to/mixed-backend-team.md#manual-lifecycle")
            .unwrap();
        INVENTORY
            .require_link(
                &INVENTORY.document("docs/docs/concepts/coding-agents.md"),
                "../how-to/mixed-backend-team.md#manual-lifecycle",
            )
            .unwrap();
        INVENTORY
            .require_link(
                &INVENTORY
                    .document("docs/docs/concepts/coding-agents.md")
                    .section("model-choice")
                    .unwrap(),
                "../spec/coding-agent-backends.md#model-selection",
            )
            .unwrap();
        let path = "docs/docs/concepts/_meta.json";
        let document = INVENTORY.document(path);
        let entries: serde_json::Value = serde_json::from_str(document.text).unwrap();
        let mut names = BTreeSet::new();
        let mut labels = BTreeSet::new();
        let entries = entries.as_array().unwrap();
        assert!(!entries.is_empty(), "{path}:1: navigation is empty");
        for entry in entries {
            let name = entry["name"].as_str().unwrap();
            let label = entry["label"].as_str().unwrap();
            assert!(
                !name.trim().is_empty() && names.insert(name),
                "{path}:1: empty/duplicate page {name}"
            );
            assert!(
                !label.trim().is_empty() && labels.insert(label),
                "{path}:1: empty/duplicate label {label}"
            );
            INVENTORY.link(&document, 1, &format!("{name}.md")).unwrap();
        }
        for (page, anchors) in [
            (
                "cli-options",
                vec![
                    "subcommand-summary",
                    "environment-variables",
                    "output-shapes",
                    "error-messages",
                    "creation-failure-compensation",
                ],
            ),
            (
                "data-model",
                vec!["tables", "message-visibility-rules", "broadcast-grouping"],
            ),
            (
                "message-envelope",
                vec!["persisted-shape", "rendered-shape"],
            ),
            (
                "multiplexer-backends",
                vec![
                    "pane-creation-ownership",
                    "push-notifications",
                    "inline-preview-errors",
                    "esc-safeguard",
                ],
            ),
            (
                "coding-agent-backends",
                vec!["spawn-argv", "claude", "codex", "opencode"],
            ),
            (
                "webui-api",
                vec!["request-headers", "endpoints", "error-format"],
            ),
        ] {
            let path = format!("docs/docs/spec/{page}.md");
            for anchor in anchors {
                INVENTORY.document(&path).section(anchor).unwrap();
            }
        }
        INVENTORY
            .require_link(
                &INVENTORY.document("docs/docs/spec/message-envelope.md"),
                "cli-options.md#output-shapes",
            )
            .unwrap();
    }

    #[test]
    fn concrete_source_paths_resolve_at_the_declared_roots() {
        for (path, text) in INVENTORY.sources.iter().filter(|(path, _)| {
            path.starts_with("skills/")
                || path.as_str() == AUTHOR
                || path.as_str() == BASH
                || path.starts_with(".claude/skills/clean-docs/")
        }) {
            let mut fence = false;
            for (line, text) in text.lines().enumerate() {
                if text.trim_start().starts_with("```") || text.trim_start().starts_with("~~~") {
                    fence = !fence;
                    continue;
                }
                if fence {
                    continue;
                }
                for span in INLINE.captures_iter(text) {
                    for raw in span[1].split_whitespace() {
                        let token = raw
                            .trim_start_matches(['(', '[', '{', '<', '"', '\''])
                            .trim_end_matches([')', ']', '}', '"', '\'', ',', ';', '.', '!', '?']);
                        let token = LOCATOR.replace(token.split('#').next().unwrap(), "");
                        if !["cafleet/", "skills/", "docs/", "admin/", "presets/"]
                            .iter()
                            .any(|prefix| token.starts_with(prefix))
                            || token.contains("://")
                            || token.contains(['*', '<', '>', '$', '{', '}', '~', '|'])
                        {
                            continue;
                        }
                        let candidate = if token == "cafleet/SKILL.md"
                            || token.starts_with("cafleet/roles/")
                            || token.starts_with("cafleet/reference/")
                        {
                            root().join("skills").join(token.as_ref())
                        } else {
                            root().join(token.as_ref())
                        };
                        assert!(
                            candidate.exists(),
                            "{path}:{}: missing concrete source {token} (resolved {})",
                            line + 1,
                            candidate.display()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn operational_placeholders_are_scoped_to_bindings_and_spawn_templates() {
        let document = INVENTORY.document(BACKENDS);
        let allowed = RUNTIME.into_iter().chain(DEFAULTS).collect::<Vec<_>>();
        for backend in ["claude", "codex", "opencode", "template"] {
            let section = document.section(backend).unwrap();
            for part in ["runtime-bindings", "role-defaults"] {
                section
                    .section(part)
                    .unwrap()
                    .operational_tokens(&allowed)
                    .unwrap();
            }
        }
        for (path, section) in [
            (DIRECTOR, "canonical-spawn-prompt-skeleton"),
            (MANUAL, "manual-lifecycle"),
        ] {
            prompt(&INVENTORY.document(path).section(section).unwrap())
                .unwrap()
                .operational_tokens(&IDENTITIES)
                .unwrap();
        }
    }

    #[test]
    fn checker_fixtures_pin_failures_and_allow_illustrative_prose() {
        let missing = Region {
            path: "skills/cafleet/SKILL.md".into(),
            text: "",
            line: 7,
            section: "fixture".into(),
        };
        let error = INVENTORY
            .link(&missing, 9, "missing-required-owner.md")
            .unwrap_err();
        assert!(
            error.contains("skills/cafleet/SKILL.md:9:") && error.contains("missing target"),
            "{error}"
        );
        let error = INVENTORY
            .link(&missing, 11, "reference/coding-agents.md#missing-backend")
            .unwrap_err();
        assert!(
            error.contains("skills/cafleet/SKILL.md:11:") && error.contains("#missing-backend"),
            "{error}"
        );
        let alias = Region {
            path: "skills/cafleet/reference/runtime/spec/cli-options.md".into(),
            text: "",
            line: 1,
            section: "fixture".into(),
        };
        let error = INVENTORY.link(&alias, 3, "SPEC.md").unwrap_err();
        assert!(
            error.contains("cli-options.md:3:")
                && error.contains("resolved skills/cafleet/reference/runtime/spec/SPEC.md"),
            "{error}"
        );
        for (name, input, expected_line, expected) in [
            (
                "misplaced",
                "# Role\n## Required reading\nUse prerequisites.\n## Later\n| # | Read |\n|---|---|\n| 1 | [backend](coding-agents.md) |\n",
                2,
                "required table missing",
            ),
            (
                "missing",
                "# Role\n## Required reading\n| # | Read |\n|---|---|\n| 1 | other |\n",
                5,
                "row 1 must link",
            ),
        ] {
            let region = Region {
                path: format!("{name}.md"),
                text: input,
                line: 1,
                section: "document".into(),
            };
            let error = required_backend(&region).unwrap_err();
            assert!(
                error.starts_with(&format!("{name}.md:{expected_line}:"))
                    && error.contains(expected),
                "{error}"
            );
        }
        for (name, input, expected_line, expected) in [
            (
                "missing",
                "| Placeholder | Value |\n|---|---|\n",
                4,
                "missing binding bg_run",
            ),
            (
                "duplicate",
                "| Placeholder | Value |\n|---|---|\n| `{bg_run}` | launch |\n| `{bg_run}` | again |\n",
                7,
                "unknown or duplicate binding bg_run",
            ),
            (
                "empty",
                "| Placeholder | Value |\n|---|---|\n| `{bg_run}` | |\n",
                6,
                "malformed table row",
            ),
        ] {
            let region = Region {
                path: format!("{name}.md"),
                text: input,
                line: 4,
                section: "codex runtime-bindings".into(),
            };
            let error = region.bindings(&["bg_run"]).unwrap_err();
            assert!(
                error.starts_with(&format!("{name}.md:{expected_line}:"))
                    && error.contains(expected),
                "{error}"
            );
        }
        let unknown = Region {
            path: "prompt.md".into(),
            text: "FLEET ID: {fleet_id}\nOWNER: {typo}",
            line: 12,
            section: "spawn template".into(),
        };
        assert_eq!(
            unknown.operational_tokens(&IDENTITIES).unwrap_err(),
            "prompt.md:13: section spawn template: unknown operational placeholder {typo}"
        );
        let illustration = Region {
            path: "illustration.md".into(),
            text: "Use design-docs/{number}-{topic}/design-doc.md; {illustrative} is prose.\n## Operational\nFLEET ID: {fleet_id}\n",
            line: 1,
            section: "document".into(),
        };
        illustration
            .section("operational")
            .unwrap()
            .operational_tokens(&IDENTITIES)
            .unwrap();
        let unsupported = Region {
            path: "heading.md".into(),
            text: "# [linked](page.md)\n",
            line: 1,
            section: "document".into(),
        };
        assert!(
            unsupported
                .section("linked")
                .unwrap_err()
                .contains("unsupported heading syntax")
        );
        let explicit = Region {
            path: "heading.md".into(),
            text: "# [linked](page.md) {#owner}\n",
            line: 1,
            section: "document".into(),
        };
        assert_eq!(explicit.section("owner").unwrap().line, 1);
        let simple = Region {
            path: "heading.md".into(),
            text: "# `monitor_runtime`\n",
            line: 4,
            section: "document".into(),
        };
        assert_eq!(simple.section("monitor_runtime").unwrap().line, 4);
    }
}
