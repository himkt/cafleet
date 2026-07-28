"""Documentation contracts for direct fixed-action monitor nudges.

These tests intentionally exercise the repository documentation as the public
contract.  Step 1 of design document 0000151 must land before the runtime
implementation, so the suite should fail until those documents and role assets
describe the new protocol.
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


def test_monitoring_concept_covers_direct_nudge_safety_and_delivery_contract():
    _assert_terms(
        "docs/concepts/monitoring.md",
        "synchronized",
        "annotation-only",
        "stall_candidate",
        "working",
        "last_stall_check_at",
        "escalation_pending",
        "cafleet member ping",
        "monitor_director_gate",
        "message show",
        "finished",
        "Director",
    )


def test_spec_defines_durable_episode_and_delivery_schema():
    _assert_terms(
        "SPEC.md",
        "0005_add_monitor_stall_episode_state.py",
        "last_stall_check_at",
        "last_stall_candidate_at",
        "last_stall_capture_sha256",
        "stall_episode_state",
        "stall_escalation_reason",
        "monitor_report_delivery",
        "monitor_director_gate",
        "one open",
        "30 second",
    )


def test_spec_defines_capture_and_monitor_cli_contracts():
    _assert_terms(
        "SPEC.md",
        "captured_at",
        "content_sha256",
        "monitor stall observe",
        "monitor stall ping-result",
        "monitor stall pending",
        "monitor report-batch",
        "--director-gate-token",
        "--finished-member-id",
        "awaiting_ack",
        "preview_outcome",
        "message show",
    )


def test_spec_defines_atomic_episode_and_report_delivery_semantics():
    _assert_terms(
        "SPEC.md",
        "nudge_claimed",
        "ping_failed",
        "ping_interrupted",
        "unchanged_after_nudge",
        "single-use",
        "backpressure",
        "ack",
        "same message id",
        "escalation_pending",
    )


def test_spec_defines_annotation_only_synchronized_monitor_loop():
    _assert_terms(
        "SPEC.md",
        "annotation-only",
        "last_stall_check_at",
        "coding_agent",
        "status:done",
        "stall-check",
        "unacked",
        "append",
        "one synchronized wake",
    )


def test_spec_removes_process_local_stall_and_unacked_maps():
    spec = _read("SPEC.md")
    assert "_last_stall_check_at" not in spec
    assert "_last_unacked_wake_at" not in spec


def test_spec_defines_token_gated_byte_identical_wake_contract():
    _assert_terms(
        "SPEC.md",
        "byte-identical",
        "coding_agent=",
        "stall_candidate",
        "working",
        "monitor stall pending",
        "cafleet member ping",
        "monitor report-batch",
        "director-gate",
        "no intervening",
        "only the Director",
    )


@pytest.mark.parametrize(
    ("relative_path", "required_terms"),
    [
        pytest.param(
            "docs/spec/data-model.md",
            (
                "last_stall_check_at",
                "last_stall_candidate_at",
                "last_stall_capture_sha256",
                "stall_episode_state",
                "stall_escalation_reason",
                "monitor_report_delivery",
                "monitor_director_gate",
            ),
            id="data-model",
        ),
        pytest.param(
            "docs/spec/cli-options.md",
            (
                "captured_at",
                "content_sha256",
                "monitor stall observe",
                "monitor stall ping-result",
                "monitor stall pending",
                "monitor report-batch",
                "--director-gate-token",
                "message show",
            ),
            id="cli-options",
        ),
        pytest.param(
            "docs/spec/multiplexer-backends.md",
            (
                "annotation",
                "coding_agent",
                "cafleet member ping",
                "aggregate",
                "same message id",
            ),
            id="multiplexer-backends",
        ),
    ],
)
def test_focused_reference_pages_cover_their_monitor_contracts(
    relative_path: str, required_terms: tuple[str, ...]
):
    _assert_terms(relative_path, *required_terms)


def test_monitor_role_defines_target_specific_durable_action_order():
    _assert_terms(
        "skills/cafleet/roles/monitor.md",
        "coding_agent=",
        "--lines 120",
        "--no-ansi",
        "stall_candidate",
        "working",
        "monitor stall pending",
        "monitor stall observe",
        "cafleet member ping",
        "monitor stall ping-result",
        "--director-gate",
        "monitor report-batch",
        "no intervening",
        "sole Director-delivery path",
        "message show --full",
    )


@pytest.mark.parametrize("backend", ["claude", "codex", "opencode"])
def test_backend_overlay_defines_working_and_stall_candidate_cues(backend: str):
    _assert_terms(
        f"skills/cafleet/reference/coding-agent/{backend}-overlay.md",
        "working",
        "stall_candidate",
        "affirmative",
        "quiet",
        "ambiguous",
    )


def test_overlay_template_binds_working_and_candidate_cues_to_monitor():
    _assert_terms(
        "skills/cafleet/reference/coding-agent/_template.md",
        "working",
        "stall_candidate",
        "monitoring member",
        "Note → applies at",
        "target",
    )


def test_supervision_contract_assigns_first_fixed_ping_to_monitor():
    _assert_terms(
        "skills/cafleet/reference/supervision.md",
        "monitoring member",
        "first confident",
        "cafleet member ping",
        "escalation_pending",
        "message show --full",
        "target-specific",
        "fresh capture",
        "Director",
    )


def test_director_role_requires_full_aggregate_consumption_and_dedup():
    _assert_terms(
        "skills/cafleet/roles/director.md",
        "monitor report batch",
        "message show",
        "--full",
        "message id",
        "ack",
        "monitoring member",
        "member ping",
        "exception",
        "finished",
    )


def test_ordinary_member_role_preserves_ping_and_prompt_prohibition():
    _assert_terms(
        "skills/cafleet/roles/member.md",
        "monitoring member",
        "exception",
        "member ping",
        "member prompt",
        "ordinary",
        "must not",
    )


def test_cafleet_skill_documents_narrow_monitor_ping_exception():
    _assert_terms(
        "skills/cafleet/SKILL.md",
        "monitoring member",
        "fixed",
        "member ping",
        "exception",
        "arbitrary",
        "finished",
        "Director",
    )


def test_bash_rule_documents_preapproved_fixed_monitor_ping_only():
    _assert_terms(
        ".claude/rules/bash-tool.md",
        "monitoring member",
        "fixed",
        "member ping",
        "exception",
        "arbitrary",
        "Esc",
        "message poll",
    )


def test_readme_remains_free_of_deep_monitor_protocol_details():
    readme = _read("README.md")
    assert "monitor report-batch" not in readme
    assert "monitor_director_gate" not in readme
    assert "stall_candidate" not in readme


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
    text = _read(relative_path)
    assert "monitor_director_gate" not in text
    assert "stall_episode_state" not in text
