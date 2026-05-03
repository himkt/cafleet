"""Tests for the inlined coding-agent spawn helpers in ``cli.py`` (design 0000046).

Two backends are supported: ``claude`` and ``codex``. The CLI module exposes
five module-level symbols that drive the spawn pipeline:

- ``_CLAUDE_BINARY = "claude"``
- ``_CODEX_BINARY = "codex"``
- ``_MEMBER_PROMPT_TEMPLATE`` — multi-line ``str.format`` template with
  ``{session_id}``, ``{agent_id}``, ``{director_name}``, ``{director_agent_id}``.
  Backend-neutral phrasing (Claude Code uses its ``Skill()`` tool; Codex reads
  ``docs/codex-members.md`` directly).
- ``_build_claude_command(prompt, *, display_name)`` — returns the claude argv.
- ``_build_codex_command(prompt)`` — returns the codex argv (no ``display_name``).
- ``_ensure_coding_agent_available(binary_name)`` — ``RuntimeError`` if the
  binary is missing on PATH. Replaces the older ``_ensure_claude_available``.

Imports are performed inside each test so that collection succeeds even before
Step 4 lands the new symbols. Each test then fails cleanly at run-time until
the implementation catches up — the standard TDD shape for this project.
"""

import importlib

import pytest


def _cli():
    """Re-import ``cafleet.cli`` and return the module.

    ``importlib.reload`` ensures monkeypatches applied to ``cafleet.cli``
    attributes earlier in the run-time do not leak between tests.
    """
    return importlib.import_module("cafleet.cli")


# --- _build_claude_command: argv shape per design-doc §2 ---


def test_build_claude_command__argv_shape():
    cli = _cli()
    assert cli._build_claude_command("PROMPT_TEXT", display_name="Bob") == [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Bob",
        "PROMPT_TEXT",
    ]


def test_build_claude_command__preserves_prompt_with_special_chars():
    cli = _cli()
    prompt = (
        "Review PR #42.\n"
        "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566.\n"
        'Quote: "hello" and {literal_braces}.'
    )
    result = cli._build_claude_command(prompt, display_name="Drafter")
    assert result[-1] == prompt
    assert result[0] == "claude"


def test_build_claude_command__preserves_display_name_with_spaces():
    cli = _cli()
    result = cli._build_claude_command("p", display_name="Code Reviewer")
    assert result == [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Code Reviewer",
        "p",
    ]


# --- _build_codex_command: argv shape per design-doc §2 ---


def test_build_codex_command__argv_shape():
    cli = _cli()
    assert cli._build_codex_command("PROMPT_TEXT") == [
        "codex",
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
        "PROMPT_TEXT",
    ]


def test_build_codex_command__no_display_name_kwarg():
    """Codex has no ``--name`` analog — the helper deliberately omits it."""
    cli = _cli()
    result = cli._build_codex_command("p")
    assert "--name" not in result
    assert result[0] == "codex"
    assert result[-1] == "p"


def test_build_codex_command__preserves_prompt_with_special_chars():
    cli = _cli()
    prompt = (
        "Review PR #42.\n"
        "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566.\n"
        'Quote: "hello" and {literal_braces}.'
    )
    result = cli._build_codex_command(prompt)
    assert result[-1] == prompt
    assert result[0] == "codex"


def test_build_codex_command__permission_tokens_precede_prompt():
    """Pinned argv ordering: workspace-write tokens before the prompt."""
    cli = _cli()
    result = cli._build_codex_command("hello")
    assert result.index("--ask-for-approval") < result.index("--sandbox")
    assert result.index("--sandbox") < result.index("hello")
    assert result[result.index("--ask-for-approval") + 1] == "never"
    assert result[result.index("--sandbox") + 1] == "workspace-write"


# --- _ensure_coding_agent_available: parametrized over both backends ---


@pytest.mark.parametrize("binary_name", ["claude", "codex"])
def test_ensure_coding_agent_available__raises_when_missing(monkeypatch, binary_name):
    cli = _cli()
    monkeypatch.setattr("cafleet.cli.shutil.which", lambda _: None)
    with pytest.raises(RuntimeError) as exc_info:
        cli._ensure_coding_agent_available(binary_name)
    assert f"binary {binary_name} not found on PATH" in str(exc_info.value)


@pytest.mark.parametrize("binary_name", ["claude", "codex"])
def test_ensure_coding_agent_available__silent_when_found(monkeypatch, binary_name):
    cli = _cli()
    monkeypatch.setattr(
        "cafleet.cli.shutil.which", lambda _: f"/usr/bin/{binary_name}"
    )
    assert cli._ensure_coding_agent_available(binary_name) is None


def test_ensure_coding_agent_available__passes_binary_name_to_which(monkeypatch):
    """The helper looks up the *passed-in* binary name, not a hardcoded one."""
    cli = _cli()
    seen: list[str] = []

    def fake_which(name):
        seen.append(name)
        return None

    monkeypatch.setattr("cafleet.cli.shutil.which", fake_which)
    with pytest.raises(RuntimeError):
        cli._ensure_coding_agent_available("codex")
    assert seen == ["codex"]


# --- _MEMBER_PROMPT_TEMPLATE: backend-neutral, all placeholders, format() works ---


_STANDARD_KWARGS = {
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
    "director_name": "Alice",
    "director_agent_id": "dir-001",
}


def test_member_prompt_template__has_required_placeholders():
    cli = _cli()
    template = cli._MEMBER_PROMPT_TEMPLATE
    assert "{session_id}" in template
    assert "{agent_id}" in template
    assert "{director_name}" in template
    assert "{director_agent_id}" in template


def test_member_prompt_template__phrasing_is_backend_neutral():
    """The template no longer hardcodes 'Load Skill(cafleet).'.

    Both backends must be addressed: Claude Code via its ``Skill()`` tool,
    Codex via reading ``docs/codex-members.md`` directly. Per design 0000046 §4.
    """
    cli = _cli()
    template = cli._MEMBER_PROMPT_TEMPLATE
    # The bare directive that was claude-only must be gone.
    assert "Load Skill(cafleet)." not in template
    # Both backends must be addressed by name.
    assert "Claude Code" in template
    assert "Codex" in template
    # The codex doc pointer must be present so codex members can self-orient.
    assert "docs/codex-members.md" in template


def test_member_prompt_template__format_succeeds_with_standard_kwargs():
    cli = _cli()
    template = cli._MEMBER_PROMPT_TEMPLATE
    result = template.format(**_STANDARD_KWARGS)
    assert "550e8400-e29b-41d4-a716-446655440000" in result
    assert "7ba91234-5678-90ab-cdef-112233445566" in result
    assert "Alice" in result
    assert "dir-001" in result
    # Substitution is total: no raw placeholders survive the .format() call.
    assert "{session_id}" not in result
    assert "{agent_id}" not in result
    assert "{director_name}" not in result
    assert "{director_agent_id}" not in result
