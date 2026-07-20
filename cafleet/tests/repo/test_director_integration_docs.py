"""Fixture assertions for the Director-side integration contract: the documented
pre-spawn selection step, unchanged flag forwarding to ``member create``, the
two-phase ``.selection/`` audit rules with the ``<unset>`` fail-closed path, and
the underpowered-member replacement protocol."""

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
DIRECTOR_REFERENCE = REPO_ROOT / "skills" / "cafleet" / "reference" / "director.md"


def _director_text():
    return DIRECTOR_REFERENCE.read_text(encoding="utf-8")


def test_director_documents_pre_spawn_selection_step():
    text = _director_text()
    assert "cafleet model select --model-list" in text
    assert "--role monitor" in text


def test_director_forwards_selector_flags_unchanged():
    text = _director_text()
    assert "Pass the returned `selected.backend` and `selected.model` unchanged" in text
    assert "--coding-agent" in text
    assert "--model" in text


def test_director_documents_two_phase_selection_audit():
    text = _director_text()
    assert ".selection/<selection_id>.pending.json" in text
    assert '"created"' in text
    assert '"failed"' in text


def test_director_documents_unset_base_fail_closed():
    text = _director_text()
    assert "MODEL_SELECTION_AUDIT_UNAVAILABLE" in text
    assert "<unset>" in text


def test_replacement_protocol_documents_evidence_classes():
    text = _director_text()
    assert "self-report" in text
    assert "[INCORRECT]" in text
    assert "member capture" in text


def test_replacement_protocol_deletes_old_member_before_respawn():
    text = _director_text()
    assert "member delete" in text
    assert "before spawning the replacement" in text


def test_replacement_protocol_documents_caps_and_no_retry():
    text = _director_text()
    assert "at most two replacements" in text
    assert "(task pointer, model key)" in text
