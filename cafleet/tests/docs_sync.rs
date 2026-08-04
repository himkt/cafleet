//! Docs-sync contracts: the repository documentation is the public contract
//! for the Director-tick supervision protocol — the fleet-level `[cafleet]
//! tick:` wake into the Director's pane, the resume clause on both injected
//! triggers, the `member ping` pending-placement skip, the `member capture`
//! pane read, and the flattened `monitor` command.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the package sits under the repo root")
        .to_path_buf()
}

fn read(relative_path: &str) -> String {
    let path = root().join(relative_path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn normalize(text: &str) -> String {
    regex::Regex::new("[-‐‑–—]+")
        .unwrap()
        .replace_all(text, " ")
        .to_string()
}

fn assert_terms(relative_path: &str, terms: &[&str]) {
    let text = read(relative_path).to_lowercase();
    let normalized_text = normalize(&text);
    let missing: Vec<&str> = terms
        .iter()
        .filter(|term| {
            let term = term.to_lowercase();
            !text.contains(&term) && !normalized_text.contains(&normalize(&term))
        })
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "{relative_path} is missing required terms: {missing:?}"
    );
}

fn assert_absent(relative_path: &str, terms: &[&str]) {
    let text = read(relative_path).to_lowercase();
    let present: Vec<&str> = terms
        .iter()
        .filter(|term| text.contains(&term.to_lowercase()))
        .copied()
        .collect();
    assert!(
        present.is_empty(),
        "{relative_path} still mentions removed terms: {present:?}"
    );
}

// Episode-machine and monitoring-member vocabulary that must not survive on
// any contract page.
const REMOVED_VOCABULARY: [&str; 17] = [
    "nudge_claimed",
    "escalation_pending",
    "ping_failed",
    "ping_interrupted",
    "unchanged_after_nudge",
    "stall_episode_state",
    "monitor_report_delivery",
    "monitor_director_gate",
    "report-batch",
    "monitor stall",
    "monitoring member",
    "monitoring-member",
    "--role monitor",
    "ready: monitor live",
    "monitor_config",
    "CAFLEET_MONITOR_STALL_INTERVAL",
    "stall-check",
];

// The pre-simplification CLI surface: command forms and flags deleted by the
// CLI interface simplification must not survive on any contract page.
const OLD_CLI_SURFACE: [&str; 8] = [
    "monitor capture",
    "monitor start",
    "--full",
    "--quiet",
    "--text-file",
    "--no-ansi",
    "--member-id",
    "--message-id",
];

#[test]
fn monitoring_concept_covers_the_director_tick_and_capture_taxonomy() {
    assert_terms(
        "docs/docs/concepts/monitoring.md",
        &[
            "awaiting_user",
            "finished",
            "working",
            "stall_candidate",
            "pre-ping capture gate",
            "[cafleet] tick:",
            "health-check",
            "Resume your work if something was still running",
            "CAFLEET_MONITOR_WAKE_INTERVAL",
            "--interval 0",
            "Esc",
            "monitor loop started",
            "member ping",
        ],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/docs/concepts/monitoring.md", &absent);
}

#[test]
fn spec_defines_the_ping_skip_and_flattened_monitor_contract() {
    assert_terms(
        "SPEC.md",
        &[
            "skipped",
            "pending placement",
            "member capture",
            "monitor loop started",
            "[cafleet] tick:",
            "health-check",
            "last_wake_at",
            "CAFLEET_MONITOR_WAKE_INTERVAL",
            "then resume your work",
        ],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("SPEC.md", &absent);
}

#[test]
fn data_model_defines_the_monitor_runtime() {
    assert_terms(
        "docs/docs/spec/data-model.md",
        &["monitor_runtime", "last_wake_at", "tick_seconds"],
    );
    let mut absent = vec!["last_stall_candidate_at", "last_stall_capture_sha256"];
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/docs/spec/data-model.md", &absent);
}

#[test]
fn cli_options_defines_the_ping_skip_and_moved_capture() {
    assert_terms(
        "docs/docs/spec/cli-options.md",
        &[
            "member capture",
            "ping skipped",
            "skipped",
            "pending placement",
            "nothing to",
            "--interval",
            "CAFLEET_MONITOR_WAKE_INTERVAL",
            "then resume your work",
        ],
    );
    let mut absent = vec!["monitor status", "monitor config"];
    absent.extend(OLD_CLI_SURFACE);
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/docs/spec/cli-options.md", &absent);
}

#[test]
fn multiplexer_backends_pins_the_pure_trigger_payload() {
    assert_terms(
        "docs/docs/spec/multiplexer-backends.md",
        &[
            "[cafleet] tick:",
            "coding_agent=",
            "Resume your work if something was still running",
            "cafleet message poll <member-id> — then",
        ],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/docs/spec/multiplexer-backends.md", &absent);
}

#[test]
fn every_backend_overlay_defines_the_capture_cues() {
    for backend in ["claude", "codex", "opencode"] {
        let overlay = format!("skills/cafleet/reference/coding-agent/{backend}-overlay.md");
        assert_terms(
            &overlay,
            &[
                "working",
                "stall_candidate",
                "quiet",
                "ambiguous",
                "pre-ping capture gate",
            ],
        );
        let mut absent = vec!["pre-nudge"];
        absent.extend(REMOVED_VOCABULARY);
        assert_absent(&overlay, &absent);
    }
    assert_terms(
        "skills/cafleet/reference/coding-agent/_template.md",
        &["working", "stall_candidate", "Note → applies at"],
    );
}

#[test]
fn the_supervision_contract_covers_quiet_members_and_plain_messages() {
    assert_terms(
        "skills/cafleet/reference/supervision.md",
        &[
            "member ping",
            "quiet",
            "finished",
            "pre-ping",
            "member capture",
            "cafleet monitor",
            "monitor loop started",
            "health-check",
        ],
    );
    let mut absent = vec!["pre-nudge"];
    absent.extend(OLD_CLI_SURFACE);
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/reference/supervision.md", &absent);
}

#[test]
fn the_director_and_member_roles_keep_the_ping_protocol() {
    assert_terms(
        "skills/cafleet/reference/director.md",
        &[
            "## Member Ping (manual inbox-poll)",
            "member ping",
            "pre-ping",
            "member capture",
            "then resume your work",
        ],
    );
    let mut absent = vec!["(manual inbox-poll nudge)", "pre-nudge"];
    absent.extend(OLD_CLI_SURFACE);
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/reference/director.md", &absent);

    assert_terms(
        "skills/cafleet/roles/member.md",
        &["member ping", "member prompt", "Director"],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/roles/member.md", &absent);
}

#[test]
fn the_cafleet_skill_and_bash_rule_document_the_director_ping() {
    assert_terms(
        "skills/cafleet/SKILL.md",
        &[
            "cafleet monitor",
            "monitor loop started",
            "member ping",
            "message send",
            "health-check",
        ],
    );
    let mut skill_absent = OLD_CLI_SURFACE.to_vec();
    skill_absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/SKILL.md", &skill_absent);

    assert_terms(
        ".claude/rules/bash-tool.md",
        &[
            "member ping",
            "member prompt",
            "Esc",
            "message poll",
            "then resume your work",
        ],
    );
    let mut absent = vec!["action = ping"];
    absent.extend(OLD_CLI_SURFACE);
    absent.extend(REMOVED_VOCABULARY);
    assert_absent(".claude/rules/bash-tool.md", &absent);
}

#[test]
fn fixed_ping_surfaces_carry_no_nudge_vocabulary() {
    // The Director's message-level stall-nudge concept lives only in the
    // cafleet-design-doc skill's coordination protocol.
    for relative_path in [
        "skills/cafleet/reference/recovery.md",
        "skills/cafleet/reference/prompt-routing.md",
        "skills/cafleet/reference/director.md",
        "docs/docs/concepts/overview.md",
    ] {
        assert_absent(relative_path, &["nudge"]);
    }
}

#[test]
fn the_readme_and_webui_api_stay_free_of_internal_monitor_state() {
    let mut readme_absent = vec!["stall_candidate"];
    readme_absent.extend(REMOVED_VOCABULARY);
    assert_absent("README.md", &readme_absent);
    assert_absent("docs/docs/spec/webui-api.md", &REMOVED_VOCABULARY);
}

// ---------------------------------------------------------------------------
// Structural guards over the `skills/` tree.
//
// These are static checks over the whole tree rather than per-page term
// assertions: they fail when a skill file points at a path that does not
// exist, drops the gated overlay read from its Required-reading block, or
// introduces a `{token}` that no overlay defines.
// ---------------------------------------------------------------------------

fn skill_markdown_files() -> Vec<String> {
    let mut files = Vec::new();
    collect_markdown(&root().join("skills"), &mut files);
    files.sort();
    files
}

fn collect_markdown(dir: &Path, out: &mut Vec<String>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read directory {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            let relative = path
                .strip_prefix(root())
                .expect("every skill file sits under the repo root");
            out.push(relative.to_string_lossy().into_owned());
        }
    }
}

/// Prose lines paired with their 1-based line number, with fenced blocks
/// dropped: those hold sample code (Python format strings, LaTeX, shell) whose
/// braces and slashes are not documentation references.
fn prose_lines(text: &str) -> Vec<(usize, String)> {
    let mut kept = Vec::new();
    let mut inside_fence = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            inside_fence = !inside_fence;
            continue;
        }
        if !inside_fence {
            kept.push((index + 1, line.to_string()));
        }
    }
    kept
}

fn inline_code_spans(text: &str) -> Vec<String> {
    regex::Regex::new(r"`([^`\n]+)`")
        .unwrap()
        .captures_iter(text)
        .map(|captures| captures[1].to_string())
        .collect()
}

/// First path segments that name a real top-level directory of the repo.
///
/// `design-docs/` is deliberately absent: every `design-docs/...` mention in
/// `skills/` is an illustrative user-supplied argument (`design-docs/0000060-foo`),
/// not a reference to a file that must exist.
const REPO_RELATIVE_PREFIXES: [&str; 5] = ["cafleet/", "skills/", "docs/", "admin/", "presets/"];

fn path_candidates(span: &str) -> Vec<String> {
    let trailing_locator = regex::Regex::new(r":\d+(-\d+)?$").unwrap();
    span.split_whitespace()
        .map(|raw| {
            let token = raw.trim_start_matches(['(', '[', '{', '<', '"', '\'']);
            let token = token.trim_end_matches([')', ']', '}', '"', '\'', ',', ';', '.', '!', '?']);
            let token = token
                .split('#')
                .next()
                .expect("split always yields at least one segment");
            trailing_locator.replace(token, "").into_owned()
        })
        .collect()
}

/// A candidate is checked only when it reads as a repo-relative path. URLs,
/// globs, `<placeholder>` forms, and `${VAR}` interpolations are legitimate
/// slash-bearing text that names no fixed file.
fn looks_repo_relative(candidate: &str) -> bool {
    REPO_RELATIVE_PREFIXES
        .iter()
        .any(|prefix| candidate.starts_with(prefix))
        && !candidate.contains("://")
        && !candidate.contains(['*', '<', '>', '$', '{', '}', '~', '|'])
}

/// Skill pages address each other by skill-relative path (`cafleet/SKILL.md`)
/// and address the repo by repo-relative path (`docs/docs/spec/cli-options.md`),
/// so a candidate resolving under either root is a live reference.
fn resolves_somewhere(candidate: &str) -> bool {
    root().join(candidate).exists() || root().join("skills").join(candidate).exists()
}

#[test]
fn skill_files_reference_no_path_that_is_missing_from_disk() {
    let mut dangling = Vec::new();
    for relative_path in skill_markdown_files() {
        for (line_number, line) in prose_lines(&read(&relative_path)) {
            for span in inline_code_spans(&line) {
                for candidate in path_candidates(&span) {
                    if looks_repo_relative(&candidate) && !resolves_somewhere(&candidate) {
                        dangling.push(format!("{relative_path}:{line_number} → {candidate}"));
                    }
                }
            }
        }
    }
    assert!(
        dangling.is_empty(),
        "skill files reference repository paths that do not exist: {dangling:#?}"
    );
}

/// The one role file that carries no Required-reading block: it is an agent
/// spec pasted verbatim into a dispatched sub-agent's prompt, not an entry
/// point for a spawned member.
const ROLE_FILE_WITHOUT_REQUIRED_READING: &str =
    "skills/cafleet-research/report/roles/web-researcher.md";

#[test]
fn every_role_file_gates_its_overlay_as_required_reading_row_one() {
    assert!(
        root().join(ROLE_FILE_WITHOUT_REQUIRED_READING).exists(),
        "the exempt role file {ROLE_FILE_WITHOUT_REQUIRED_READING} moved — retarget the exemption"
    );

    let heading = regex::Regex::new(r"(?m)^#+[ \t]+Required[ -]reading").unwrap();
    let mut offenders = Vec::new();
    for relative_path in skill_markdown_files() {
        let text = read(&relative_path);
        // A role file must *have* the block: folding content out of one must
        // never carry the gated overlay read away with it. Other skill pages
        // are checked for content only, since not all of them gate reads.
        let block_is_mandatory = relative_path.contains("/roles/")
            && relative_path != ROLE_FILE_WITHOUT_REQUIRED_READING;
        let Some(block) = heading.find(&text) else {
            if block_is_mandatory {
                offenders.push(format!(
                    "{relative_path} → role file has no Required-reading block"
                ));
            }
            continue;
        };
        match text[block.end()..]
            .lines()
            .find(|line| line.trim_start().starts_with("| 1 |"))
        {
            None => offenders.push(format!("{relative_path} → no row #1 in the block")),
            Some(row) if !row.contains("overlay") => {
                offenders.push(format!(
                    "{relative_path} → row #1 does not name the overlay"
                ));
            }
            Some(_) => {}
        }
    }
    assert!(
        offenders.is_empty(),
        "every role file must gate the reader's overlay as Required-reading row #1: {offenders:#?}"
    );
}

/// The nine placeholders every backend overlay resolves.
const OVERLAY_PLACEHOLDERS: [&str; 9] = [
    "decision_surface",
    "reviewer_model",
    "permission_flags",
    "bg_run",
    "bg_stop",
    "task_coord",
    "pane_title",
    "skill_loader",
    "effort_levels",
];

/// The four identity placeholders `cafleet member create` substitutes at spawn.
const FORMAT_PLACEHOLDERS: [&str; 4] = [
    "fleet_id",
    "member_id",
    "director_member_id",
    "coding_agent",
];

/// Brace tokens that are deliberately not overlay placeholders. Each has a
/// documented home outside the overlay mechanism, so a token entering the tree
/// without one still fails this check.
const NON_OVERLAY_TOKENS: [&str; 7] = [
    // Meta-references: prose describing the resolution rule itself.
    "token",
    "placeholder",
    // Workflow-local path variables.
    "slug",
    "dir_path",
    // web-researcher discovery-query examples.
    "topic",
    "current_year",
    "current_month",
];

#[test]
fn every_brace_token_in_skills_belongs_to_the_known_vocabulary() {
    let token = regex::Regex::new(r"\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    let mut unknown = Vec::new();
    for relative_path in skill_markdown_files() {
        for (_, line) in prose_lines(&read(&relative_path)) {
            for captures in token.captures_iter(&line) {
                let occurrence = captures.get(0).expect("the whole match");
                // `${VAR}` is shell interpolation and `@{upstream}` is git
                // revision syntax — neither draws from the overlay vocabulary.
                if matches!(
                    line[..occurrence.start()].chars().next_back(),
                    Some('$' | '@')
                ) {
                    continue;
                }
                let name = &captures[1];
                let known = OVERLAY_PLACEHOLDERS.contains(&name)
                    || FORMAT_PLACEHOLDERS.contains(&name)
                    || NON_OVERLAY_TOKENS.contains(&name);
                if !known {
                    unknown.push(format!("{relative_path} → {{{name}}}"));
                }
            }
        }
    }
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "these tokens resolve to a literal brace at spawn — give each an overlay value \
         or a documented home: {unknown:#?}"
    );
}

#[test]
fn every_backend_overlay_defines_the_full_placeholder_vocabulary() {
    for overlay in [
        "claude-overlay.md",
        "codex-overlay.md",
        "opencode-overlay.md",
        "_template.md",
    ] {
        let relative_path = format!("skills/cafleet/reference/coding-agent/{overlay}");
        let text = read(&relative_path);
        let missing: Vec<&str> = OVERLAY_PLACEHOLDERS
            .iter()
            .filter(|placeholder| !text.contains(&format!("{{{placeholder}}}")))
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "{relative_path} leaves placeholders undefined: {missing:?}"
        );
    }
}
