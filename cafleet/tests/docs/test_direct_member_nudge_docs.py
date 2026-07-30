"""Documentation contracts for the simplified monitor ping protocol.

These tests exercise the repository documentation as the public contract:
the pure-trigger wake, the monitoring member's two-wake in-context judgment,
the ``member ping`` pending-placement skip, plain per-event Director messages,
and the ``monitor`` group = ``start`` + ``capture`` surface.
"""

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]


def _read(relative_path: str) -> str:
    return (ROOT / relative_path).read_text(encoding="utf-8")


def _assert_terms(relative_path: str, *terms: str) -> None:
    text = _read(relative_path).lower()
    normalized_text = re.sub(r"[-‐‑–—]+", " ", text)
    missing = [
        term
        for term in terms
        if term.lower() not in text
        and re.sub(r"[-‐‑–—]+", " ", term.lower()) not in normalized_text
    ]
    assert not missing, f"{relative_path} is missing required terms: {missing}"


def _assert_absent(relative_path: str, *terms: str) -> None:
    text = _read(relative_path).lower()
    present = [term for term in terms if term.lower() in text]
    assert not present, f"{relative_path} still mentions removed terms: {present}"


# Episode-machine vocabulary that must not survive on any contract page.
_REMOVED_VOCABULARY = (
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
)


def test_monitoring_concept_covers_judgment_protocol():
    _assert_terms(
        "docs/concepts/monitoring.md",
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
    )
    _assert_absent("docs/concepts/monitoring.md", *_REMOVED_VOCABULARY)


def test_monitoring_concept_documents_pure_trigger_wake():
    _assert_terms(
        "docs/concepts/monitoring.md",
        "wake trigger",
        "pointer sentence",
        "role protocol",
    )


def test_spec_defines_ping_skip_and_monitor_group_contract():
    _assert_terms(
        "SPEC.md",
        "skipped",
        "pending placement",
        "monitor capture",
        "monitor loop started",
        "Follow your monitor role protocol",
        "0006",
    )
    _assert_absent("SPEC.md", *_REMOVED_VOCABULARY)


def test_data_model_defines_trimmed_monitor_config():
    _assert_terms(
        "docs/spec/data-model.md",
        "monitor_config",
        "interval_seconds",
        "last_ping_at",
        "enabled",
        "last_stall_check_at",
        "0006",
    )
    _assert_absent(
        "docs/spec/data-model.md",
        "last_stall_candidate_at",
        "last_stall_capture_sha256",
        *_REMOVED_VOCABULARY,
    )


def test_cli_options_defines_ping_skip_and_moved_capture():
    _assert_terms(
        "docs/spec/cli-options.md",
        "monitor capture",
        "ping skipped",
        "skipped",
        "pending placement",
        "nothing to",
    )
    _assert_absent(
        "docs/spec/cli-options.md",
        "member capture",
        "monitor status",
        "monitor config",
        *_REMOVED_VOCABULARY,
    )


def test_multiplexer_backends_pins_pure_trigger_payload():
    _assert_terms(
        "docs/spec/multiplexer-backends.md",
        "[monitor] wake:",
        "coding_agent=",
        "Follow your monitor role protocol",
    )
    _assert_absent("docs/spec/multiplexer-backends.md", *_REMOVED_VOCABULARY)


def test_monitor_role_is_sole_normative_protocol_carrier():
    _assert_terms(
        "skills/cafleet/roles/monitor.md",
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
    )
    _assert_absent("skills/cafleet/roles/monitor.md", *_REMOVED_VOCABULARY)


@pytest.mark.parametrize("backend", ["claude", "codex", "opencode"])
def test_backend_overlay_defines_working_and_stall_candidate_cues(backend: str):
    overlay = f"skills/cafleet/reference/coding-agent/{backend}-overlay.md"
    _assert_terms(
        overlay,
        "working",
        "stall_candidate",
        "quiet",
        "ambiguous",
        "pre-ping capture gate",
    )
    _assert_absent(overlay, "pre-nudge", *_REMOVED_VOCABULARY)


def test_overlay_template_binds_cues_to_monitor():
    _assert_terms(
        "skills/cafleet/reference/coding-agent/_template.md",
        "working",
        "stall_candidate",
        "monitoring member",
        "Note → applies at",
    )
    _assert_absent(
        "skills/cafleet/reference/coding-agent/_template.md",
        "pre-nudge",
        *_REMOVED_VOCABULARY,
    )


def test_supervision_contract_covers_quiet_members_and_plain_messages():
    _assert_terms(
        "skills/cafleet/reference/supervision.md",
        "monitoring member",
        "cafleet member ping",
        "quiet",
        "finished",
        "pre-ping",
        "monitor capture",
    )
    _assert_absent(
        "skills/cafleet/reference/supervision.md",
        "pre-nudge",
        *_REMOVED_VOCABULARY,
    )


def test_director_role_renames_ping_heading_and_drops_aggregates():
    _assert_terms(
        "skills/cafleet/reference/director.md",
        "## Member Ping (manual inbox-poll)",
        "member ping",
        "monitoring member",
        "pre-ping",
        "monitor capture",
    )
    _assert_absent(
        "skills/cafleet/reference/director.md",
        "(manual inbox-poll nudge)",
        "pre-nudge",
        *_REMOVED_VOCABULARY,
    )


def test_ordinary_member_role_preserves_ping_and_prompt_prohibition():
    _assert_terms(
        "skills/cafleet/roles/member.md",
        "monitoring member",
        "exception",
        "member ping",
        "member prompt",
        "ordinary",
    )


def test_cafleet_skill_documents_narrow_monitor_ping_exception():
    _assert_terms(
        "skills/cafleet/SKILL.md",
        "monitoring member",
        "fixed",
        "member ping",
        "exception",
        "quiet",
        "message send",
    )
    _assert_absent("skills/cafleet/SKILL.md", *_REMOVED_VOCABULARY)


def test_bash_rule_documents_preapproved_fixed_monitor_ping_only():
    _assert_terms(
        ".claude/rules/bash-tool.md",
        "monitoring member",
        "fixed",
        "member ping",
        "quiet period",
        "byte-identical",
        "Esc",
        "message poll",
    )
    _assert_absent(".claude/rules/bash-tool.md", "action = ping", *_REMOVED_VOCABULARY)


@pytest.mark.parametrize(
    "relative_path",
    [
        "skills/cafleet/reference/recovery.md",
        "skills/cafleet/reference/prompt-routing.md",
        "skills/cafleet/reference/director.md",
        "skills/cafleet/roles/monitor.md",
        "docs/concepts/overview.md",
    ],
)
def test_fixed_ping_surfaces_carry_no_nudge_vocabulary(relative_path: str):
    """The fixed-ping / wake-trigger senses of "nudge" are respelled; the
    Director's message-level stall-nudge concept lives only in the
    cafleet-design-doc skill's coordination protocol."""
    _assert_absent(relative_path, "nudge")


def test_readme_remains_free_of_deep_monitor_protocol_details():
    _assert_absent("README.md", "stall_candidate", *_REMOVED_VOCABULARY)


@pytest.mark.parametrize(
    "relative_path",
    [
        "docs/api/broker.md",
        "docs/api/multiplexer.md",
        "docs/spec/webui-api.md",
    ],
)
def test_unrelated_api_and_webui_contracts_do_not_gain_internal_state(
    relative_path: str,
):
    _assert_absent(relative_path, *_REMOVED_VOCABULARY)
