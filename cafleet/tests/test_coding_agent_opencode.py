"""Unit tests for ``cafleet.coding_agent.opencode.OpencodeAgent``.

Verifies the byte-exact spawn argv (with regression guards against
re-adding the dropped flags), the ``display_name`` ignore semantics, the
registry wiring, and the ``ensure_available`` integration with
``materialize_cafleet_agent``.
"""

import pytest

from cafleet.coding_agent import (
    CODING_AGENTS,
    CodingAgent,
    OpencodeAgent,
)
from cafleet.coding_agent.opencode_preset import CAFLEET_AGENT

# ---------------------------------------------------------------------------
# Registry wiring (§2)
# ---------------------------------------------------------------------------


def test_opencode_is_registered_in_coding_agents():
    """Per §2: ``CODING_AGENTS["opencode"]`` is an ``OpencodeAgent`` instance.
    Click's ``--coding-agent`` choice list picks up ``"opencode"`` automatically
    via ``list(CODING_AGENTS.keys())`` at ``cli.py:893`` and ``cli.py:260``."""
    assert "opencode" in CODING_AGENTS
    assert isinstance(CODING_AGENTS["opencode"], OpencodeAgent)


def test_opencode_satisfies_coding_agent_protocol():
    """OpencodeAgent is structurally compatible with the CodingAgent Protocol."""
    assert isinstance(OpencodeAgent(), CodingAgent)


def test_opencode_agent_name_and_binary_name():
    """Per §1: ``name == "opencode"`` matches the registry key and
    ``binary_name == "opencode"`` is what ``shutil.which`` resolves."""
    impl = OpencodeAgent()
    assert impl.name == "opencode"
    assert impl.binary_name == "opencode"


# ---------------------------------------------------------------------------
# build_spawn_argv (§1) — byte-exact shape + regression guards
# ---------------------------------------------------------------------------


def test_build_spawn_argv_byte_exact():
    """Per §1: spawn argv is exactly ``["opencode", "--agent", "cafleet",
    "--prompt", <prompt>]`` — pinned token list and ordering."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("PROMPT_TEXT", display_name="ignored")
    assert argv == [
        "opencode",
        "--agent",
        "cafleet",
        "--prompt",
        "PROMPT_TEXT",
    ]


def test_build_spawn_argv_does_not_pass_run_subcommand():
    """``opencode run`` is the headless / scripting entry; the spawn argv
    MUST NOT contain ``"run"`` as a subcommand token."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("p", display_name="x")
    assert "run" not in argv


def test_build_spawn_argv_does_not_pass_interactive_flag():
    """``--interactive`` is an internal flag of the ``opencode run``
    subcommand (not in public docs); the spawn argv MUST NOT contain it."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("p", display_name="x")
    assert "--interactive" not in argv


def test_build_spawn_argv_does_not_pass_dangerously_skip_permissions():
    """``--dangerously-skip-permissions`` is documented only under
    ``opencode run`` and is silently ignored in interactive code paths.
    Regression guard against accidentally re-adding it."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("p", display_name="x")
    assert "--dangerously-skip-permissions" not in argv


def test_build_spawn_argv_passes_prompt_via_prompt_flag_not_positional():
    """Per §1: bare ``opencode`` takes ``[project]`` as its positional, NOT
    ``[message..]``. The prompt MUST be passed as the value of ``--prompt``.
    Without this, opencode would silently misinterpret the prompt as a
    project path."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("PROMPT_TEXT", display_name="x")
    prompt_flag_idx = argv.index("--prompt")
    assert argv[prompt_flag_idx + 1] == "PROMPT_TEXT"


def test_build_spawn_argv_includes_agent_cafleet_flag():
    """Per §1: ``--agent cafleet`` binds the spawn to the CAFleet permission
    ruleset. Without it the spawn falls back to opencode's default ``build``
    agent (catch-all allow) and the safety floor is lost."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("p", display_name="x")
    agent_flag_idx = argv.index("--agent")
    assert argv[agent_flag_idx + 1] == "cafleet"


def test_build_spawn_argv_ignores_display_name():
    """Per §1: opencode has no ``--name`` analog so ``display_name`` is
    silently ignored — confirmed by argv equivalence across different
    display_name values."""
    impl = OpencodeAgent()
    argv_a = impl.build_spawn_argv("p", display_name="Alice")
    argv_b = impl.build_spawn_argv("p", display_name="Bob With Spaces")
    argv_c = impl.build_spawn_argv("p", display_name="")
    assert argv_a == argv_b == argv_c
    assert "Alice" not in " ".join(argv_a)
    assert "Bob" not in " ".join(argv_b)


def test_build_spawn_argv_preserves_prompt_with_special_chars():
    """Mirrors the existing claude / codex tests: special characters in the
    prompt survive intact as a single argv element."""
    impl = OpencodeAgent()
    prompt = 'Review PR #42.\nUse --agent-id 42.\nQuote: "hello" and {literal_braces}.'
    argv = impl.build_spawn_argv(prompt, display_name="ignored")
    # The prompt is the last element (value of --prompt) and unmodified.
    assert argv[-1] == prompt
    assert argv[0] == "opencode"


def test_build_spawn_argv_returns_list_of_strings():
    """Protocol-shape sanity: every element is a string, type is ``list``."""
    impl = OpencodeAgent()
    argv = impl.build_spawn_argv("p", display_name="x")
    assert isinstance(argv, list)
    assert all(isinstance(token, str) for token in argv)


# ---------------------------------------------------------------------------
# ensure_available (§1, §4.2) — PATH probe + materialization integration
# ---------------------------------------------------------------------------


def test_ensure_available_calls_materialize_with_cafleet_agent(tmp_path, monkeypatch):
    """Per §1: ``ensure_available`` does two things — PATH probe + materialize.
    Verifies (c) of the Step-5 task: a monkeypatched spy on
    ``materialize_cafleet_agent`` is called exactly once with ``CAFLEET_AGENT``.

    Spy is installed on the symbol the ``opencode`` module imported (function
    objects are bound at import time, so patching the source module does not
    affect the call site)."""
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")

    calls: list = []

    def spy(definition):
        calls.append(definition)

    monkeypatch.setattr("cafleet.coding_agent.opencode.materialize_cafleet_agent", spy)

    OpencodeAgent().ensure_available()

    assert len(calls) == 1
    assert calls[0] is CAFLEET_AGENT


def test_ensure_available_raises_when_binary_missing(tmp_path, monkeypatch):
    """Per the shared ``ensure_binary_on_path`` contract: missing binary
    raises ``RuntimeError`` BEFORE any materialization side effect runs."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda _: None)

    with pytest.raises(RuntimeError, match="not found on PATH"):
        OpencodeAgent().ensure_available()

    # The deny-list file must NOT be materialized when the binary is absent.
    assert not (tmp_path / ".opencode" / "agents" / "cafleet.md").exists()


def test_ensure_available_materializes_preset_on_first_call(tmp_path, monkeypatch):
    """End-to-end: binary available + HOME pointed at tmp_path →
    ``~/.opencode/agents/cafleet.md`` is materialized with the rendered
    preset content."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")

    OpencodeAgent().ensure_available()

    target = tmp_path / ".opencode" / "agents" / "cafleet.md"
    assert target.exists()
    assert target.read_text(encoding="utf-8") == CAFLEET_AGENT.to_markdown()


def test_ensure_available_is_idempotent_on_second_call(tmp_path, monkeypatch):
    """Per §1: ``ensure_available`` is safe to call on every spawn — the
    skip-if-exists rule makes the second call a cheap no-op that does NOT
    overwrite a customized preset file."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")

    target = tmp_path / ".opencode" / "agents" / "cafleet.md"
    target.parent.mkdir(parents=True)
    custom = "# customized by operator\n"
    target.write_text(custom, encoding="utf-8")

    OpencodeAgent().ensure_available()
    OpencodeAgent().ensure_available()

    assert target.read_text(encoding="utf-8") == custom
