"""Contract tests for the ``CodingAgent`` Protocol and registered impls.

The parametrized Protocol-shape assertions guard against signature drift —
adding a method to ``CodingAgent`` that an impl forgets surfaces here. The
per-impl byte-exact argv assertions pin the spawn argv emitted by
``ClaudeCodeAgent.build_spawn_argv``, ``CodexAgent.build_spawn_argv``, and
``OpencodeAgent.build_spawn_argv`` to the exact token list (and ordering)
that the multiplexer's ``split_window`` expects when launching the
coding-agent process.
"""

import pytest

from cafleet.coding_agent import CODING_AGENTS, CodingAgent


@pytest.fixture(autouse=True)
def _redirect_home_for_opencode_materialize(tmp_path, monkeypatch):
    """Redirect HOME → ``tmp_path`` for every test in this file.

    ``OpencodeAgent.ensure_available()`` materializes the CAFleet agent
    preset to ``~/.opencode/agents/cafleet.md`` as a spawn precondition.
    When the protocol-contract parametrization
    runs the ``ensure_available`` cases for OpencodeAgent, that write
    would otherwise pollute the real user's ``$HOME``. Redirecting HOME
    is a no-op for claude / codex (they do not touch ``$HOME``) and
    contains the opencode side effect inside the per-test ``tmp_path``."""
    monkeypatch.setenv("HOME", str(tmp_path))


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
    prompt = 'Review PR #42.\nUse --member-id 42.\nQuote: "hello" and {literal_braces}.'
    argv = impl.build_spawn_argv(prompt, display_name="Drafter")
    assert argv[-1] == prompt
    assert argv[0] == "claude"


def test_codex_build_spawn_argv__preserves_prompt_with_special_chars():
    impl = CODING_AGENTS["codex"]
    prompt = 'Review PR #42.\nUse --member-id 42.\nQuote: "hello" and {literal_braces}.'
    argv = impl.build_spawn_argv(prompt, display_name="ignored")
    assert argv[-1] == prompt
    assert argv[0] == "codex"


# --- ``model`` keyword: validate_model + argv emission ---


_PINNED_ARGV_WITHOUT_MODEL = {
    "claude": [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Bob",
        "PROMPT_TEXT",
    ],
    "codex": [
        "codex",
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
        "PROMPT_TEXT",
    ],
    "opencode": [
        "opencode",
        "--agent",
        "cafleet",
        "--prompt",
        "PROMPT_TEXT",
    ],
}


@pytest.mark.parametrize("name", list(CODING_AGENTS))
def test_build_spawn_argv__model_omitted_is_byte_identical_to_pinned(name):
    """Omitting ``model`` keeps the spawn argv byte-identical to the pinned list."""
    argv = CODING_AGENTS[name].build_spawn_argv("PROMPT_TEXT", display_name="Bob")
    assert argv == _PINNED_ARGV_WITHOUT_MODEL[name]


@pytest.mark.parametrize("name", list(CODING_AGENTS))
def test_build_spawn_argv__model_none_explicit_is_byte_identical_to_pinned(name):
    """``model=None`` emits no model tokens — same argv as omitting the keyword."""
    argv = CODING_AGENTS[name].build_spawn_argv(
        "PROMPT_TEXT", display_name="Bob", model=None
    )
    assert argv == _PINNED_ARGV_WITHOUT_MODEL[name]


def test_claude_build_spawn_argv__model_set_byte_exact():
    """``--model <m>`` lands between ``--name <display>`` and the prompt."""
    impl = CODING_AGENTS["claude"]
    assert impl.build_spawn_argv("PROMPT_TEXT", display_name="Bob", model="sonnet") == [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Bob",
        "--model",
        "sonnet",
        "PROMPT_TEXT",
    ]


def test_codex_build_spawn_argv__model_set_byte_exact():
    """``--model <m>`` lands between ``workspace-write`` and the prompt."""
    impl = CODING_AGENTS["codex"]
    assert impl.build_spawn_argv(
        "PROMPT_TEXT", display_name="ignored", model="gpt-5.4-mini"
    ) == [
        "codex",
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
        "--model",
        "gpt-5.4-mini",
        "PROMPT_TEXT",
    ]


def test_opencode_build_spawn_argv__model_set_byte_exact():
    """``--model <m>`` lands between ``cafleet`` and the ``--prompt`` pair."""
    impl = CODING_AGENTS["opencode"]
    assert impl.build_spawn_argv(
        "PROMPT_TEXT", display_name="ignored", model="anthropic/claude-sonnet-4-6"
    ) == [
        "opencode",
        "--agent",
        "cafleet",
        "--model",
        "anthropic/claude-sonnet-4-6",
        "--prompt",
        "PROMPT_TEXT",
    ]


@pytest.mark.parametrize("name", ["claude", "codex"])
@pytest.mark.parametrize(
    "model",
    ["sonnet", "gpt-5.4-mini", "", "   ", "anthropic/claude-sonnet-4-6", "no-such"],
)
def test_validate_model__claude_codex_accept_any_string(name, model):
    """Pass-through policy: the backend binary itself rejects unknown models,
    so newly released models work without a cafleet release. Empty and
    whitespace-only strings pass too — no CLI-level emptiness check."""
    assert CODING_AGENTS[name].validate_model(model) is None


@pytest.mark.parametrize("name", list(CODING_AGENTS))
def test_validate_model__none_is_always_valid(name):
    """``None`` (flag omitted) is valid for every backend."""
    assert CODING_AGENTS[name].validate_model(None) is None
