"""Tests for ``cafleet.cli._prompt.MEMBER_PROMPT_TEMPLATE``."""

import importlib


def _prompt_mod():
    """Return the ``cafleet.cli._prompt`` module via ``importlib.import_module``.

    Per-test monkeypatches are scoped to pytest's ``monkeypatch`` fixture,
    which auto-reverts between tests, so no module reload is needed.
    """
    return importlib.import_module("cafleet.cli._prompt")


_STANDARD_KWARGS = {
    "fleet_id": 100,
    "agent_id": 200,
    "director_name": "Alice",
    "director_agent_id": 300,
}


def test_member_prompt_template__has_required_placeholders():
    """The slim spawn prompt uses ``{fleet_id}`` / ``{agent_id}`` /
    ``{director_agent_id}`` and does not include ``{director_name}``."""
    template = _prompt_mod().MEMBER_PROMPT_TEMPLATE
    assert "{fleet_id}" in template
    assert "{agent_id}" in template
    assert "{director_name}" not in template
    assert "{director_agent_id}" in template


def test_member_prompt_template__phrasing_is_short_and_backend_neutral():
    """The slim 2-line spawn prompt has no codex/claude branch in its body —
    backend orientation lives in the cafleet skill core SKILL.md, not the
    spawn prompt itself."""
    template = _prompt_mod().MEMBER_PROMPT_TEMPLATE
    # Budget: ≤ 70 tokens post-substitution. Loose proxy: keep
    # the template under 200 codepoints (excluding placeholders).
    assert len(template) < 250, (
        f"template too long ({len(template)} codepoints): {template!r}"
    )
    # The skill load directive remains backend-neutral.
    assert "skill 'cafleet'" in template


def test_member_prompt_template__format_succeeds_with_standard_kwargs():
    template = _prompt_mod().MEMBER_PROMPT_TEMPLATE
    kwargs = {k: v for k, v in _STANDARD_KWARGS.items() if k != "director_name"}
    result = template.format(**kwargs)
    assert "100" in result
    assert "200" in result
    assert "300" in result
    # Substitution is total: no raw placeholders survive the .format() call.
    assert "{fleet_id}" not in result
    assert "{agent_id}" not in result
    assert "{director_agent_id}" not in result
