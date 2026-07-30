//! Docs-sync contracts: the repository documentation is the public contract
//! for the monitor ping protocol — the pure-trigger wake, the monitoring
//! member's two-wake in-context judgment, the `member ping`
//! pending-placement skip, and the `monitor` group surface.

use std::path::PathBuf;

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

// Episode-machine vocabulary that must not survive on any contract page.
const REMOVED_VOCABULARY: [&str; 10] = [
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
];

#[test]
fn monitoring_concept_covers_the_judgment_protocol_and_pure_trigger_wake() {
    assert_terms(
        "docs/concepts/monitoring.md",
        &[
            "awaiting_user",
            "unknown",
            "finished",
            "working",
            "stall_candidate",
            "quiet",
            "byte-identical",
            "stall-check",
            "cafleet member ping",
            "cafleet message send",
            "monitor loop started",
            "wake trigger",
            "pointer sentence",
            "role protocol",
        ],
    );
    assert_absent("docs/concepts/monitoring.md", &REMOVED_VOCABULARY);
}

#[test]
fn spec_defines_the_ping_skip_and_monitor_group_contract() {
    assert_terms(
        "SPEC.md",
        &[
            "skipped",
            "pending placement",
            "monitor capture",
            "monitor loop started",
            "Follow your monitor role protocol",
        ],
    );
    assert_absent("SPEC.md", &REMOVED_VOCABULARY);
}

#[test]
fn data_model_defines_the_trimmed_monitor_config() {
    assert_terms(
        "docs/spec/data-model.md",
        &[
            "monitor_config",
            "interval_seconds",
            "last_ping_at",
            "enabled",
            "last_stall_check_at",
        ],
    );
    let mut absent = vec!["last_stall_candidate_at", "last_stall_capture_sha256"];
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/spec/data-model.md", &absent);
}

#[test]
fn cli_options_defines_the_ping_skip_and_moved_capture() {
    assert_terms(
        "docs/spec/cli-options.md",
        &[
            "monitor capture",
            "ping skipped",
            "skipped",
            "pending placement",
            "nothing to",
        ],
    );
    let mut absent = vec!["member capture", "monitor status", "monitor config"];
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("docs/spec/cli-options.md", &absent);
}

#[test]
fn multiplexer_backends_pins_the_pure_trigger_payload() {
    assert_terms(
        "docs/spec/multiplexer-backends.md",
        &[
            "[monitor] wake:",
            "coding_agent=",
            "Follow your monitor role protocol",
        ],
    );
    assert_absent("docs/spec/multiplexer-backends.md", &REMOVED_VOCABULARY);
}

#[test]
fn the_monitor_role_is_the_sole_normative_protocol_carrier() {
    assert_terms(
        "skills/cafleet/roles/monitor.md",
        &[
            "--lines 120",
            "--no-ansi",
            "--json",
            "content_sha256",
            "stall_candidate",
            "finished",
            "quiet",
            "byte-identical",
            "stall-check",
            "cafleet member ping",
            "cafleet message send",
            "ready: monitor live",
            "monitor loop started",
        ],
    );
    assert_absent("skills/cafleet/roles/monitor.md", &REMOVED_VOCABULARY);
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
        &[
            "working",
            "stall_candidate",
            "monitoring member",
            "Note → applies at",
        ],
    );
}

#[test]
fn the_supervision_contract_covers_quiet_members_and_plain_messages() {
    assert_terms(
        "skills/cafleet/reference/supervision.md",
        &[
            "monitoring member",
            "cafleet member ping",
            "quiet",
            "finished",
            "pre-ping",
            "monitor capture",
        ],
    );
    let mut absent = vec!["pre-nudge"];
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
            "monitoring member",
            "pre-ping",
            "monitor capture",
        ],
    );
    let mut absent = vec!["(manual inbox-poll nudge)", "pre-nudge"];
    absent.extend(REMOVED_VOCABULARY);
    assert_absent("skills/cafleet/reference/director.md", &absent);

    assert_terms(
        "skills/cafleet/roles/member.md",
        &[
            "monitoring member",
            "exception",
            "member ping",
            "member prompt",
            "ordinary",
        ],
    );
}

#[test]
fn the_cafleet_skill_and_bash_rule_document_the_fixed_ping_exception() {
    assert_terms(
        "skills/cafleet/SKILL.md",
        &[
            "monitoring member",
            "fixed",
            "member ping",
            "exception",
            "quiet",
            "message send",
        ],
    );
    assert_absent("skills/cafleet/SKILL.md", &REMOVED_VOCABULARY);

    assert_terms(
        ".claude/rules/bash-tool.md",
        &[
            "monitoring member",
            "fixed",
            "member ping",
            "quiet period",
            "byte-identical",
            "Esc",
            "message poll",
        ],
    );
    let mut absent = vec!["action = ping"];
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
        "skills/cafleet/roles/monitor.md",
        "docs/concepts/overview.md",
    ] {
        assert_absent(relative_path, &["nudge"]);
    }
}

#[test]
fn the_readme_and_webui_api_stay_free_of_internal_monitor_state() {
    let mut readme_absent = vec!["stall_candidate"];
    readme_absent.extend(REMOVED_VOCABULARY);
    assert_absent("README.md", &readme_absent);
    assert_absent("docs/spec/webui-api.md", &REMOVED_VOCABULARY);
}
