"""Unit tests for ``cafleet.coding_agent.opencode.OpencodeAgent``.

Verifies the byte-exact spawn argv (with regression guards against
re-adding the dropped flags), the ``display_name`` ignore semantics, the
registry wiring, and the ``ensure_available`` spawn preconditions (PATH
probe + preset-file existence).
"""

import re

import pytest

from cafleet.coding_agent import (
    CODING_AGENTS,
    OpencodeAgent,
)

# ---------------------------------------------------------------------------
# Registry wiring (§2)
# ---------------------------------------------------------------------------


def test_opencode_is_registered_in_coding_agents():
    """Per §2: ``CODING_AGENTS["opencode"]`` is an ``OpencodeAgent`` instance.
    Click's ``--coding-agent`` choice list picks up ``"opencode"`` automatically
    via ``list(CODING_AGENTS.keys())`` in ``cli/member.py`` and ``cli/fleet.py``."""
    assert "opencode" in CODING_AGENTS
    assert isinstance(CODING_AGENTS["opencode"], OpencodeAgent)
    # ``binary_name`` is what ``shutil.which`` resolves — no retained duplicate
    # elsewhere (``name`` and Protocol conformance are covered by test_protocol.py).
    assert OpencodeAgent().binary_name == "opencode"


# ---------------------------------------------------------------------------
# build_spawn_argv (§1) — byte-exact shape + regression guards
# ---------------------------------------------------------------------------


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
    prompt = 'Review PR #42.\nUse --member-id 42.\nQuote: "hello" and {literal_braces}.'
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
# validate_model — <provider-id>/<model-id> first-slash format rule
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "model",
    [
        "anthropic/claude-sonnet-4-6",
        "openai/gpt-5.5",
        "a/b/c",
    ],
)
def test_validate_model_accepts_provider_slash_model(model):
    """First-slash split into two non-empty segments. ``a/b/c`` is accepted as
    provider ``a`` + model id ``b/c`` — model ids may themselves contain
    slashes."""
    assert OpencodeAgent().validate_model(model) is None


@pytest.mark.parametrize("model", ["no-slash", "/x", "x/", ""])
def test_validate_model_rejects_malformed_with_exact_message(model):
    """No ``/``, empty provider segment, empty model-id segment, and the empty
    string are all rejected with the documented error message."""
    expected = (
        "--model for the opencode backend must be "
        f"'<provider-id>/<model-id>' (got '{model}')."
    )
    with pytest.raises(ValueError, match=f"^{re.escape(expected)}$"):
        OpencodeAgent().validate_model(model)


# ---------------------------------------------------------------------------
# ensure_available — PATH probe + preset-existence spawn precondition
# ---------------------------------------------------------------------------


def _preset_path(home):
    return home / ".opencode" / "agents" / "cafleet.md"


def test_ensure_available_raises_when_binary_missing(tmp_path, monkeypatch):
    """Per the shared ``ensure_binary_on_path`` contract: a missing binary
    raises the PATH ``RuntimeError`` first — with the preset also absent, the
    PATH error is the one reported."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda _: None)

    with pytest.raises(RuntimeError, match="not found on PATH"):
        OpencodeAgent().ensure_available()


def test_ensure_available_raises_when_preset_missing(tmp_path, monkeypatch):
    """Binary on PATH but no ``~/.opencode/agents/cafleet.md``: the spawn
    precondition fails with guidance to run ``cafleet setup`` — and
    ``ensure_available`` writes no file."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")

    preset = _preset_path(tmp_path)
    expected = (
        f"opencode agent preset not found at {preset}; "
        "run 'cafleet setup' first"
    )
    with pytest.raises(RuntimeError, match=f"^{re.escape(expected)}$"):
        OpencodeAgent().ensure_available()

    assert not preset.exists()


def test_ensure_available_rejects_preset_directory(tmp_path, monkeypatch):
    """The existence check is ``is_file()``: a directory squatting on the
    preset path fails the precondition with the same missing-preset error."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")

    _preset_path(tmp_path).mkdir(parents=True)

    with pytest.raises(RuntimeError, match="opencode agent preset not found at"):
        OpencodeAgent().ensure_available()


def test_ensure_available_succeeds_when_preset_exists(tmp_path, monkeypatch):
    """Binary on PATH + preset file present: the check passes and the file is
    left untouched (read-only precondition, no rewrite)."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")

    preset = _preset_path(tmp_path)
    preset.parent.mkdir(parents=True)
    content = "# installed by cafleet setup\n"
    preset.write_text(content, encoding="utf-8")

    assert OpencodeAgent().ensure_available() is None
    assert preset.read_text(encoding="utf-8") == content
