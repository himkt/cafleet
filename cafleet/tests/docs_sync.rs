mod structural_contracts {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Component, Path, PathBuf};
    use std::sync::{Arc, LazyLock};

    const BACKENDS: &str = "skills/cafleet/reference/coding-agents.md";
    const CORE: &str = "skills/cafleet/SKILL.md";
    const DIRECTOR: &str = "skills/cafleet/roles/director.md";
    const MEMBER: &str = "skills/cafleet/roles/member.md";
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
    const RUNTIME: [&str; 5] = [
        "decision_surface",
        "permission_flags",
        "pane_title",
        "skill_loader",
        "effort_levels",
    ];
    const SUBSECTIONS: [&str; 5] = [
        "runtime-bindings",
        "role-defaults",
        "model-catalog",
        "note--applies-at",
        "pane-state-capture-cues",
    ];
    const DEFAULTS: [&str; 4] = [
        "reviewer_model",
        "reviewer_effort",
        "other_model",
        "other_effort",
    ];
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
            for tree in ["skills", "docs/docs", ".claude/skills", ".claude/rules"] {
                walk(
                    &root.join(tree),
                    &mut paths,
                    &mut BTreeSet::new(),
                    &mut directory_visits,
                );
            }
            paths.extend([
                root.join("README.md"),
                root.join("CLAUDE.md"),
                root.join("SPEC.md"),
            ]);
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

        fn subsections(&self) -> Result<Vec<String>, String> {
            let child = self.text.chars().take_while(|c| *c == '#').count() + 1;
            let mut fence = false;
            let mut ids = Vec::new();
            for (index, line) in self.text.lines().enumerate() {
                if line.trim_start().starts_with("~~~") || line.trim_start().starts_with("```") {
                    fence = !fence;
                }
                let depth = line.chars().take_while(|c| *c == '#').count();
                if !fence && depth == child && line.as_bytes().get(depth) == Some(&b' ') {
                    ids.push(
                        heading_id(line[depth + 1..].trim())
                            .map_err(|error| self.error(self.line + index, &error))?,
                    );
                }
            }
            Ok(ids)
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

        fn require_pattern(&self, description: &str, pattern: &str) {
            let pattern = regex::Regex::new(pattern).expect("contract pattern must compile");
            assert!(
                pattern.is_match(self.text),
                "{}",
                self.error(
                    self.line,
                    &format!("required contract missing: {description}")
                )
            );
        }

        fn json_examples(&self) -> Vec<serde_json::Value> {
            let mut examples = Vec::new();
            let mut remaining = self.text;
            while let Some((_, after)) = remaining.split_once("```json\n") {
                let (json, rest) = after.split_once("\n```").unwrap_or_else(|| {
                    panic!("{}", self.error(self.line, "unclosed JSON example"))
                });
                examples.push(serde_json::from_str(json).unwrap_or_else(|error| {
                    panic!(
                        "{}",
                        self.error(self.line, &format!("invalid JSON example: {error}"))
                    )
                }));
                remaining = rest;
            }
            assert!(
                !examples.is_empty(),
                "{}",
                self.error(self.line, "JSON example required")
            );
            examples
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
                if key.ends_with("_effort") {
                    let valid = if backend == "opencode" {
                        value == "—"
                    } else {
                        matches!(value, "low" | "medium" | "high" | "xhigh" | "max")
                    };
                    assert!(valid, "{backend} {key} has unsupported value {value}");
                    continue;
                }
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

    fn assert_catalog_rows_are_unique_prefixed_and_priced(
        catalog: &Region<'_>,
        required_prefix: &str,
    ) {
        let rows = catalog.rows().unwrap();
        assert_eq!(
            rows[0].1,
            ["Model", "Class", "Input $/MTok", "Output $/MTok"],
            "{}",
            catalog.error(rows[0].0, "expected Model / Class / Input / Output table")
        );
        let mut models = BTreeSet::new();
        let decimal = regex::Regex::new(r"^[0-9]+\.[0-9]{2,}$").unwrap();
        for (line, row) in rows.iter().skip(2) {
            let model = row[0].trim_matches('`');
            assert!(
                model.starts_with(required_prefix),
                "{}",
                catalog.error(
                    *line,
                    &format!("model {model} needs the {required_prefix} prefix")
                )
            );
            assert!(
                models.insert(model),
                "{}",
                catalog.error(*line, &format!("duplicate model {model}"))
            );
            assert!(
                !row[1].is_empty(),
                "{}",
                catalog.error(*line, &format!("model {model} needs a Class"))
            );
            for (column, price) in ["Input $/MTok", "Output $/MTok"].into_iter().zip(&row[2..]) {
                assert!(
                    decimal.is_match(price),
                    "{}",
                    catalog.error(
                        *line,
                        &format!(
                            "{model} {column} needs a decimal with at least two fractional digits"
                        )
                    )
                );
                let amount = price.parse::<f64>().unwrap_or_else(|error| {
                    panic!(
                        "{}",
                        catalog.error(*line, &format!("invalid {model} {column}: {error}"))
                    )
                });
                assert!(
                    amount.is_finite() && amount >= 0.0,
                    "{}",
                    catalog.error(
                        *line,
                        &format!("{model} {column} needs a non-negative finite price")
                    )
                );
            }
        }
    }

    #[test]
    fn opencode_catalog_rows_are_unique_prefixed_and_priced() {
        let catalog = INVENTORY
            .document(BACKENDS)
            .section("opencode")
            .unwrap()
            .section("model-catalog")
            .unwrap();
        assert_catalog_rows_are_unique_prefixed_and_priced(&catalog, "opencode/");
    }

    #[test]
    fn opencode_go_catalog_rows_are_unique_prefixed_and_priced() {
        let catalog = INVENTORY
            .document(BACKENDS)
            .section("opencode")
            .unwrap()
            .section("model-catalog")
            .unwrap()
            .section("opencode-go")
            .unwrap();
        assert_catalog_rows_are_unique_prefixed_and_priced(&catalog, "opencode-go/");
    }

    #[test]
    fn backend_sections_hold_the_canonical_subsections_in_order() {
        let document = INVENTORY.document(BACKENDS);
        for backend in ["claude", "codex", "opencode", "template"] {
            let section = document.section(backend).unwrap();
            assert_eq!(
                section.subsections().unwrap(),
                SUBSECTIONS,
                "{}",
                section.error(
                    section.line,
                    "backend subsections must match the canonical list"
                )
            );
        }
    }

    #[test]
    fn backend_capture_cues_are_director_inspection_guidance() {
        let document = INVENTORY.document(BACKENDS);
        for backend in ["claude", "codex", "opencode"] {
            let section = document.section(backend).unwrap();
            let cues = section.section("pane-state-capture-cues").unwrap();
            cues.require(&["member capture", "fleet capture"]).unwrap();
            cues.require_pattern(
                "cues guide Director visual inspection",
                r"(?i)Director.{0,20}(visual )?inspection",
            );
            cues.require_pattern(
                "delivery and dispatch never consult the cues",
                r"(?i)delivery and dispatch do not consult",
            );
            assert!(
                !cues.text.contains("transport evidence"),
                "{}",
                cues.error(
                    cues.line,
                    "capture cues are not transport readiness evidence"
                )
            );
            let rows = cues.rows().unwrap();
            let actual = rows
                .iter()
                .skip(2)
                .map(|(_, row)| row[0].trim_matches('`'))
                .collect::<BTreeSet<_>>();
            assert_eq!(
                actual,
                BTreeSet::from(["idle", "working", "awaiting_user", "draft", "unknown"]),
                "{}",
                cues.error(
                    cues.line,
                    "capture cues must distinguish empty, active and retained input"
                )
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
                "cafleet fleet create",
                "--coding-agent",
                "cafleet member create --fleet-id 1",
                "cafleet message send --from-member-id",
                "--to-member-id",
                "cafleet message wait",
                "--timeout 300 --json",
                "cafleet message ack",
            ])
            .unwrap();
        manual
            .ordered(&[
                "cafleet member delete",
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
    fn message_and_exec_examples_retain_commands_and_protocol_links() {
        let send = INVENTORY.document(CORE).section("send-unicast").unwrap();
        send.require(&[
            "cafleet message send --from-member-id",
            "--to-member-id",
            "--file",
            "--json",
        ])
        .unwrap();
        for target in [
            "reference/runtime/spec/cli-options.md#json-output",
            "reference/runtime/spec/multiplexer-backends.md#push-notifications",
            "reference/runtime/spec/cli-options.md#message-send",
        ] {
            INVENTORY.require_link(&send, target).unwrap();
        }
        let core = INVENTORY.document(CORE);
        INVENTORY
            .require_link(&core, "reference/runtime/spec/cli-options.md#message-wait")
            .unwrap();
        send.require(&[
            "[cafleet msg ID from SENDER TIMESTAMP]",
            "cafleet message show ID --json",
        ])
        .unwrap();
        send.require_pattern(
            "ordinary members receive Escape-first pane previews",
            r"(?is)ordinary.member.{0,80}Escape-first.{0,80}recipient's pane",
        );
        send.require_pattern(
            "the root Director receives through the native inbox stream",
            r"(?is)root Director.{0,80}(native|message listen)",
        );
        send.require_pattern(
            "Director sends report broker persistence without pane submission",
            r"(?is)Director.{0,120}notification_sent.{0,20}false",
        );
        let routing = INVENTORY.document(ROUTING);
        routing
            .require(&["Need to run: <command>. My harness denied it."])
            .unwrap();
        routing
            .section("director-side-dispatch")
            .unwrap()
            .ordered(&[
                "cafleet member exec <member-id>",
                "cafleet message ack <message-id>",
            ])
            .unwrap();
        for target in [
            "../roles/member.md#command-execution",
            "../roles/director.md#member-exec",
            "../roles/director.md#member-prompt",
        ] {
            INVENTORY.require_link(&routing, target).unwrap();
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
            .require_link(&isolation, "reference/supervision.md#director-wake")
            .unwrap();
        let prompt = INVENTORY
            .document(DIRECTOR)
            .section("member-prompt")
            .unwrap();
        prompt
            .require(&["cafleet member prompt <member-id>"])
            .unwrap();
        prompt.require_pattern("prompt leads with Escape", r"(?i)leads with Escape");
        prompt.require_pattern(
            "prompt rejects the Director's own pane",
            r"(?i)rejects the Director's own pane",
        );
        prompt.require_pattern(
            "retained draft check before prompting",
            r"(?is)draft.{0,80}before prompting",
        );
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
            "cafleet member exec",
            "skills/cafleet/roles/member.md",
            "skills/cafleet/reference/prompt-routing.md",
        ])
        .unwrap();
        for path in ["docs/docs/spec/cli-options.md", "SPEC.md"] {
            INVENTORY
                .document(path)
                .section("member-prompt")
                .unwrap()
                .require(&[
                    "cannot prompt the Director's own pane",
                    "member ID's pane input remained busy; prompt was not dispatched.",
                    "prompt was not dispatched to member ID: CAUSE",
                    "Inspect the pane before retrying.",
                    "Escape",
                ])
                .unwrap();
        }
    }

    #[test]
    fn removed_input_gates_have_no_living_mentions() {
        let removed = [
            ["member", "ping"].join(" "),
            ["asynchronous", "wait", "rule"].join("-"),
            ["poll", "trigger"].join("-"),
            ["pane", "state", "classification"].join("-"),
            ["send", "inline", "preview"].join("_"),
        ];
        for (path, text) in &INVENTORY.sources {
            let source = INVENTORY.document(path);
            for (offset, line) in text.lines().enumerate() {
                for needle in &removed {
                    assert!(
                        !line.contains(needle.as_str()),
                        "{}",
                        source.error(
                            offset + 1,
                            &format!("removed concept {needle:?} is mentioned")
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn receive_and_recovery_instructions_link_to_their_owners() {
        for (path, targets) in [
            (
                CORE,
                vec![
                    "roles/director.md",
                    "roles/member.md",
                    "reference/supervision.md",
                    "reference/coding-agents.md",
                ],
            ),
            (DIRECTOR, vec!["../reference/supervision.md#shutdown"]),
            (SUPERVISION, vec!["coding-agents.md"]),
            (MEMBER, vec!["../reference/prompt-routing.md"]),
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
                "docs/docs/concepts/storage.md",
                vec![
                    "../spec/cli-options.md#cafleet-setup",
                    "../spec/cli-options.md#stale-assets-guard",
                ],
            ),
            (
                "docs/docs/spec/cli-options.md",
                vec!["coding-agent-backends.md#spawn-argv"],
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
            .require(&["cafleet fleet create", "cafleet member create", "ready"])
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
            .require(&["cafleet doctor", "cafleet fleet create"])
            .unwrap();
        let spec = INVENTORY.document("SPEC.md");
        INVENTORY
            .require_link(&spec, "#6-module-specifications")
            .unwrap();
        for path in ["docs/docs/spec/cli-options.md", "SPEC.md"] {
            let document = INVENTORY.document(path);
            document
                .section("member-list-output")
                .unwrap()
                .require(&[
                    "unconfirmed_previews",
                    "idle",
                    "preview_started_at",
                    "notified_at",
                    "input_required",
                    "(created_at, message_id)",
                ])
                .unwrap();
            document
                .section("member-show")
                .unwrap()
                .require(&["unconfirmed_previews", "last key"])
                .unwrap();
        }
        spec.section("member-capture")
            .unwrap()
            .require(&["content_sha256", "--lines", "--ansi"])
            .unwrap();
        spec.section("message-wait")
            .unwrap()
            .require(&["--timeout", "300", "notification_errors", "outcome"])
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
            ("data-model", vec!["tables", "message-visibility-rules"]),
            (
                "message-envelope",
                vec!["persisted-shape", "rendered-shape"],
            ),
            (
                "multiplexer-backends",
                vec!["pane-creation-ownership", "push-notifications"],
            ),
            (
                "coding-agent-backends",
                vec!["spawn-argv", "claude", "codex", "opencode"],
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
    fn discovery_selects_independent_sessions_for_delegation() {
        let core = INVENTORY.document(CORE);
        let frontmatter = core
            .text
            .strip_prefix("---\n")
            .and_then(|text| text.split_once("\n---"))
            .expect("CAFleet discovery needs YAML frontmatter")
            .0;
        let description = Region {
            text: frontmatter,
            ..core.clone()
        };
        description
            .require(&["description:", "tmux", "herdr", "durable"])
            .unwrap();
        description.require_pattern("proactive delegation", r"(?i)proactiv\w*");
        description.require_pattern("subagent substitution", r"(?i)subagents?");
        description.require_pattern("agent-team substitution", r"(?i)agent teams?");
        let bindings = INVENTORY
            .document(BACKENDS)
            .section("claude")
            .unwrap()
            .section("runtime-bindings")
            .unwrap()
            .bindings(&RUNTIME)
            .unwrap();
        assert_eq!(bindings["skill_loader"], "the Skill tool");
    }

    #[test]
    fn director_owns_native_reception_and_reconciles_each_received_batch() {
        let supervision = INVENTORY.document(SUPERVISION);
        let wake = supervision.section("director-wake").unwrap();
        wake.require(&[
            "cafleet message poll <director-member-id> --json",
            "cafleet fleet capture",
            "cafleet member capture",
            "cafleet member list <fleet-id> --json",
            "unconfirmed_previews",
            "cafleet member prompt <member-id> \"cafleet message poll <member-id> --json\"",
            "ready",
            "message",
            "closed",
            "incomplete JSONL",
            "context compaction",
        ])
        .unwrap();
        wake.require_pattern(
            "one native receiver remains owned across messages",
            r"(?is)one.{0,30}(owned|native).{0,30}(handle|receiver)",
        );
        wake.require_pattern(
            "poll on each batch or wake",
            r"(?i)(on|every|each).{0,35}(batch|wake)",
        );
        wake.require_pattern(
            "empty native output retains the same live receiver",
            r"(?is)(no output|empty read).{0,100}(same|retain|keep)",
        );
        supervision.require_pattern(
            "deferred actions remain host-owned",
            r"(?is)deferred.{0,120}host-owned",
        );
        supervision.require_pattern(
            "retained draft relay",
            r"(?is)(draft|did not submit).{0,220}(relay|ask the user)",
        );
        wake.require_pattern(
            "required decisions await operator answers",
            r"(?is)decisions.{0,50}pending.{0,60}(operator|answer)",
        );
        wake.require_pattern(
            "automatic Director reception uses stdout and broker reads",
            r"(?is)Director.{0,70}broker reads and stdout only",
        );
        let recovery = wake.section("receiver-recovery").unwrap();
        recovery
            .require(&[
                "Unexpected EOF",
                "fresh successful",
                "termination",
                "manual",
                "activation",
                "pause",
                "completion",
                "budget",
                "goal lifecycle",
            ])
            .unwrap();
        recovery.require_pattern(
            "unknown receiver liveness preserves ownership before replacement",
            r"(?is)liveness unknown.{0,200}(preserve|retain).{0,100}termination",
        );
        let follow_up = wake.section("sender-follow-up").unwrap();
        follow_up
            .require(&[
                "Message N was persisted.",
                "message N was not delivered to your pane; poll your inbox",
            ])
            .unwrap();
        follow_up.require_pattern("exactly one follow-up", r"(?i)exactly one follow-up");
        supervision
            .section("communication-model")
            .unwrap()
            .require(&[
                "ACK",
                "consumption",
                "poll --json",
                "show --json",
                "replayed",
                "completed",
            ])
            .unwrap();
        supervision
            .section("spawn-protocol")
            .unwrap()
            .ordered(&[
                "cafleet fleet create",
                "message listen",
                "ready",
                "cafleet member create",
            ])
            .unwrap();
        supervision
            .section("shutdown")
            .unwrap()
            .ordered(&[
                "Consume the final inbox",
                "cafleet fleet delete",
                "fleet_deleted",
                "native task exit",
            ])
            .unwrap();
        supervision.require_pattern(
            "explicit user handoff",
            r"(?i)explicit.{0,80}(handoff|hand.{0,10}back)",
        );
        supervision.require_pattern(
            "captures never gate delivery",
            r"(?is)delivery and dispatch never wait on (them|captures)",
        );
        for path in [CORE, DIRECTOR] {
            INVENTORY
                .require_link(
                    &INVENTORY.document(path),
                    if path == CORE {
                        "reference/supervision.md#director-wake"
                    } else {
                        "../reference/supervision.md#director-wake"
                    },
                )
                .unwrap();
        }
    }

    #[test]
    fn native_receive_tool_recipes_have_a_single_backend_owner() {
        let commands = [
            "cafleet message listen <director-id> --timeout 300",
            "cafleet message listen 1 --timeout 210",
        ];
        let mut found = BTreeSet::new();
        for (path, text) in INVENTORY.sources.iter().filter(|(path, _)| {
            path.starts_with("skills/") && !path.starts_with("skills/cafleet/reference/runtime/")
        }) {
            let source = INVENTORY.document(path);
            for (offset, line) in text.lines().enumerate() {
                for command in commands {
                    if line.contains(command) {
                        assert_eq!(
                            path,
                            BACKENDS,
                            "{}",
                            source.error(
                                offset + 1,
                                "backend instructions own concrete native receive recipes"
                            )
                        );
                        assert!(found.insert(command), "native recipe duplicated: {command}");
                    }
                }
            }
        }
        assert_eq!(found, BTreeSet::from(commands));
        let listen = INVENTORY
            .document(CORE)
            .section("listen-native-inbox-stream")
            .unwrap();
        for target in [
            "reference/supervision.md#director-wake",
            "reference/coding-agents.md",
            "reference/runtime/spec/cli-options.md#message-listen",
        ] {
            INVENTORY.require_link(&listen, target).unwrap();
        }
    }

    #[test]
    fn members_poll_at_startup_report_boundary_and_wake() {
        let member = INVENTORY.document(MEMBER);
        member
            .ordered(&[
                "cafleet message send --from-member-id <my-member-id>",
                "cafleet message poll <my-member-id>",
            ])
            .unwrap();
        member.require_pattern(
            "poll before reporting completion",
            r"(?is)poll.{0,150}before.{0,100}(complet|report)",
        );
        member.require_pattern(
            "turn-boundary inbox handling",
            r"(?is)(queued|new).{0,80}(assignment|message).{0,180}(idle|end.{0,10}turn)",
        );
        member.require_pattern("wake inbox check", r"(?i)(on|each|every).{0,30}wake");
        member
            .require(&[
                "--json",
                "Director",
                "member prompt",
                "member exec",
                "Message N was persisted.",
                "message N was not delivered to your pane; poll your inbox",
            ])
            .unwrap();
        member.require_pattern("exactly one sender follow-up", r"(?i)exactly one follow-up");
        member.require_pattern(
            "a failed follow-up ends the turn",
            r"(?is)follow-up also.{0,80}end your turn",
        );
    }

    #[test]
    fn wait_specifies_bounded_results_and_notification_retry_ownership() {
        for path in ["docs/docs/spec/cli-options.md", "SPEC.md"] {
            let wait = INVENTORY.document(path).section("message-wait").unwrap();
            wait.require(&[
                "--timeout",
                "300",
                "1800",
                "member_id",
                "outcome",
                "messages",
                "notification_errors",
                "message_id",
                "delivery",
                "not_sent",
                "uncertain",
            ])
            .unwrap();
            wait.require_pattern("monotonic deadline", r"(?i)monotonic");
            wait.require_pattern(
                "one-second retry interval",
                r"(?i)(one[- ]second|1[- ]second|every second)",
            );
            wait.require_pattern("round-robin member selection", r"(?i)round[- ]robin");
            wait.require_pattern(
                "five-second in-flight budget",
                r"(?i)(five[- ]second|5[- ]second|five seconds|5 seconds)",
            );
            wait.require_pattern(
                "normal outcomes succeed",
                r"(?is)(exit.{0,40}0|0.{0,40}(exit|outcome))",
            );
            wait.require_pattern("diagnostics revalidated", r"(?i)revalidat\w*");
            wait.require(&["ACK", "Director"]).unwrap();
        }
    }

    #[test]
    fn notification_spec_defines_recipient_selection_and_exact_preview() {
        let mux = INVENTORY
            .document("docs/docs/spec/multiplexer-backends.md")
            .section("push-notifications")
            .unwrap();
        mux.require(&[
            "[cafleet msg",
            "notified_at",
            "⏎",
            "␉",
            "[cafleet] Resume your work if something was still running.",
            "[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.",
            "Body is N characters; run cafleet message show MESSAGE_ID --json to read it.",
            "4000",
            "CAFLEET_MAX_TEXT_LEN",
            "unconfirmed_previews",
        ])
        .unwrap();
        mux.require_pattern(
            "oldest pending preview",
            r"(?is)oldest.{0,100}(pending|unack)",
        );
        mux.require_pattern(
            "automatic pane previews select concrete ordinary recipients",
            r"(?is)ordinary recipients.{0,50}concrete panes.{0,50}automatic",
        );
        mux.require_pattern(
            "the root Director reads its inbox through the native stream",
            r"(?is)root Director.{0,120}message listen.{0,160}(zero|no) automatic pane input",
        );
        mux.require_pattern(
            "Director exclusion precedes preview selection, locking and multiplexer resolution",
            r"(?is)Exclude the Director before.{0,120}preview.{0,80}lock.{0,80}multiplexer",
        );
        mux.require_pattern("shared input lock", r"(?i)(input|pane).{0,30}lock");
        mux.require_pattern(
            "one lock hold drains oldest-first",
            r"(?is)drains.{0,80}oldest-first.{0,120}one lock hold",
        );
        mux.require_pattern(
            "delivery leaves the message pending until ACK",
            r"(?is)leaves `status_state` unchanged.{0,200}ACK",
        );
        mux.require_pattern(
            "a retained draft can merge with a delivery",
            r"(?is)draft.{0,240}merge",
        );
        mux.require_pattern(
            "retry rotation selects other ordinary members",
            r"(?is)Rotation selects.{0,90}ordinary.member.{0,100}excluding.{0,70}Director.{0,70}subject",
        );
        let send = INVENTORY
            .document("docs/docs/spec/cli-options.md")
            .section("message-send")
            .unwrap();
        send.require(&["notification_sent", "persisted", "preview"])
            .unwrap();
        send.require_pattern(
            "committed send remains successful",
            r"(?is)(commit|persist).{0,220}(exit.{0,15}0|success)",
        );
    }

    #[test]
    fn codex_recipe_retains_one_session_and_preserves_host_authority() {
        let recipe = INVENTORY
            .document(BACKENDS)
            .section("codex-native-director-reception")
            .unwrap();
        recipe
            .require(&[
                "exec_command",
                "session_id",
                "write_stdin",
                "chars",
                "JSONL",
                "ready",
                "closed",
                "context compaction",
                "activation",
                "pause",
                "completion",
                "budget",
            ])
            .unwrap();
        recipe.require_pattern(
            "repeated reads use the same session",
            r"(?is)write_stdin.{0,80}same session",
        );
        recipe.require_pattern(
            "reads have a bounded yield",
            r"(?is)bounded yield.{0,40}(10,000|10000)",
        );
        recipe.require_pattern(
            "reception runs independently of host goal presence",
            r"(?is)independently.{0,60}host goal",
        );
        recipe.require_pattern(
            "idle reception waits for a new host activation",
            r"(?is)next user/native activation.{0,160}fully idle Codex",
        );
        recipe.require_pattern(
            "partial JSONL survives chunk boundaries",
            r"(?is)incomplete JSONL across chunks",
        );
        recipe.require_pattern(
            "replacement establishes the old receiver exit",
            r"(?is)closed.{0,100}(establish|observed).{0,40}(termination|exit)",
        );
        recipe.require_pattern(
            "goal lifecycle authority is preserved",
            r"(?is)(never|no).{0,30}(call|use).{0,30}goal lifecycle",
        );
        recipe.require_pattern(
            "user decisions gate dependent actions",
            r"(?is)user decisions.{0,50}gate dependent work",
        );
        recipe.require_pattern(
            "goal preservation needs independent state evidence",
            r"(?is)independently observed goal.state.{0,100}acceptance evidence",
        );
        INVENTORY
            .require_link(&recipe, "supervision.md#receiver-recovery")
            .unwrap();
    }

    #[test]
    fn claude_recipe_owns_monitor_events_renewal_and_source_child_cleanup() {
        let recipe = INVENTORY
            .document(BACKENDS)
            .section("claude-native-director-reception")
            .unwrap();
        let examples = recipe.json_examples();
        assert_eq!(examples.len(), 1);
        assert_eq!(
            examples[0]["command"],
            "cafleet message listen 1 --timeout 210"
        );
        assert_eq!(examples[0]["timeout_ms"], 240000);
        recipe
            .require(&[
                "Monitor",
                "ready",
                "task ID",
                "ACK",
                "replay",
                "/goal",
                "pause",
                "budget",
                "Bash(run_in_background=true)",
            ])
            .unwrap();
        recipe.require_pattern(
            "native events carry the stream protocol",
            r"(?i)(JSONL|protocol record)",
        );
        recipe.require_pattern(
            "Monitor authorization is independently checked",
            r"(?is)authorization separately from Bash",
        );
        recipe.require_pattern(
            "individual records may start turns before process exit",
            r"(?is)stdout event.{0,100}new model turn before.{0,40}exits",
        );
        recipe.require_pattern(
            "renewal waits for old task and child exit",
            r"(?is)establish.{0,100}task.{0,50}source process.{0,50}exited",
        );
        recipe.require_pattern(
            "closure and completion deduplicate by owned task ID",
            r"(?is)Deduplicate.{0,100}owned task ID",
        );
        recipe.require_pattern(
            "stale completion notifications retain current ownership",
            r"(?is)ignor.{0,30}stale completion.{0,80}ownership",
        );
        recipe.require_pattern(
            "open questions retain their decision gate",
            r"(?is)open user question.{0,120}no answer or authority",
        );
        recipe.require_pattern(
            "cancellation and real expiry require acceptance evidence",
            r"(?is)Actual Monitor expiry.{0,120}acceptance evidence",
        );
        INVENTORY
            .require_link(&recipe, "supervision.md#receiver-recovery")
            .unwrap();
    }

    #[test]
    fn listen_specifies_read_only_jsonl_lifecycle_and_resource_deadlines() {
        for path in ["docs/docs/spec/cli-options.md", "SPEC.md"] {
            let listen = INVENTORY.document(path).section("message-listen").unwrap();
            listen
                .require(&[
                    "cafleet message listen MEMBER_ID [--timeout SECONDS]",
                    "JSONL",
                    "1..=1800",
                    "monotonic",
                    "ready",
                    "closed",
                    "member_inactive",
                    "fleet_deleted",
                    "created_at ASC, message_id ASC",
                    "input_required",
                    "notified_at",
                    "preview_started_at",
                    "poll --json",
                    "show --json",
                    "ACK",
                    "backpressure",
                    "progress handler",
                    "1,000",
                    "decoding each row",
                    "before commit",
                    "cleanup failures",
                    "descriptor flags",
                    "partial writes",
                    "flush",
                    "exit 1",
                ])
                .unwrap();
            listen.require_pattern(
                "new records leave the original lease unchanged",
                r"(?is)activity leaves expiry unchanged",
            );
            listen.require_pattern(
                "replay suppression waits after no newly emitted IDs",
                r"(?is)only previously emitted pending IDs waits one second",
            );
            listen.require_pattern(
                "restart replays pending records",
                r"(?is)restart replays.{0,30}pending IDs",
            );
            listen.require_pattern("listener consumption and preview fields remain unchanged", r"(?is)leaves consumption.{0,100}notified_at.{0,100}preview_started_at.{0,50}unchanged");
            listen.require_pattern(
                "snapshot completion precedes output and sleep",
                r"(?is)Commit before output or sleep",
            );
            listen.require_pattern(
                "reader state is released before output",
                r"(?is)short read transaction.{0,300}Commit before output",
            );
            let after = listen
                .text
                .split_once("```json\n")
                .expect("JSONL protocol example")
                .1;
            let jsonl = after
                .split_once("\n```")
                .expect("closed JSONL protocol example")
                .0;
            let records: Vec<serde_json::Value> = jsonl
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                records
                    .iter()
                    .map(|record| record["event"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["ready", "message", "closed"]
            );
            assert_eq!(records[0].as_object().unwrap().len(), 3);
            assert_eq!(records[1].as_object().unwrap().len(), 2);
            assert_eq!(
                records[1]["message"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                [
                    "message_id",
                    "from_member_id",
                    "to_member_id",
                    "created_at",
                    "status_state",
                    "status_timestamp",
                    "text"
                ]
            );
            assert_eq!(records[2]["reason"], "timeout");
        }
    }

    #[test]
    fn message_examples_use_the_point_to_point_record() {
        fn collect<'a>(value: &'a serde_json::Value, records: &mut Vec<&'a serde_json::Value>) {
            match value {
                serde_json::Value::Object(object) if object.contains_key("message_id") => {
                    records.push(value)
                }
                serde_json::Value::Object(object) => {
                    for value in object.values() {
                        collect(value, records);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        collect(value, records);
                    }
                }
                _ => {}
            }
        }
        let envelope = INVENTORY.document("docs/docs/spec/message-envelope.md");
        let examples = envelope.json_examples();
        let mut records = Vec::new();
        for example in &examples {
            collect(example, &mut records);
        }
        assert!(
            !records.is_empty(),
            "message-envelope needs a complete message example"
        );
        for record in records {
            let object = record.as_object().unwrap();
            assert_eq!(
                object.keys().map(String::as_str).collect::<Vec<_>>(),
                [
                    "message_id",
                    "from_member_id",
                    "to_member_id",
                    "created_at",
                    "status_state",
                    "status_timestamp",
                    "text",
                ]
            );
            assert!(
                object["to_member_id"].is_i64(),
                "recipient must be an integer"
            );
            assert!(matches!(
                object["status_state"].as_str(),
                Some("input_required" | "completed")
            ));
        }
    }

    #[test]
    fn data_model_documents_current_storage_and_delivery_columns() {
        let data = INVENTORY.document("docs/docs/spec/data-model.md");
        let schema = INVENTORY
            .document("SPEC.md")
            .section("8-database-schema")
            .unwrap();
        for (table, columns) in [
            (
                "messages",
                vec![
                    "message_id",
                    "from_member_id",
                    "to_member_id",
                    "created_at",
                    "status_state",
                    "status_timestamp",
                    "text",
                    "notified_at",
                ],
            ),
            (
                "members",
                vec![
                    "member_id",
                    "fleet_id",
                    "name",
                    "description",
                    "status",
                    "registered_at",
                    "deregistered_at",
                ],
            ),
        ] {
            data.section(table).unwrap();
            let declaration = regex::Regex::new(&format!(
                r"(?is)CREATE TABLE(?: IF NOT EXISTS)? {table}\s*\((.*?)\n\);"
            ))
            .unwrap();
            let declaration = declaration
                .captures(schema.text)
                .unwrap_or_else(|| panic!("SPEC schema needs CREATE TABLE {table}"));
            let actual = declaration[1]
                .lines()
                .filter_map(|line| {
                    let column = line.split_whitespace().next()?;
                    (!matches!(
                        column,
                        "PRIMARY" | "FOREIGN" | "UNIQUE" | "CHECK" | "CONSTRAINT"
                    ))
                    .then_some(column)
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(
                actual,
                columns.into_iter().collect(),
                "{table}: current columns"
            );
        }
        let storage = INVENTORY.document("docs/docs/concepts/storage.md");
        storage
            .require(&["cafleet_v8.db", "application_id", "1128351288"])
            .unwrap();
        storage.require_pattern(
            "incompatible file preservation",
            r"(?is)incompatib\w*.{0,260}(preserv|unchanged|before.{0,30}writ)",
        );
        data.require(&["asset_installs", "member_placements", "foreign"])
            .unwrap();
    }

    #[test]
    fn database_diagnosis_documents_recorded_version_shapes_and_asset_precedence() {
        for path in ["docs/docs/spec/cli-options.md", "SPEC.md"] {
            let document = INVENTORY.document(path);
            let database = document.section("database-section").unwrap();
            database
                .require(&[
                    "application_id=0",
                    "1128351288",
                    "asset_installs",
                    "database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.",
                ])
                .unwrap();
            database.require_pattern(
                "supported shapes follow the recorded migration version",
                r"(?is)application identity.{0,60}1128351288.{0,100}read the recorded version.{0,150}against that recorded version's migrations",
            );
            database.require_pattern(
                "newer versions retain their upgrade diagnosis",
                r"(?is)newer version reports\s+ahead without comparing its unknown shape to the current head",
            );
            database.require_pattern(
                "a matching ledger does not replace format validation",
                r"(?is)missing required table.{0,260}incompatible.{0,100}ledger version equals\s+head",
            );
            database.require_pattern(
                "a missing install table has the format diagnosis",
                r"(?is)dropped.{0,30}asset_installs.{0,70}incompatible-format diagnosis before any install-row lookup",
            );
            document
                .section("stale-assets-guard")
                .unwrap()
                .require_pattern(
                    "missing install rows belong to a compatible current database",
                    r"(?is)No agent has a row.{0,120}compatible at-head database",
                );
        }
    }

    #[test]
    fn native_exec_contract_exposes_submission_and_uncertainty() {
        for path in ["docs/docs/spec/cli-options.md", "SPEC.md"] {
            let exec = INVENTORY.document(path).section("member-exec").unwrap();
            exec.require(&[
                "--file",
                "COMMAND",
                "member_id",
                "pane_id",
                "coding_agent",
                "command",
                "dispatched",
                "command must be one non-empty line without control characters.",
                "Inspect the pane before retrying.",
            ])
            .unwrap();
            exec.require_pattern(
                "visible native shell input",
                r"(?i)(`! `|! COMMAND|! <command>)",
            );
            exec.require_pattern(
                "successful dispatch is submission only",
                r"(?is)(exit.{0,30}0|success).{0,180}(submi|dispatch)",
            );
            exec.require_pattern(
                "partial delivery uncertainty",
                r"(?is)uncertain.{0,120}CAUSE",
            );
        }
        let dispatch = INVENTORY
            .document(ROUTING)
            .section("director-side-dispatch")
            .unwrap();
        dispatch
            .ordered(&[
                "cafleet member exec",
                "cafleet member capture",
                "cafleet member prompt",
            ])
            .unwrap();
        dispatch.require_pattern(
            "resume after native output",
            r"(?is)(output|result).{0,200}(continu|resum)",
        );
        dispatch.require_pattern(
            "inspect before uncertain retry",
            r"(?is)uncertain.{0,180}(captur|inspect)",
        );
    }

    #[test]
    fn input_contract_preserves_drafts_and_serializes_complete_submission() {
        let mux = INVENTORY.document("docs/docs/spec/multiplexer-backends.md");
        mux.require(&[
            ".pane-locks/",
            "SHA-256",
            "tmux send-keys -t PANE Escape",
            "herdr pane send-keys PANE escape",
            "insufficient pane input budget for literal input, settle and Enter",
            "member ID's pane input remained busy; preview MESSAGE_ID was not sent.",
            "member ID's pane input remained busy; prompt was not dispatched.",
            "member ID's pane input remained busy; native command was not dispatched.",
        ])
        .unwrap();
        mux.require_pattern(
            "process-shared exclusion",
            r"(?i)(process[- ]shared|cross[- ]process|OS file lock)",
        );
        mux.require_pattern(
            "lock covers Enter and stamp",
            r"(?is)(lock|guard).{0,400}Enter.{0,180}(stamp|notified_at)",
        );
        mux.require_pattern(
            "no input path reads the pane",
            r"(?i)No input path captures the pane",
        );
        mux.require_pattern(
            "bounded lock wait with the input reservation",
            r"(?is)3\.5-second input reservation.{0,80}1\.5 seconds",
        );
        let termination = mux.section("termination-during-pane-input").unwrap();
        termination
            .require(&[
                "SIGINT",
                "SIGTERM",
                "SIGHUP",
                "SIGKILL",
                "unconfirmed_previews",
            ])
            .unwrap();
        let backends = INVENTORY.document("docs/docs/spec/coding-agent-backends.md");
        backends
            .section("spawn-argv")
            .unwrap()
            .require(&["--prompt-suggestions false"])
            .unwrap();
    }

    #[test]
    fn backend_notes_preserve_host_evidence_and_consuming_links() {
        let backend_doc = INVENTORY.document(BACKENDS);
        let human_doc = INVENTORY.document("docs/docs/spec/coding-agent-backends.md");
        for backend in ["claude", "codex", "opencode"] {
            let section = backend_doc.section(backend).unwrap();
            let notes = section.section("note--applies-at").unwrap();
            let rows = notes.rows().unwrap();
            assert!(
                rows.iter().skip(2).any(|(_, row)| row.iter().any(|cell| {
                    LINKS.captures_iter(cell).any(|link| {
                        link[1].contains("supervision.md") || link[1].contains("prompt-routing.md")
                    })
                })),
                "{backend}: bind host facts to their consuming instructions"
            );
            let human = human_doc.section(backend).unwrap();
            for region in [&notes, &human] {
                region.require_pattern(
                    "native shell provenance",
                    r"(?i)native.{0,35}(shell|input|command)",
                );
                region.require_pattern(
                    "permission evidence boundary",
                    r"(?i)(permission|authoriz|sandbox)",
                );
                match backend {
                    "claude" => {
                        region.require_pattern(
                            "transcript evaluator",
                            r"(?i)(evaluat\w*.{0,100}transcript|transcript.{0,100}evaluat)",
                        );
                        region.require_pattern(
                            "background defers evaluation",
                            r"(?is)background.{0,220}(defer|evaluat|result)",
                        );
                    }
                    "codex" => {
                        region.require_pattern(
                            "thread-owned objective",
                            r"(?is)(objective|goal).{0,100}thread|thread.{0,100}(objective|goal)",
                        );
                        region.require_pattern(
                            "native handle ownership",
                            r"(?i)(task|session|tool).{0,40}handle",
                        );
                    }
                    "opencode" => {
                        region.require_pattern("assignment execution", r"(?i)assign\w*");
                        region.require_pattern("workspace tool capability", r"(?i)workspace");
                        region.require_pattern("selected provider/model", r"(?i)(provider|model)");
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn workflows_preserve_phase_gates_and_use_the_shared_receive_procedure() {
        for path in [CREATE, EXECUTE, INTERVIEW] {
            let workflow = INVENTORY.document(path);
            INVENTORY
                .require_link(&workflow, "../../cafleet/reference/supervision.md")
                .unwrap();
            workflow
                .require(&[
                    "cafleet fleet create",
                    "cafleet member create",
                    "cafleet message poll <director-member-id> --json",
                    "--json",
                ])
                .unwrap();
            workflow.require_pattern(
                "native reception is armed before member replies",
                r"(?is)arm native reception before depending on member ready replies",
            );
            workflow.require_pattern(
                "one receiver spans dispatch and replies",
                r"(?is)retain one receiver through dispatch and replies",
            );
            workflow.require_pattern(
                "poll on each native batch or wake",
                r"(?is)On every received batch/wake",
            );
            workflow.require_pattern(
                "deferred work is recorded before ACK",
                r"(?is)reconcile replay.{0,80}record deferred actions.{0,50}ACK consumption",
            );
            workflow.require_pattern(
                "host continuation and user decision gates are preserved",
                r"(?is)Preserve user decision gates and host continuation controls",
            );
            INVENTORY
                .require_link(
                    &workflow,
                    "../../cafleet/reference/supervision.md#director-wake",
                )
                .unwrap();
            INVENTORY
                .require_link(&workflow, "../../cafleet/reference/coding-agents.md")
                .unwrap();
        }
        INVENTORY
            .document(CREATE)
            .section("step-2-clarification-phase-director")
            .unwrap()
            .require(&["Drafter", "questions", "answers"])
            .unwrap();
        INVENTORY
            .document(CREATE)
            .section("director-responsibilities")
            .unwrap()
            .require(&["Drafter", "Reviewer", "explicit user approval"])
            .unwrap();
        let execute = INVENTORY.document(EXECUTE);
        execute
            .require(&[
                "Phase A",
                "Phase B",
                "Phase C",
                "Phase D",
                "Step 5",
                "Step 6",
                "COMMENT(user-relay)",
            ])
            .unwrap();
        let analyzer = INVENTORY.document("skills/cafleet-design-doc/interview/roles/analyzer.md");
        analyzer
            .require(&["Total: N questions", "--file -"])
            .unwrap();
        analyzer.require_pattern(
            "Analyzer remains read-only",
            r"(?is)(read.only|no.file.edit|do NOT.{0,30}edit)",
        );
    }

    #[test]
    fn local_owner_links_and_runtime_mirrors_resolve() {
        for (path, text) in &INVENTORY.sources {
            if !path.ends_with(".md") && !path.ends_with(".mdx") {
                continue;
            }
            let source = INVENTORY.document(path);
            let mut fenced = false;
            for (offset, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                    fenced = !fenced;
                    continue;
                }
                if fenced {
                    continue;
                }
                for link in LINKS.captures_iter(line) {
                    let matched = link.get(0).unwrap();
                    if INLINE.find_iter(line).any(|literal| {
                        literal.start() <= matched.start() && literal.end() >= matched.end()
                    }) {
                        continue;
                    }
                    let target = &link[1];
                    if target.contains("://")
                        || target.starts_with("mailto:")
                        || target.starts_with('/')
                        || target.contains(['$', '{', '}'])
                        || target.contains('<') && !target.ends_with("#<name>")
                    {
                        continue;
                    }
                    INVENTORY.link(&source, offset + 1, target).unwrap();
                }
            }
        }
    }

    #[test]
    fn site_navigation_entries_resolve_to_pages_or_directories() {
        for path in INVENTORY
            .sources
            .keys()
            .filter(|path| path.starts_with("docs/docs/") && path.ends_with("/_meta.json"))
        {
            let document = INVENTORY.document(path);
            let entries: serde_json::Value = serde_json::from_str(document.text).unwrap();
            let mut names = BTreeSet::new();
            for entry in entries.as_array().expect("navigation is an array") {
                let name = entry["name"].as_str().expect("navigation entry has a name");
                assert!(names.insert(name), "{path}: duplicate page {name}");
                let parent = root().join(path).parent().unwrap().to_path_buf();
                match entry["type"].as_str() {
                    Some("dir") => assert!(
                        parent.join(name).is_dir(),
                        "{path}: missing directory {name}"
                    ),
                    Some("file") | None => {
                        INVENTORY.link(&document, 1, &format!("{name}.md")).unwrap()
                    }
                    Some(kind) => panic!("{path}: unknown navigation type {kind}"),
                }
            }
        }
        let config = std::fs::read_to_string(root().join("docs/rspress.config.ts")).unwrap();
        assert!(config.contains("checkDeadLinks: true"));
    }

    #[test]
    fn human_documentation_keeps_its_independent_audience() {
        for (path, text) in &INVENTORY.sources {
            if !path.starts_with("docs/docs/") {
                continue;
            }
            let source = INVENTORY.document(path);
            for (offset, line) in text.lines().enumerate() {
                for link in LINKS.captures_iter(line) {
                    assert!(
                        !link[1].contains("skills/cafleet"),
                        "{}",
                        source.error(
                            offset + 1,
                            "human docs explain behavior through their own owner pages"
                        )
                    );
                }
            }
        }
        let backend = INVENTORY.document(BACKENDS);
        for link in LINKS.captures_iter(backend.text) {
            assert!(!link[1].contains("docs/docs/spec/coding-agent-backends"));
        }
    }

    #[test]
    fn permissions_documentation_covers_message_wait_and_director_capture() {
        let coverage = INVENTORY
            .document("docs/docs/spec/cli-options.md")
            .section("permissionsallow-coverage")
            .unwrap();
        coverage
            .require(&["cafleet message wait", "cafleet fleet capture"])
            .unwrap();
        for (path, permission) in [
            (
                "presets/codex/cafleet.rules",
                "pattern = [\"cafleet\"], decision = \"allow\"",
            ),
            ("presets/opencode/cafleet.md", "\"cafleet *\": \"allow\""),
        ] {
            let text = std::fs::read_to_string(root().join(path)).unwrap();
            assert!(
                text.contains(permission),
                "{path}: retain member broker capability"
            );
        }
    }

    #[test]
    fn opencode_preset_declares_the_shell_allowlist_and_member_dispatch_denials() {
        let preset = std::fs::read_to_string(root().join("presets/opencode/cafleet.md")).unwrap();
        let frontmatter = preset
            .strip_prefix("---\n")
            .and_then(|text| text.split_once("\n---\n"))
            .expect("the opencode preset has delimited frontmatter")
            .0;
        assert!(frontmatter.contains("deny-by-default shell allowlist"));
        let shell = frontmatter
            .split_once("    \"shell\": {\n")
            .expect("the opencode preset declares its command rules under shell")
            .1
            .split_once("\n    },")
            .expect("the shell ruleset has a closing delimiter")
            .0;
        assert!(shell.starts_with("      \"*\": \"deny\","));
        let broker = shell.find("\"cafleet *\": \"allow\"").unwrap();
        for dispatch in ["prompt", "exec"] {
            let denial = shell
                .find(&format!("\"cafleet member {dispatch} *\": \"deny\""))
                .unwrap();
            assert!(
                broker < denial,
                "the member dispatch denial follows its broker allow rule"
            );
        }
        assert!(preset.contains("Shell defaults to deny"));
    }

    #[test]
    fn opencode_preset_matches_the_verbatim_specification() {
        let preset = std::fs::read_to_string(root().join("presets/opencode/cafleet.md")).unwrap();
        let specification = INVENTORY
            .document("SPEC.md")
            .section("exact-preset-file-contents-verbatim")
            .unwrap();
        let verbatim = specification
            .text
            .split_once("````markdown\n")
            .expect("the specification includes the verbatim preset fence")
            .1
            .split_once("````")
            .expect("the verbatim preset fence is closed")
            .0;
        assert_eq!(preset, verbatim);
    }

    #[test]
    fn migration_rule_points_to_the_current_chain_guard() {
        INVENTORY
            .document(".claude/rules/database-migrations.md")
            .require(&["cafleet/src/db/mod.rs", "embedded", "V1__baseline.sql"])
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
                        if !["cafleet/", "skills/", "docs/", "presets/"]
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
                "missing binding pane_title",
            ),
            (
                "duplicate",
                "| Placeholder | Value |\n|---|---|\n| `{pane_title}` | title |\n| `{pane_title}` | again |\n",
                7,
                "unknown or duplicate binding pane_title",
            ),
            (
                "empty",
                "| Placeholder | Value |\n|---|---|\n| `{pane_title}` | |\n",
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
            let error = region.bindings(&["pane_title"]).unwrap_err();
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
            text: "# `asset_installs`\n",
            line: 4,
            section: "document".into(),
        };
        assert_eq!(simple.section("asset_installs").unwrap().line, 4);
    }
}
