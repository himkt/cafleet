"""Tests for ``cafleet.cli._MEMBER_PROMPT_TEMPLATE``."""

import importlib


def _cli():
    """Return the ``cafleet.cli`` module via ``importlib.import_module``.

    Per-test monkeypatches are scoped to pytest's ``monkeypatch`` fixture,
    which auto-reverts between tests, so no module reload is needed.
    """
    return importlib.import_module("cafleet.cli")


_STANDARD_KWARGS = {
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
    "director_name": "Alice",
    "director_agent_id": "dir-001",
}


def test_member_prompt_template__has_required_placeholders():
    """Post-Surface-6: the slim spawn prompt drops ``{director_name}``."""
    cli = _cli()
    template = cli._MEMBER_PROMPT_TEMPLATE
    assert "{session_id}" in template
    assert "{agent_id}" in template
    assert "{director_name}" not in template
    assert "{director_agent_id}" in template


def test_member_prompt_template__phrasing_is_short_and_backend_neutral():
    """Post-Surface-6: the slim 2-line spawn prompt drops the codex/claude
    branch from the template body. Codex orientation lives in the cafleet
    skill core SKILL.md, not the spawn prompt itself."""
    cli = _cli()
    template = cli._MEMBER_PROMPT_TEMPLATE
    # Surface 6 budget: ≤ 70 tokens post-substitution. Loose proxy: keep
    # the template under 200 codepoints (excluding placeholders).
    assert len(template) < 250, (
        f"template too long ({len(template)} codepoints): {template!r}"
    )
    # The skill load directive remains backend-neutral.
    assert "skill 'cafleet'" in template


def test_member_prompt_template__format_succeeds_with_standard_kwargs():
    cli = _cli()
    template = cli._MEMBER_PROMPT_TEMPLATE
    kwargs = {k: v for k, v in _STANDARD_KWARGS.items() if k != "director_name"}
    result = template.format(**kwargs)
    assert "550e8400-e29b-41d4-a716-446655440000" in result
    assert "7ba91234-5678-90ab-cdef-112233445566" in result
    assert "dir-001" in result
    # Substitution is total: no raw placeholders survive the .format() call.
    assert "{session_id}" not in result
    assert "{agent_id}" not in result
    assert "{director_agent_id}" not in result
