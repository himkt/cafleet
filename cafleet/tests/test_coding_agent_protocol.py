"""Contract tests for the ``CodingAgent`` Protocol and registered impls.

The parametrized Protocol-shape assertions guard against signature drift —
adding a method to ``CodingAgent`` that an impl forgets surfaces here. The
per-impl byte-exact argv assertions pin the spawn argv emitted by
``ClaudeCodeAgent.build_spawn_argv`` and ``CodexAgent.build_spawn_argv`` to
the exact token list (and ordering) that the multiplexer's ``split_window``
expects when launching the coding-agent process.
"""

import pytest

from cafleet.coding_agent import CODING_AGENTS, CodingAgent


@pytest.mark.parametrize(("name", "impl"), list(CODING_AGENTS.items()))
def test_impl_satisfies_protocol(name, impl):
    assert isinstance(impl, CodingAgent)
    assert impl.name == name


@pytest.mark.parametrize("impl", list(CODING_AGENTS.values()))
def test_build_spawn_argv_includes_prompt_last(impl):
    argv = impl.build_spawn_argv("<PROMPT>", display_name="some-name")
    assert isinstance(argv, list)
    assert all(isinstance(x, str) for x in argv)
    assert argv[-1] == "<PROMPT>"
    assert argv[0] == impl.binary_name


@pytest.mark.parametrize("impl", list(CODING_AGENTS.values()))
def test_ensure_available_raises_when_binary_missing(impl, monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: None)
    with pytest.raises(RuntimeError, match="not found on PATH"):
        impl.ensure_available()


@pytest.mark.parametrize("impl", list(CODING_AGENTS.values()))
def test_ensure_available_silent_when_binary_found(impl, monkeypatch):
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")
    assert impl.ensure_available() is None


# --- Per-impl byte-exact argv assertions (Programmer's Step-4 note) ---


def test_claude_build_spawn_argv__byte_exact():
    """Claude argv: ``[claude, --permission-mode, dontAsk, --name, <display>, <prompt>]`` verbatim."""
    impl = CODING_AGENTS["claude"]
    assert impl.build_spawn_argv("PROMPT_TEXT", display_name="Bob") == [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Bob",
        "PROMPT_TEXT",
    ]


def test_claude_build_spawn_argv__preserves_display_name_with_spaces():
    impl = CODING_AGENTS["claude"]
    assert impl.build_spawn_argv("p", display_name="Code Reviewer") == [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Code Reviewer",
        "p",
    ]


def test_claude_build_spawn_argv__preserves_prompt_with_special_chars():
    impl = CODING_AGENTS["claude"]
    prompt = (
        "Review PR #42.\n"
        "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566.\n"
        'Quote: "hello" and {literal_braces}.'
    )
    argv = impl.build_spawn_argv(prompt, display_name="Drafter")
    assert argv[-1] == prompt
    assert argv[0] == "claude"


def test_codex_build_spawn_argv__byte_exact():
    """Codex argv: ``[codex, --ask-for-approval, never, --sandbox, workspace-write, <prompt>]`` verbatim — no ``--name``."""
    impl = CODING_AGENTS["codex"]
    argv = impl.build_spawn_argv("PROMPT_TEXT", display_name="ignored")
    assert argv == [
        "codex",
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
        "PROMPT_TEXT",
    ]
    assert "--name" not in argv


def test_codex_build_spawn_argv__permission_tokens_precede_prompt():
    """Pinned ordering: ``--ask-for-approval`` < ``--sandbox`` < prompt."""
    impl = CODING_AGENTS["codex"]
    argv = impl.build_spawn_argv("hello", display_name="x")
    assert argv.index("--ask-for-approval") < argv.index("--sandbox")
    assert argv.index("--sandbox") < argv.index("hello")
    assert argv[argv.index("--ask-for-approval") + 1] == "never"
    assert argv[argv.index("--sandbox") + 1] == "workspace-write"


def test_codex_build_spawn_argv__preserves_prompt_with_special_chars():
    impl = CODING_AGENTS["codex"]
    prompt = (
        "Review PR #42.\n"
        "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566.\n"
        'Quote: "hello" and {literal_braces}.'
    )
    argv = impl.build_spawn_argv(prompt, display_name="ignored")
    assert argv[-1] == prompt
    assert argv[0] == "codex"
