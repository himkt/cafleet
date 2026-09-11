//! Docs-sync contracts: the repository documentation is the public contract
//! for the monitor-member supervision protocol — the fleet-level `[cafleet]
//! tick:` wake into the monitor member's pane, the on-wake classification
//! with the fixed-ping exception, the `monitor live` spawn gate, the resume
//! clause on both injected triggers, the `member ping` pending-placement
//! skip, the `member capture` pane read, and the flattened `monitor` command.

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

fn assert_terms_in(context: &str, text: &str, terms: &[&str]) {
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
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
        "{context} is missing required terms: {missing:?}"
    );
}

fn assert_terms(relative_path: &str, terms: &[&str]) {
    assert_terms_in(relative_path, &read(relative_path), terms);
}

const OVERLAYS_FILE: &str = "skills/cafleet/reference/coding-agents.md";

/// Slice a markdown file at top-level `## ` boundaries.
/// Panics (test failure) when the named section is missing.
fn markdown_section<'a>(context: &str, text: &'a str, name: &str) -> &'a str {
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        match (start, line.trim_end().strip_prefix("## ")) {
            (None, Some(title)) if title == name => start = Some(offset),
            (Some(section_start), Some(_)) => return &text[section_start..offset],
            _ => {}
        }
        offset += line.len();
    }
    match start {
        Some(section_start) => &text[section_start..],
        None => panic!("{context} has no top-level `## {name}` section"),
    }
}

fn overlay_section<'a>(text: &'a str, name: &str) -> &'a str {
    markdown_section(OVERLAYS_FILE, text, name)
}

fn backend_subsection<'a>(text: &'a str, backend: &str, name: &str) -> &'a str {
    let body = overlay_section(text, backend);
    let heading = format!("### {name}\n");
    let start = body
        .find(&heading)
        .unwrap_or_else(|| panic!("{OVERLAYS_FILE} § {backend} is missing subsection {name}"))
        + heading.len();
    let remainder = &body[start..];
    match remainder.find("\n### ") {
        Some(end) => &remainder[..end],
        None => remainder,
    }
}

fn table_rows(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .filter(|line| line.starts_with('|'))
        .map(|line| line.trim_matches('|').split('|').map(str::trim).collect())
        .collect()
}

fn section_at_anchor<'a>(path: &str, text: &'a str, anchor: &str) -> &'a str {
    let heading = regex::Regex::new(r"^(#{1,6}) +(.+?)\s*$").unwrap();
    let explicit_id = regex::Regex::new(r"\{#([^}]+)\}\s*$").unwrap();
    let mut offset = 0;
    let mut start = None;
    let mut inside_fence = false;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            inside_fence = !inside_fence;
        }
        if !inside_fence && let Some(captures) = heading.captures(line.trim_end()) {
            let depth = captures[1].len();
            if let Some((begin, parent_depth)) = start {
                if depth <= parent_depth {
                    return &text[begin..offset];
                }
            } else {
                let title = &captures[2];
                let id = match explicit_id.captures(title) {
                    Some(id) => id[1].to_string(),
                    None => title
                        .to_lowercase()
                        .chars()
                        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-')
                        .map(|c| if c.is_whitespace() { '-' } else { c })
                        .collect(),
                };
                if id == anchor {
                    start = Some((offset, depth));
                }
            }
        }
        offset += line.len();
    }
    let (begin, _) = start.unwrap_or_else(|| panic!("{path} has no heading for #{anchor}"));
    &text[begin..]
}

fn assert_section_terms(path: &str, anchor: &str, terms: &[&str]) {
    let text = read(path);
    let section = section_at_anchor(path, &text, anchor)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert_terms_in(&format!("{path}#{anchor}"), &section, terms);
}

fn assert_required_link(path: &str, target: &str, action: &str) {
    let text = read(path);
    let required = section_at_anchor(path, &text, "required-reading");
    let row = required
        .lines()
        .find(|line| line.contains(target))
        .unwrap_or_else(|| panic!("{path} must require {target} before {action}"));
    assert_terms_in(path, row, &[action]);
    assert_terms_in(path, required, &["before"]);
}

fn assert_contract_pattern(context: &str, text: &str, pattern: &str) {
    let compact = text
        .replace('`', "")
        .replace("**", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .unwrap();
    assert!(
        regex.is_match(&compact),
        "{context} must state the contract matching {pattern:?}"
    );
}

#[test]
fn workflow_create_finalization_requires_the_exact_director_approval_route() {
    let route = "ready (doc) — user approved; finalize";
    let create = "skills/cafleet-design-doc/create/create.md";
    let drafter = "skills/cafleet-design-doc/create/roles/drafter.md";
    for path in [create, drafter] {
        assert!(
            read(path).contains(route),
            "{path} must use the explicit finalization route"
        );
    }
    assert_section_terms(
        create,
        "step-6-finalize--clean-up-director",
        &["Reviewer", "user approval", "COMMENT(", "addressed (doc)"],
    );
    let text = read(drafter);
    let workflow = section_at_anchor(drafter, &text, "workflow");
    assert_contract_pattern(
        drafter,
        workflow,
        r"finaliz.{0,700}(?:marker.free|no.{0,30}COMMENT|absence.{0,30}marker)",
    );
    assert_terms_in(
        drafter,
        workflow,
        &[
            "Approved",
            "Last Updated",
            "Progress",
            "actionable",
            "addressed (doc)",
        ],
    );
}

#[test]
fn workflow_plain_ready_with_markers_routes_drafter_revision() {
    let path = "skills/cafleet-design-doc/create/roles/drafter.md";
    let text = read(path);
    let workflow = section_at_anchor(path, &text, "workflow");
    assert_contract_pattern(
        path,
        workflow,
        r"plain.{0,60}ready \(doc\).{0,350}(?:marker|COMMENT).{0,350}(?:revise|resolv|process).{0,350}addressed \(doc\)",
    );
}

#[test]
fn workflow_plain_ready_without_markers_preserves_metadata_and_ends_the_turn() {
    let path = "skills/cafleet-design-doc/create/roles/drafter.md";
    let text = read(path);
    let workflow = section_at_anchor(path, &text, "workflow");
    assert_contract_pattern(
        path,
        workflow,
        r"(?:no|without).{0,40}(?:marker|COMMENT).{0,300}(?:(?:preserv|unchanged).{0,100}(?:metadata|Status)|(?:metadata|Status).{0,100}unchanged)",
    );
    assert_contract_pattern(
        path,
        workflow,
        r"(?:no|without).{0,40}(?:marker|COMMENT).{0,500}(?:end.{0,30}turn|idle)",
    );
    assert_contract_pattern(
        path,
        workflow,
        r"plain.{0,60}ready \(doc\).{0,900}(?:acknowledge|ACK)",
    );
}

#[test]
fn workflow_create_resume_and_review_only_modes_rejoin_the_quality_loop() {
    let path = "skills/cafleet-design-doc/create/create.md";
    let text = read(path);
    let review = section_at_anchor(path, &text, "step-3-internal-quality-loop-director");
    assert_terms_in(
        path,
        review,
        &[
            "complete (doc)",
            "addressed (doc)",
            "resume",
            "QUALITY_REVIEW_ONLY",
        ],
    );
    assert_contract_pattern(
        path,
        &text,
        r"review.only.{0,400}(?:wait.{0,60}revision|revision.{0,60}wait)",
    );
    let drafter = "skills/cafleet-design-doc/create/roles/drafter.md";
    assert_section_terms(
        drafter,
        "resume-mode",
        &["entire document", "markers", "addressed (doc)"],
    );
}

#[test]
fn workflow_specialized_role_roster_keeps_seven_independent_startup_contracts() {
    let expected = [
        "create/roles/drafter.md",
        "create/roles/reviewer.md",
        "execute/roles/programmer.md",
        "execute/roles/reviewer.md",
        "execute/roles/tester.md",
        "execute/roles/verifier.md",
        "interview/roles/analyzer.md",
    ];
    let prefix = "skills/cafleet-design-doc/";
    let actual: Vec<_> = skill_markdown_files()
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(prefix)
                .filter(|path| path.contains("/roles/"))
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(actual, expected);
    for relative in expected {
        let path = format!("{prefix}{relative}");
        assert_required_link(&path, "coding-agents.md", "resolve");
        assert_required_link(&path, "base-dir.md", "BASE");
        assert_required_link(&path, "coordination.md", "coordination");
    }
}

#[test]
fn workflow_director_owners_keep_diagnostics_composition_and_separate_phase_commits() {
    for path in [
        "skills/cafleet-design-doc/create/create.md",
        "skills/cafleet-design-doc/execute/execute.md",
    ] {
        assert_section_terms(
            path,
            "director-responsibilities",
            &["Director", "review", "approval"],
        );
        let text = read(path);
        assert!(text.contains("cafleet member capture <member-id> --lines 200"));
        assert_contract_pattern(
            path,
            &text,
            r"(?:silently|disclos|inform|tell).{0,160}(?:re.spawn|replac)|(?:re.spawn|replac).{0,160}(?:silently|disclos|inform|tell)",
        );
    }
    let execute = "skills/cafleet-design-doc/execute/execute.md";
    assert_section_terms(
        execute,
        "team-composition",
        &["Programmer", "Tester", "Verifier", "documentation"],
    );
    assert_section_terms(
        execute,
        "commit-protocol",
        &[
            "Director",
            "tests",
            "implementation",
            "separate",
            "eligible",
        ],
    );
}

#[test]
fn workflow_payload_receivers_require_json_for_all_three_exemptions() {
    let coordination = "skills/cafleet-design-doc/reference/coordination.md";
    let text = read(coordination);
    for pattern in [
        r"clarification.{0,500}(?:exempt|free.form)|(?:exempt|free.form).{0,500}clarification",
        r"Analyzer",
        r"Verifier.{0,120}tool.discovery|tool.discovery.{0,120}Verifier",
        r"--json.{0,300}(?:full|complete)|(?:full|complete).{0,300}--json",
    ] {
        assert_contract_pattern(coordination, &text, pattern);
    }
    for (path, anchor) in [
        (
            "skills/cafleet-design-doc/create/create.md",
            "step-2-clarification-phase-director",
        ),
        (
            "skills/cafleet-design-doc/interview/interview.md",
            "2e-wait-for-the-analyzers-question-list",
        ),
    ] {
        assert_section_terms(path, anchor, &["--json", "cafleet message", "ack"]);
    }
    let execute = "skills/cafleet-design-doc/execute/execute.md";
    assert_contract_pattern(
        execute,
        &read(execute),
        r"(?:tool.discovery.{0,600}--json|--json.{0,600}tool.discovery)",
    );
}

#[test]
fn workflow_no_tester_composition_routes_test_findings_in_both_review_loops() {
    let path = "skills/cafleet-design-doc/execute/execute.md";
    let text = read(path);
    for anchor in ["review-loop", "revision-loop-comment-marker-based-feedback"] {
        let section = section_at_anchor(path, &text, anchor);
        assert_contract_pattern(
            path,
            section,
            r"(?:no Tester|Tester.{0,30}not spawned).{0,180}Programmer|Programmer.{0,180}(?:no Tester|Tester.{0,30}not spawned)",
        );
        assert_terms_in(path, section, &["test", "ready ("]);
    }
}

#[test]
fn workflow_local_approval_survives_upstream_tracking_at_finalization() {
    let path = "skills/cafleet-design-doc/execute/execute.md";
    let text = read(path);
    let finalization = section_at_anchor(path, &text, "step-8-finalize--clean-up-director");
    assert_contract_pattern(
        path,
        finalization,
        r"approve.local.{0,400}(?:skip|no|without).{0,80}push",
    );
    assert_contract_pattern(
        path,
        finalization,
        r"(?:regardless|even|despite).{0,100}(?:upstream|track)|(?:upstream|track).{0,100}(?:regardless|even|despite)",
    );
    assert_contract_pattern(
        path,
        finalization,
        r"(?:remote|push).{0,80}authoriz|authoriz.{0,80}(?:remote|push)",
    );
}

#[test]
fn workflow_user_revisions_require_reviewer_approval_before_representation() {
    let create = "skills/cafleet-design-doc/create/create.md";
    assert_section_terms(
        create,
        "step-5-user-feedback-loop-director",
        &["COMMENT(user-relay)", "quality loop", "re-present"],
    );
    let execute = "skills/cafleet-design-doc/execute/execute.md";
    let text = read(execute);
    let revisions = section_at_anchor(
        execute,
        &text,
        "revision-loop-comment-marker-based-feedback",
    );
    assert_terms_in(
        execute,
        revisions,
        &["COMMENT(user-relay)", "ready (doc)", "approved (doc)"],
    );
    assert_contract_pattern(
        execute,
        revisions,
        r"(?:revision|revised).{0,600}Reviewer.{0,400}(?:before|only after)",
    );
}

#[test]
fn workflow_guidelines_own_design_doc_path_normalization() {
    let path = "skills/cafleet-design-doc/reference/guidelines.md";
    assert_section_terms(
        path,
        "file-layout",
        &[
            "/design-doc.md",
            "design-docs/",
            "strip",
            "prepend",
            "absolute",
            "base-dir.md",
        ],
    );
    for workflow in [
        "create/create.md",
        "execute/execute.md",
        "interview/interview.md",
    ] {
        let path = format!("skills/cafleet-design-doc/{workflow}");
        assert_required_link(
            &path,
            "guidelines.md#file-layout",
            "Before argument normalization",
        );
    }
}

#[test]
fn shared_director_role_owns_spawn_and_action_headings() {
    let path = "skills/cafleet/roles/director.md";
    let text = read(path);
    for anchor in [
        "model-selection",
        "model-name-to-backend-inference",
        "canonical-spawn-prompt-skeleton",
        "spawn-prompt-size-limit",
        "member-create--scratch-and-audit-files",
        "model-replacement",
        "member-prompt",
        "member-ping-manual-inbox-poll",
        "answering-a-members-relayed-question",
    ] {
        section_at_anchor(path, &text, anchor);
    }
    assert_section_terms(
        path,
        "canonical-spawn-prompt-skeleton",
        &[
            "ROLE DEFINITION:",
            "FLEET ID: {fleet_id}",
            "DIRECTOR MEMBER ID: {director_member_id}",
            "YOUR MEMBER ID: {member_id}",
            "CODING AGENT: {coding_agent}",
            "IMPORTANT:",
            "verbatim",
        ],
    );
    assert_section_terms(
        path,
        "member-create--scratch-and-audit-files",
        &[".prompts/", "UTC", "suffix", "substitution", "--file"],
    );
    assert_section_terms(
        path,
        "spawn-prompt-size-limit",
        &["--file", "argv", "cleanup"],
    );
    assert_section_terms(
        path,
        "model-replacement",
        &["same backend", "two", "user", "pinned", "evidence"],
    );
}

#[test]
fn shared_director_requires_supervision_and_action_specific_recovery_reads() {
    let path = "skills/cafleet/roles/director.md";
    assert_required_link(path, "../reference/supervision.md", "supervision");
    assert_required_link(path, "../reference/supervision.md#recovery", "recover");
    assert_required_link(path, "../reference/supervision.md#shutdown", "tear");
    assert_required_link(path, "../reference/prompt-routing.md", "request");
    assert_required_link(
        "skills/cafleet/SKILL.md",
        "reference/supervision.md#recovery",
        "recover",
    );
    assert_required_link(
        "skills/cafleet/SKILL.md",
        "reference/supervision.md#shutdown",
        "immediately before",
    );
}

#[test]
fn shared_supervision_recovery_preserves_uncertainty_and_role_authority() {
    assert_section_terms(
        "skills/cafleet/reference/supervision.md",
        "recovery",
        &[
            "cafleet doctor",
            "cafleet member capture",
            "--lines",
            "unknown",
            "registry",
            "physical pane",
            "awaiting_user",
            "explicit",
            "question",
            "prompt-routing.md",
            "member delete",
            "member create",
            "new",
            "member_id",
        ],
    );
}

#[test]
fn shared_supervision_shutdown_checks_closure_within_authorized_scope() {
    let path = "skills/cafleet/reference/supervision.md";
    assert_section_terms(
        path,
        "shutdown",
        &[
            "monitor member",
            "first",
            "remaining member",
            "member list",
            "root Director",
            "fleet delete",
            "fleet list",
            "authorized",
            "scope",
        ],
    );
    let text = read(path);
    let section = section_at_anchor(path, &text, "shutdown");
    let mut remainder = section;
    for command in ["member delete", "member list", "fleet delete", "fleet list"] {
        let position = remainder
            .find(command)
            .unwrap_or_else(|| panic!("{path}#shutdown must place {command} in cleanup order"));
        remainder = &remainder[position + command.len()..];
    }
}

#[test]
fn shared_core_broadcast_owns_usage_and_runtime_delivery_references() {
    assert_section_terms(
        "skills/cafleet/SKILL.md",
        "broadcast",
        &[
            "cafleet message broadcast",
            "--from-member-id",
            "--file",
            "--json",
            "origin_message_id",
            "broadcast_summary",
            "recipient",
            "ack",
            "runtime/spec/cli-options.md#message-broadcast",
            "runtime/spec/message-envelope.md",
        ],
    );
}

#[test]
fn shared_member_startup_supports_shell_only_readers_before_operational_ready() {
    let path = "skills/cafleet/roles/member.md";
    let text = read(path);
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_terms_in(
        path,
        &compact,
        &[
            "role",
            "prerequisite",
            "non-shell",
            "only text reader",
            "first operational",
            "ready",
        ],
    );
    assert_section_terms(
        path,
        "command-execution",
        &[
            "Bash",
            "directly",
            "output",
            "denied",
            "Director",
            "prompt-routing.md",
        ],
    );
    assert_required_link(path, "../reference/base-dir.md", "BASE");
    assert_required_link(path, "../reference/prompt-routing.md", "before routing");
    assert_terms(
        "skills/cafleet/reference/prompt-routing.md",
        &[
            "../roles/member.md#command-execution",
            "../roles/director.md#member-prompt",
            "../roles/director.md#member-ping-manual-inbox-poll",
        ],
    );
}

#[test]
fn shared_base_contract_presents_member_write_states_before_the_resolver() {
    let path = "skills/cafleet/reference/base-dir.md";
    let text = read(path);
    let protocol = section_at_anchor(path, &text, "no-bypass-write-protocol");
    let procedure = section_at_anchor(path, &text, "procedure");
    let protocol_offset = text
        .find(protocol)
        .expect("protocol section belongs to this file");
    let procedure_offset = text
        .find(procedure)
        .expect("procedure section belongs to this file");
    assert!(
        protocol_offset < procedure_offset,
        "members read their input/write contract before Director resolution"
    );
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_terms_in(
        path,
        &compact,
        &[
            "Members",
            "literal",
            "BASE:",
            "re-resolv",
            "<unset>",
            "audit-disabled no BASE in spawn prompt",
            "${BASE} != <unset>",
            "Error: BASE is <unset>; refusing to fall back to /tmp",
            "consumer-supplied",
            ".prompts/",
            "dot-prefixed",
        ],
    );
}

#[test]
fn shared_send_failure_routes_ordinary_members_and_preserves_failed_relay_evidence() {
    assert_section_terms(
        "skills/cafleet/SKILL.md",
        "send-unicast",
        &[
            "persisted",
            "not resend",
            "ordinary member",
            "Director",
            "message poll",
            "relay",
            "failure",
            "id",
        ],
    );
    let path = "skills/cafleet/SKILL.md";
    let text = read(path);
    let section = section_at_anchor(path, &text, "send-unicast")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    assert!(
        regex::Regex::new(r"(?:no|not|never|without)[^.!\n]{0,120}(?:recursive|recurs|duplicate)")
            .unwrap()
            .is_match(&section),
        "secondary failure reporting must preserve evidence without recursive duplicate reports"
    );
}

#[test]
fn shared_requested_shell_dispatch_distinguishes_success_failure_and_plain_prompt() {
    let path = "skills/cafleet/reference/prompt-routing.md";
    assert_section_terms(
        path,
        "director-side-dispatch",
        &[
            "successful",
            "failure",
            "ping",
            "ack",
            "Serialize",
            "poll order",
        ],
    );
    let text = read(path);
    let section = section_at_anchor(path, &text, "director-side-dispatch");
    let mut remainder = section;
    for command in [
        "cafleet member prompt",
        "cafleet member ping",
        "cafleet message ack",
    ] {
        let position = remainder.find(command).unwrap_or_else(|| {
            panic!("{path} must preserve requested-shell success order: {command}")
        });
        remainder = &remainder[position + command.len()..];
    }
    let compact = section
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    assert!(
        regex::Regex::new(r"(?:fail\w*|non-zero shell dispatch)[^.]{0,160}(?:no ping|skip[^.]{0,60}ping|do not[^.]{0,60}ping)|(?:no ping|skip[^.]{0,60}ping)[^.]{0,160}fail")
            .unwrap().is_match(&compact),
        "failed shell dispatch skips the success ping"
    );
    assert_section_terms(
        path,
        "the-two-forms-shell-vs-plain",
        &["successful", "No ping follows the plain form"],
    );
}

#[test]
fn public_manual_lifecycle_contains_a_complete_installed_monitor_prompt() {
    let path = "docs/docs/how-to/mixed-backend-team.md";
    let text = read(path);
    assert_eq!(text.lines().next(), Some("# Run a fleet"));
    let section = section_at_anchor(path, &text, "manual-lifecycle");
    let prompt = regex::Regex::new(r"(?s)```text\n(.*?)\n```")
        .unwrap()
        .captures_iter(section)
        .find(|capture| capture[1].contains("ROLE DEFINITION:"))
        .expect("the manual lifecycle includes the complete monitor role prompt");
    assert_terms_in(
        path,
        &prompt[1],
        &[
            "ROLE DEFINITION: Open /",
            "/skills/cafleet/roles/monitor.md",
            "/skills/cafleet/reference/coding-agents.md",
            "/skills/cafleet/SKILL.md",
            "/skills/cafleet/reference/base-dir.md",
            "FLEET ID: {fleet_id}",
            "DIRECTOR MEMBER ID: {director_member_id}",
            "YOUR MEMBER ID: {member_id}",
            "CODING AGENT: {coding_agent}",
            "BASE: /",
            "monitor live",
        ],
    );
    assert_section_terms(
        path,
        "manual-lifecycle",
        &[
            "cafleet doctor",
            "config-dir-resolution",
            "claude",
            "codex",
            "opencode",
        ],
    );
}

#[test]
fn public_manual_lifecycle_gates_work_on_live_ready_and_fresh_capture() {
    assert_section_terms(
        "docs/docs/how-to/mixed-backend-team.md",
        "manual-lifecycle",
        &[
            "cafleet fleet create",
            "--monitor-file",
            "startup",
            "monitor live",
            "before",
            "ordinary member",
            "cafleet member create --fleet-id 1",
            "ready",
            "fresh capture",
            "cafleet message send --from-member-id 2 --to-member-id 4",
            "cafleet message poll 2",
            "cafleet message ack",
        ],
    );
}

#[test]
fn public_manual_lifecycle_verifies_monitor_first_shutdown() {
    let path = "docs/docs/how-to/mixed-backend-team.md";
    let text = read(path);
    let section = section_at_anchor(path, &text, "manual-lifecycle");
    let commands = [
        "cafleet member delete 3",
        "cafleet member delete 4",
        "cafleet member list 1",
        "cafleet fleet delete 1",
        "cafleet fleet list",
    ];
    let mut remainder = section;
    for command in commands {
        let position = remainder
            .find(command)
            .unwrap_or_else(|| panic!("{path} must include {command:?} in teardown order"));
        remainder = &remainder[position + command.len()..];
    }
    assert_terms_in(path, section, &["monitor", "first", "only", "Director"]);
}

#[test]
fn public_quickstart_keeps_install_configure_trust_and_routes_to_manual_lifecycle() {
    let path = "docs/docs/quickstart.md";
    assert_section_terms(
        path,
        "install",
        &["brew install himkt/tap/cafleet", "cafleet setup"],
    );
    assert_section_terms(
        path,
        "configure",
        &[
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "OPENCODE_CONFIG_DIR",
            ".config/opencode/skills",
        ],
    );
    assert_section_terms(
        path,
        "trust-the-working-directory",
        &["directory", "trust", "worktree"],
    );
    assert_terms(path, &["how-to/mixed-backend-team.md#manual-lifecycle"]);
}

#[test]
fn public_concept_navigation_orders_the_five_current_owners() {
    let path = "docs/docs/concepts/_meta.json";
    let entries: serde_json::Value = serde_json::from_str(&read(path)).unwrap();
    let actual: Vec<_> = entries
        .as_array()
        .expect("concept navigation is an array")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().expect("navigation entry has a name"),
                entry["label"]
                    .as_str()
                    .expect("navigation entry has a label"),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            ("overview", "Overview"),
            ("coding-agents", "Coding agents"),
            ("member-lifecycle", "Member lifecycle"),
            ("monitoring", "Monitoring"),
            ("storage", "Storage"),
        ]
    );
}

#[test]
fn public_overview_owns_fleet_routing_and_non_authentication() {
    assert_section_terms(
        "docs/docs/concepts/overview.md",
        "fleet-isolation",
        &["fleet_id", "routing", "authentication", "non-secret"],
    );
}

#[test]
fn public_coding_agents_owns_model_choice_and_manual_prompt_navigation() {
    let path = "docs/docs/concepts/coding-agents.md";
    assert_section_terms(
        path,
        "model-choice",
        &[
            "Director",
            "--coding-agent",
            "--model",
            "cost efficiency mode",
            "monitor",
            "reviewer",
        ],
    );
    assert_terms(path, &["mixed-backend-team.md#manual-lifecycle"]);
}

#[test]
fn public_spec_pages_keep_their_contract_owner_anchors() {
    for (path, anchors) in [
        (
            "docs/docs/spec/cli-options.md",
            &[
                "subcommand-summary",
                "environment-variables",
                "output-shapes",
                "error-messages",
                "creation-failure-compensation",
            ][..],
        ),
        (
            "docs/docs/spec/data-model.md",
            &["tables", "message-visibility-rules", "broadcast-grouping"][..],
        ),
        (
            "docs/docs/spec/message-envelope.md",
            &["persisted-shape", "rendered-shape"][..],
        ),
        (
            "docs/docs/spec/multiplexer-backends.md",
            &[
                "pane-creation-ownership",
                "push-notifications",
                "inline-preview-errors",
                "esc-safeguard",
            ][..],
        ),
        (
            "docs/docs/spec/coding-agent-backends.md",
            &["spawn-argv", "claude", "codex", "opencode"][..],
        ),
        (
            "docs/docs/spec/webui-api.md",
            &["request-headers", "endpoints", "error-format"][..],
        ),
    ] {
        let text = read(path);
        for anchor in anchors {
            assert!(!section_at_anchor(path, &text, anchor).is_empty());
        }
    }
    assert_terms(
        "docs/docs/spec/message-envelope.md",
        &["cli-options.md#output-shapes"],
    );
}

#[test]
fn public_monitoring_keeps_quiet_state_and_director_dispatch_conditions_inline() {
    assert_section_terms(
        "docs/docs/concepts/monitoring.md",
        "the-monitor-members-wake-protocol",
        &[
            "cafleet monitor scan",
            "content_sha256",
            "two consecutive wakes",
            "baseline",
            "re-arms",
            "at most once per quiet period",
            "Director",
            "unacked",
            "greater than 0",
            "unknown",
            "fresh capture",
            "working",
            "awaiting_user",
            "defer",
            "persistence",
            "captured prompt",
        ],
    );
}

#[test]
fn public_monitoring_cadence_keeps_forced_scheduled_and_restart_distinctions() {
    assert_section_terms(
        "docs/docs/concepts/monitoring.md",
        "cadence-and-tick-precision",
        &[
            "600s",
            "5s",
            "--interval 0",
            "heartbeating",
            "first wake",
            "last delivered wake",
            "one tick",
            "coalesce",
            "pending",
            "same write",
            "no ordinary members",
            "no timestamp",
            "loop restarts",
            "never been woken",
            "failed wake",
            "retries",
        ],
    );
}

#[test]
fn public_storage_keeps_the_four_step_compatible_binary_recovery() {
    let path = "docs/docs/concepts/storage.md";
    let text = read(path);
    let section = section_at_anchor(path, &text, "duplicate-monitor-recovery");
    let steps: Vec<_> = regex::Regex::new(r"(?m)^([1-4])\. ")
        .unwrap()
        .captures_iter(section)
        .map(|captures| captures[1].to_string())
        .collect();
    assert_eq!(steps, ["1", "2", "3", "4"]);
    assert_section_terms(
        path,
        "duplicate-monitor-recovery",
        &[
            "Stop new registrations",
            "retain",
            "preceding release",
            "old schema",
            "separate binary path",
            "CAFLEET_DATABASE_URL",
            "backend configuration",
            "assets",
            "independently",
            "restore",
            "compatible",
            "member delete <surplus-id>",
            "isolated invocation",
            "new binary",
            "setup",
            "downgrade",
        ],
    );
    let restore = section.find("restore").expect("restore compatible assets");
    let delete = section
        .find("member delete <surplus-id>")
        .expect("delete surplus monitor");
    assert!(
        restore < delete,
        "restore compatible assets before deleting surplus monitors"
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
const REMOVED_VOCABULARY: [&str; 16] = [
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
fn monitoring_concept_covers_the_monitor_member_and_capture_taxonomy() {
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
            "monitor member",
            "monitor live",
            "--role monitor",
            "Follow your monitor role protocol",
        ],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/docs/concepts/monitoring.md", &absent);
}

#[test]
fn monitoring_concept_explains_the_codex_managed_session_lifecycle() {
    let path = "docs/docs/concepts/monitoring.md";
    assert_terms(
        path,
        &[
            "backend-resolved long-lived execution",
            "monitor member",
            "session ID",
            "initial output",
            "one immediate poll",
            "failed start",
            "monitor live",
            "broker message",
            "before other work",
            "monitor restarted",
            "Director",
            "broker",
        ],
    );
    assert_absent(
        path,
        &[
            "runs as a background task",
            "plain backgrounded command",
            "works identically on any backend",
        ],
    );
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
fn spec_separates_blocking_runtime_from_backend_resolved_hosting() {
    assert_terms(
        "SPEC.md",
        &[
            "backend-resolved long-lived execution",
            "monitor member",
            "in-process (blocking)",
            "monitor loop started",
            "monitor live",
            "SIGTERM",
            "SIGINT",
        ],
    );
    assert_absent("SPEC.md", &["as a background task in its own pane"]);
}

#[test]
fn cli_and_webui_specs_keep_hosting_backend_neutral() {
    assert_terms(
        "docs/docs/spec/cli-options.md",
        &[
            "long-lived execution",
            "monitor member",
            "in-process",
            "blocks",
            "monitor loop started",
        ],
    );
    assert_absent("docs/docs/spec/cli-options.md", &["background task"]);

    assert_terms(
        "docs/docs/spec/webui-api.md",
        &[
            "CLI-only",
            "long-lived execution",
            "monitor member",
            "no `POST`/`DELETE` counterpart",
            "deleting the monitor member",
        ],
    );
    assert_absent("docs/docs/spec/webui-api.md", &["background task"]);
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
            "Director:",
            "Follow your monitor role protocol",
            "Resume your work if something was still running",
            "cafleet message poll <member-id> — then",
        ],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/docs/spec/multiplexer-backends.md", &absent);
}

#[test]
fn the_monitor_role_is_the_sole_normative_protocol_carrier() {
    assert_terms(
        "skills/cafleet/roles/monitor.md",
        &[
            "sole normative",
            "on-wake protocol",
            "cafleet monitor scan",
            "cafleet member ping",
            "cafleet message send",
            "monitor live",
            "two consecutive wakes",
            "content_sha256",
            "confirmed quiet",
            "unacked",
        ],
    );
    let mut absent = OLD_CLI_SURFACE.to_vec();
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/roles/monitor.md", &absent);
}

#[test]
fn every_backend_overlay_defines_the_capture_cues() {
    let text = read(OVERLAYS_FILE);
    for backend in ["claude", "codex", "opencode"] {
        assert_terms_in(
            &format!("{OVERLAYS_FILE} § {backend}"),
            overlay_section(&text, backend),
            &[
                "working",
                "stall_candidate",
                "quiet",
                "ambiguous",
                "on-wake classification",
                "pre-ping capture gate",
            ],
        );
    }
    assert_terms_in(
        &format!("{OVERLAYS_FILE} § Template"),
        overlay_section(&text, "Template"),
        &[
            "working",
            "stall_candidate",
            "Note → applies at",
            "on-wake classification",
        ],
    );
    assert_terms_in(
        &format!("{OVERLAYS_FILE} intro"),
        &text,
        &["monitor member"],
    );
    let mut absent = vec!["pre-nudge"];
    absent.extend(REMOVED_VOCABULARY);
    assert_absent(OVERLAYS_FILE, &absent);
}

#[test]
fn codex_overlay_defines_the_retained_managed_session_lifecycle() {
    let text = read(OVERLAYS_FILE);
    assert_terms_in(
        &format!("{OVERLAYS_FILE} § codex"),
        overlay_section(&text, "codex"),
        &[
            "managed execution session",
            "retained session ID",
            "without shell `&`",
            "initial output",
            "one immediate poll",
            "monitor loop started",
            "active but unconfirmed",
            "terminate",
            "monitor live",
            "broker message reopens",
            "before any other work",
            "interrupting or terminating",
        ],
    );
}

#[test]
fn codex_worked_launch_does_not_use_the_obsolete_shell_ampersand() {
    let text = read(OVERLAYS_FILE);
    let codex = overlay_section(&text, "codex");
    assert!(
        codex.contains("cafleet monitor <fleet-id>"),
        "{OVERLAYS_FILE} § codex must show the monitor launch command"
    );
    assert!(
        !codex.contains("cafleet monitor <fleet-id> &"),
        "{OVERLAYS_FILE} § codex still uses the obsolete shell-backgrounded command"
    );
}

#[test]
fn only_opencode_retains_the_shell_ampersand_worked_command() {
    let text = read(OVERLAYS_FILE);
    let command = "cafleet monitor <fleet-id> &";
    assert_eq!(
        text.matches(command).count(),
        1,
        "{OVERLAYS_FILE} must contain exactly one shell-ampersand worked command"
    );
    assert!(
        overlay_section(&text, "opencode").contains(command),
        "{OVERLAYS_FILE} § opencode must own the sole shell-ampersand worked command"
    );
}

#[test]
fn claude_and_opencode_overlays_preserve_their_launch_and_stop_contracts() {
    let text = read(OVERLAYS_FILE);
    assert_terms_in(
        &format!("{OVERLAYS_FILE} § claude"),
        overlay_section(&text, "claude"),
        &[
            "run_in_background: true",
            "TaskStop",
            "cafleet monitor <fleet-id>",
        ],
    );
    assert_terms_in(
        &format!("{OVERLAYS_FILE} § opencode"),
        overlay_section(&text, "opencode"),
        &[
            "backgrounded `!` shell command",
            "killing the recorded background process",
            "cafleet monitor <fleet-id> &",
        ],
    );
}

#[test]
fn monitor_role_defines_the_backend_resolved_loop_lifecycle() {
    let path = "skills/cafleet/roles/monitor.md";
    assert_terms(
        path,
        &[
            "long-lived execution",
            "{bg_run}",
            "{bg_stop}",
            "initial output",
            "one immediate poll",
            "failed start",
            "terminate",
            "monitor loop started",
            "monitor live",
            "exit notification",
            "broker message",
            "before other work",
            "monitor restarted",
            "session ID",
            "monitor member",
            "Director",
        ],
    );
    assert_absent(
        path,
        &[
            "as a background task",
            "loop's background task",
            "loop task itself",
        ],
    );
}

#[test]
fn shared_skill_pages_make_the_monitor_member_the_execution_owner() {
    assert_terms(
        "skills/cafleet/reference/supervision.md",
        &[
            "backend-resolved long-lived execution",
            "monitor member",
            "execution handle",
            "liveness checks",
            "Director",
            "broker",
        ],
    );
    assert_terms(
        "skills/cafleet/SKILL.md",
        &[
            "monitor member",
            "own pane",
            "monitor live",
            "roles/monitor.md",
        ],
    );
    assert_terms(
        "skills/cafleet/roles/monitor.md",
        &["long-lived execution", "monitor member", "backend"],
    );
    assert_terms(
        "skills/cafleet/roles/director.md",
        &["monitor member", "wake source", "first"],
    );
    assert_absent(
        "skills/cafleet/reference/supervision.md",
        &["just a backgrounded command"],
    );
    assert_absent(
        "skills/cafleet/SKILL.md",
        &["as a background task in its own pane"],
    );
    assert_absent(
        "skills/cafleet/roles/director.md",
        &["loop's background task"],
    );
}

#[test]
fn skill_author_guidance_keeps_the_heartbeat_backend_neutral() {
    let path = ".claude/skills/skill-author/SKILL.md";
    assert_section_terms(
        path,
        "phase-prerequisites",
        &[
            "runtime bindings",
            "base-dir.md",
            "roles/director.md#canonical-spawn-prompt-skeleton",
            "roles/director.md#member-create--scratch-and-audit-files",
            "supervision.md#spawn-protocol",
            "coordination.md",
            "supervision.md#recovery",
            "supervision.md#shutdown",
            "monitor live",
        ],
    );
    assert_section_terms(
        path,
        "resolve-and-bootstrap",
        &[
            "cafleet doctor",
            "--monitor-file",
            "--monitor-model",
            "monitor owns launching and confirming its loop",
            "only after that gate",
        ],
    );
    assert_absent(
        path,
        &[
            "as a background task in its own pane",
            "both are identical on any backend",
        ],
    );
}

#[test]
fn the_supervision_contract_covers_the_monitor_member_and_quiet_members() {
    assert_terms(
        "skills/cafleet/reference/supervision.md",
        &[
            "member ping",
            "quiet",
            "finished",
            "pre-ping",
            "member capture",
            "cafleet monitor",
            "health-check",
            "monitor member",
            "monitor live",
            "--role monitor",
        ],
    );
    let mut absent = vec!["pre-nudge"];
    absent.extend(OLD_CLI_SURFACE);
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/reference/supervision.md", &absent);
}

#[test]
fn the_bootstrap_docs_carry_the_monitor_file_contract() {
    assert_terms(
        "docs/docs/spec/cli-options.md",
        &["--monitor-file", "--monitor-model"],
    );
    assert_terms("SPEC.md", &["--monitor-file", "--monitor-model"]);
    assert_terms(
        "skills/cafleet/reference/supervision.md",
        &["--monitor-file"],
    );
    assert_terms(
        "skills/cafleet/roles/monitor.md",
        &["fleet create", "--monitor-file", "--role monitor"],
    );
}

#[test]
fn the_director_and_member_roles_keep_the_ping_protocol() {
    assert_terms(
        "skills/cafleet/roles/director.md",
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
    assert_absent("skills/cafleet/roles/director.md", &absent);

    assert_terms(
        "skills/cafleet/roles/member.md",
        &["member ping", "member prompt", "Director", "monitor member"],
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
            "monitor member",
            "monitor live",
            "--role monitor",
            "monitor_model",
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
        "skills/cafleet/reference/prompt-routing.md",
        "skills/cafleet/roles/director.md",
        "docs/docs/concepts/overview.md",
    ] {
        assert_absent(relative_path, &["nudge"]);
    }
    let path = "skills/cafleet/reference/supervision.md";
    let text = read(path);
    assert!(
        !section_at_anchor(path, &text, "recovery")
            .to_lowercase()
            .contains("nudge"),
        "fixed-ping recovery uses the current command vocabulary"
    );
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

#[test]
fn every_role_file_gates_its_overlay_as_required_reading_row_one() {
    let heading = regex::Regex::new(r"(?m)^#+[ \t]+Required[ -]reading").unwrap();
    let reference = regex::Regex::new(r"\]\(([^)#]*coding-agents\.md)(?:#([^)]*))?\)").unwrap();
    let expected_path = root()
        .join(OVERLAYS_FILE)
        .canonicalize()
        .expect("the unified coding-agent reference exists");
    let mut files = skill_markdown_files();
    collect_markdown(&root().join(".claude/skills/clean-docs"), &mut files);
    let mut offenders = Vec::new();
    for relative_path in files {
        let text = read(&relative_path);
        // A role file must *have* the block: folding content out of one must
        // never carry the gated overlay read away with it. Other skill pages
        // are checked for content only, since not all of them gate reads.
        let block_is_mandatory = relative_path.contains("/roles/");
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
            Some(row) => {
                let Some(link) = reference.captures(row) else {
                    offenders.push(format!(
                        "{relative_path} → row #1 must link to the unified coding-agent reference"
                    ));
                    continue;
                };
                let parent = root().join(&relative_path);
                let target = parent
                    .parent()
                    .expect("a skill file has a parent directory")
                    .join(&link[1]);
                match target.canonicalize() {
                    Ok(path) if path == expected_path => {}
                    _ => offenders.push(format!(
                        "{relative_path} → row #1 link must resolve to {OVERLAYS_FILE}"
                    )),
                }
                if let Some(anchor) = link.get(2) {
                    let names: Vec<_> = match anchor.as_str() {
                        "<name>" => vec!["claude", "codex", "opencode"],
                        name => vec![name],
                    };
                    let reference_text = read(OVERLAYS_FILE);
                    for name in names {
                        if !reference_text
                            .lines()
                            .any(|line| line == format!("## {name}"))
                        {
                            offenders.push(format!(
                                "{relative_path} → row #1 backend anchor #{name} must exist"
                            ));
                        }
                    }
                }
            }
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
    "monitor_model",
    "permission_flags",
    "bg_run",
    "bg_stop",
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
const NON_OVERLAY_TOKENS: [&str; 4] = [
    // Meta-references: prose describing the resolution rule itself.
    "token",
    "placeholder",
    // Workflow-local path variables.
    "slug",
    "dir_path",
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
    let text = read(OVERLAYS_FILE);
    for backend in ["claude", "codex", "opencode"] {
        let backend_rows = table_rows(overlay_section(&text, backend));
        for placeholder in OVERLAY_PLACEHOLDERS {
            let token = format!("`{{{placeholder}}}`");
            assert_eq!(
                backend_rows.iter().filter(|row| row[0] == token).count(),
                1,
                "{backend} requires one authoritative assignment for {token}"
            );
        }
        for subsection in ["Runtime bindings", "Role defaults"] {
            let rows = table_rows(backend_subsection(&text, backend, subsection));
            assert!(rows.len() >= 2, "{backend} {subsection} requires a table");
            assert_eq!(rows[0], ["Placeholder", "Value"], "{backend} {subsection}");
            let assignments = &rows[2..];
            assert!(
                assignments
                    .iter()
                    .all(|row| row.len() == 2 && !row[1].is_empty()),
                "{backend} {subsection} requires a value for every placeholder"
            );
            let mut actual: Vec<_> = assignments.iter().map(|row| row[0]).collect();
            actual.sort_unstable();
            let mut expected: Vec<_> = OVERLAY_PLACEHOLDERS
                .iter()
                .filter(|name| name.ends_with("_model") == (subsection == "Role defaults"))
                .map(|name| format!("`{{{name}}}`"))
                .collect();
            expected.sort_unstable();
            assert_eq!(actual, expected, "{backend} {subsection} assignments");
        }
    }
    for subsection in ["Runtime bindings", "Role defaults"] {
        let template = backend_subsection(&text, "Template", subsection);
        for placeholder in OVERLAY_PLACEHOLDERS
            .iter()
            .filter(|name| name.ends_with("_model") == (subsection == "Role defaults"))
        {
            let token = format!("`{{{placeholder}}}`");
            assert_eq!(
                template.matches(&token).count(),
                1,
                "Template {subsection} must prescribe {token} once"
            );
        }
    }
    assert_terms_in(
        "Template Role defaults",
        backend_subsection(&text, "Template", "Role defaults"),
        &["most capable", "lightweight", "must belong to the catalog"],
    );
}

#[test]
fn coding_agent_sections_expose_the_complete_backend_lookup_contract() {
    let text = read(OVERLAYS_FILE);
    let backends = ["claude", "codex", "opencode", "Template"];
    let actual: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .collect();
    assert_eq!(
        actual, backends,
        "backend anchors and order must remain stable"
    );
    for backend in backends {
        let actual: Vec<_> = overlay_section(&text, backend)
            .lines()
            .filter_map(|line| line.strip_prefix("### "))
            .collect();
        assert_eq!(
            actual,
            [
                "Runtime bindings",
                "Role defaults",
                "Model catalog",
                "Note → applies at",
                "Pane-state capture cues",
                "Worked resolution",
            ],
            "{backend} must provide each lookup in the documented order"
        );
    }
}

#[test]
fn backend_role_defaults_resolve_to_local_catalog_models_or_aliases() {
    let text = read(OVERLAYS_FILE);
    for backend in ["claude", "codex", "opencode"] {
        let catalog = table_rows(backend_subsection(&text, backend, "Model catalog"));
        assert!(
            catalog.len() > 2,
            "{backend} requires a populated model catalog"
        );
        let token_columns: Vec<_> = catalog[0]
            .iter()
            .enumerate()
            .filter_map(|(index, name)| matches!(*name, "Model" | "Alias").then_some(index))
            .collect();
        assert_eq!(
            catalog[0][0], "Model",
            "{backend} catalog starts with the spawn token"
        );
        for row in &catalog[2..] {
            assert_eq!(
                row.len(),
                catalog[0].len(),
                "{backend} catalog row: {row:?}"
            );
        }
        for row in table_rows(backend_subsection(&text, backend, "Role defaults"))
            .iter()
            .skip(2)
        {
            assert_eq!(row.len(), 2, "{backend} role assignment: {row:?}");
            let model = row[1].trim_matches('`');
            assert!(
                !model.is_empty()
                    && model != "—"
                    && catalog[2..].iter().any(|entry| {
                        token_columns
                            .iter()
                            .any(|&column| entry[column].trim_matches('`') == model)
                    }),
                "{backend} {} must resolve to its own catalog: {model}",
                row[0]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Issue #338 contracts: one-shot command isolation, the Send partial-failure
// recovery, and the asynchronous-wait turn boundary.
// ---------------------------------------------------------------------------

#[test]
fn the_core_skill_owns_the_one_shot_isolation_rule() {
    assert_terms(
        "skills/cafleet/SKILL.md",
        &[
            "one-shot",
            "only command in its shell-tool invocation",
            "separate shell-tool",
            "newline",
            "&&",
            "pipe",
            "shell `&`",
            "NAME=value",
            "immediately precede",
            "helper process",
            "redirection",
            "sole exception",
            "cafleet monitor",
            "overlay",
            "Asynchronous Wait Rule",
        ],
    );
}

#[test]
fn the_isolation_section_carries_the_permission_error_diagnostic() {
    assert_terms(
        "skills/cafleet/SKILL.md",
        &[
            "Operation not permitted",
            "Permission denied",
            "auto-approval scope",
            "allow rules",
            "compound invocation",
            "re-run",
            "isolated invocation",
            "no-resend",
        ],
    );
}

#[test]
fn the_send_section_pins_the_persisted_no_resend_recovery() {
    let path = "skills/cafleet/SKILL.md";
    let text = read(path);
    assert_terms_in(
        &format!("{path} § Send (Unicast)"),
        markdown_section(path, &text, "Send (Unicast)"),
        &[
            "was persisted",
            "committed",
            "not resend",
            "member ping",
            "message poll",
            "ack",
            "isolated",
            "retry",
        ],
    );
}

#[test]
fn supervision_pins_the_asynchronous_turn_boundary() {
    assert_terms(
        "skills/cafleet/reference/supervision.md",
        &[
            "Asynchronous Wait Rule",
            "asynchronous handoff is a turn boundary",
            "end or yield",
            "MUST NOT",
            "busy-wait",
            "recurring polling",
            "resumes the workflow in a later turn",
            "one-off status check",
            "timer",
        ],
    );
}

#[test]
fn design_doc_workflows_resume_on_notifications_not_recurring_polls() {
    assert_terms(
        "skills/cafleet-design-doc/create/create.md",
        &["turn boundary"],
    );
    assert_terms(
        "skills/cafleet-design-doc/interview/interview.md",
        &["turn boundary"],
    );
    assert_terms(
        "skills/cafleet-design-doc/execute/execute.md",
        &["later turn"],
    );

    // Directive phrases only: the normative prohibition text (which names
    // sleep / periodic polling as forbidden objects) lives in the core
    // supervision reference, which this sweep deliberately excludes.
    let patterns = [
        ("periodic-poll directive", r"(?i)periodic[^\n]{0,60}poll"),
        ("poll-until directive", r"(?i)\bpoll\b[^\n]{0,80}\buntil\b"),
        ("wait-again-with-poll directive", r"(?i)wait again with"),
        ("wait-via-poll directive", r"(?i)via `cafleet message poll"),
        ("director-side sleep guidance", r"(?i)\bsleep\b"),
        ("busy-wait guidance", r"(?i)busy[- ]wait"),
        ("scheduler guidance", r"(?i)\bscheduler\b"),
        ("timer-loop guidance", r"(?i)\btimer\b"),
    ];
    let mut files = Vec::new();
    collect_markdown(&root().join("skills/cafleet-design-doc"), &mut files);
    files.sort();
    let mut offenders = Vec::new();
    for relative_path in &files {
        let text = read(relative_path);
        for (label, pattern) in &patterns {
            let regex = regex::Regex::new(pattern).unwrap();
            for (index, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    offenders.push(format!("{relative_path}:{} → {label}", index + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "dependent workflows must dispatch, end or yield, and resume on a \
         notification in a later turn — not wait through recurring polls: {offenders:#?}"
    );
}
