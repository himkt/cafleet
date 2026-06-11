"""Tests for ``resolve_prompt`` and ``cafleet member create``.

Covers placeholder substitution (default + custom + doubled brace + error
branches), `--prompt-file` validation matrix, spawn-argv shape per backend,
permission-mode injection, and binary-missing exit messages.
"""

import json
import os
import sys

import click
import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import cli
from cafleet.cli._prompt import resolve_prompt
from cafleet.multiplexer import MultiplexerContext as DirectorContext


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture
def director_agent_id():
    return 200


@pytest.fixture
def new_agent_id():
    return 300


@pytest.fixture
def ctx(fleet_id):
    command = click.Command("member-create")
    context = click.Context(command)
    context.obj = {"fleet_id": fleet_id, "json_output": False}
    return context


@pytest.fixture
def mock_get_agent(monkeypatch):
    def fake_get_agent(agent_id, fleet_id):
        return {"agent_id": agent_id, "name": "Director-X"}

    monkeypatch.setattr(broker, "get_agent", fake_get_agent)
    return fake_get_agent


_CLI_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def bootstrapped_fleet(tmp_path, monkeypatch, _reset_engine_singletons):
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available",
        lambda self: None,
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.context_discovery",
        lambda self: _CLI_FAKE_DIRECTOR_CTX,
    )

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    create = runner.invoke(cli, ["fleet", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    return data["fleet_id"], data["director"]["agent_id"], runner


@pytest.fixture
def split_window_recorder(monkeypatch):
    calls: list[dict] = []

    def fake_split_window(self, **kwargs):
        calls.append(kwargs)
        return "%42"

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.split_window", fake_split_window
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.select_layout",
        lambda self, **_: None,
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_exit",
        lambda self, **_: None,
        raising=False,
    )
    return calls


@pytest.fixture
def stub_coding_agent_binaries(monkeypatch):
    monkeypatch.setattr(
        "cafleet.coding_agent.base.shutil.which", lambda _: "/usr/bin/stub"
    )


def _invoke_member_create(
    runner: CliRunner,
    fleet_id: int,
    director_id: int,
    *,
    coding_agent: str = "claude",
    prompt_file: str | None = None,
    inline_prompt: str | None = None,
    name: str = "Member",
    json_output: bool = False,
    model: str | None = None,
):
    args = ["--fleet-id", str(fleet_id)]
    if json_output:
        args.append("--json")
    args.extend(
        [
            "member",
            "create",
            "--agent-id",
            str(director_id),
            "--name",
            name,
            "--description",
            f"{name} for tests",
        ]
    )
    if coding_agent != "claude":
        args.extend(["--coding-agent", coding_agent])
    if model is not None:
        args.extend(["--model", model])
    if prompt_file is not None:
        args.extend(["--prompt-file", prompt_file])
    if inline_prompt is not None:
        args.extend(["--", inline_prompt])
    return runner.invoke(cli, args)


@pytest.mark.parametrize(
    ("scenario", "prompt_argv", "asserts"),
    [
        ("default_path_all_placeholders_substituted", (), "default"),
        (
            "custom_path_substitutes_agent_id",
            ("message", "for", "{agent_id}"),
            "agent_id_only",
        ),
        (
            "custom_path_no_placeholders_passthrough",
            ("no", "placeholders", "here"),
            "passthrough",
        ),
        (
            "doubled_brace_collapses_to_single",
            ("data", "is", "{{not", "a", "placeholder}}", "closed"),
            "doubled_brace",
        ),
    ],
)
def test_resolve_prompt__substitution_matrix(
    ctx,
    director_agent_id,
    new_agent_id,
    fleet_id,
    mock_get_agent,
    scenario,
    prompt_argv,
    asserts,
):
    result = resolve_prompt(
        ctx,
        director_agent_id=director_agent_id,
        new_agent_id=new_agent_id,
        prompt_argv=prompt_argv,
    )
    if asserts == "default":
        assert str(fleet_id) in result
        assert str(new_agent_id) in result
        assert str(director_agent_id) in result
        for raw in ("{fleet_id}", "{agent_id}", "{director_agent_id}"):
            assert raw not in result
    elif asserts == "agent_id_only":
        assert result == f"message for {new_agent_id}"
    elif asserts == "passthrough":
        assert result == "no placeholders here"
    else:
        assert result == "data is {not a placeholder} closed"
        assert str(new_agent_id) not in result
        assert str(director_agent_id) not in result


@pytest.mark.parametrize(
    ("scenario", "prompt_argv", "expect_message_contains"),
    [
        (
            "unknown_placeholder",
            ("hello", "{foo}"),
            ("foo", "{fleet_id}", "{agent_id}"),
        ),
        ("unmatched_brace", ("hello", "{unclosed"), ("{{", "}}")),
        ("attribute_access", ("hello", "{agent_id.foo}"), ("{{", "}}")),
    ],
)
def test_resolve_prompt__malformed_raises_usage_error(
    ctx,
    director_agent_id,
    new_agent_id,
    mock_get_agent,
    scenario,
    prompt_argv,
    expect_message_contains,
):
    with pytest.raises(click.UsageError) as exc_info:
        resolve_prompt(
            ctx,
            director_agent_id=director_agent_id,
            new_agent_id=new_agent_id,
            prompt_argv=prompt_argv,
        )
    message = str(exc_info.value)
    for needle in expect_message_contains:
        assert needle in message


def test_prompt_file__relative_path_rejected(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        prompt_file="./foo.md",
    )
    assert result.exit_code == 2, result.output
    assert "--prompt-file requires an absolute path" in result.output
    assert "./foo.md" in result.output
    assert split_window_recorder == []


@pytest.mark.parametrize(
    ("scenario", "fixture_setup", "expected_exit", "expected_substring"),
    [
        ("not_found", "missing", 1, "file does not exist or is not a regular file"),
        (
            "directory_not_regular",
            "directory",
            1,
            "file does not exist or is not a regular file",
        ),
        ("empty_zero_bytes", "empty", 1, "file is empty"),
        ("empty_whitespace_only", "whitespace", 1, "file is empty"),
        ("invalid_utf8", "bad_utf8", 1, "file is not valid UTF-8"),
        ("unknown_placeholder", "unknown_placeholder", 2, "Unknown placeholder"),
    ],
)
def test_prompt_file__error_variants(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    scenario,
    fixture_setup,
    expected_exit,
    expected_substring,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    if fixture_setup == "missing":
        target = tmp_path / "does-not-exist.md"
    elif fixture_setup == "directory":
        target = tmp_path / "subdir"
        target.mkdir()
    elif fixture_setup == "empty":
        target = tmp_path / "empty.md"
        target.write_bytes(b"")
    elif fixture_setup == "whitespace":
        target = tmp_path / "ws.md"
        target.write_text("\n   \t\n", encoding="utf-8")
    elif fixture_setup == "bad_utf8":
        target = tmp_path / "bad.md"
        target.write_bytes(b"\xff\xfe\xfd")
    else:
        target = tmp_path / "prompt.md"
        target.write_text("hi {unknown}", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        prompt_file=str(target),
    )
    assert result.exit_code == expected_exit, result.output
    assert expected_substring in result.output
    assert split_window_recorder == []


@pytest.mark.skipif(
    sys.platform == "win32" or (hasattr(os, "geteuid") and os.geteuid() == 0),
    reason="POSIX-only test; root bypasses the read-permission check",
)
def test_prompt_file__not_readable_exits_with_message(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    unreadable_file = tmp_path / "unreadable.md"
    unreadable_file.write_text("hello", encoding="utf-8")
    unreadable_file.chmod(0o000)
    try:
        result = _invoke_member_create(
            runner,
            fleet_id,
            director_id,
            prompt_file=str(unreadable_file),
        )
        assert result.exit_code == 1, result.output
        assert "file is not readable" in result.output
        assert str(unreadable_file) in result.output
        assert split_window_recorder == []
    finally:
        unreadable_file.chmod(0o644)


def test_prompt_file__mutually_exclusive_with_positional(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("hello", encoding="utf-8")
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        prompt_file=str(prompt_path),
        inline_prompt="hello positional",
    )
    assert result.exit_code == 2, result.output
    assert (
        "--prompt-file and the positional prompt argument are mutually exclusive"
        in result.output
    )
    assert split_window_recorder == []


def test_prompt_file__parity_with_positional_form(tmp_path):
    fleet_id = 100
    director_id = 200
    new_agent_id = 300
    template = "hello {agent_id} from director {director_agent_id}"

    command = click.Command("member-create")
    ctx = click.Context(command)
    ctx.obj = {"fleet_id": fleet_id, "json_output": False}

    inline_result = resolve_prompt(
        ctx,
        director_agent_id=director_id,
        new_agent_id=new_agent_id,
        prompt_argv=tuple(template.split(" ")),
        prompt_file=None,
    )
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text(template, encoding="utf-8")
    file_result = resolve_prompt(
        ctx,
        director_agent_id=director_id,
        new_agent_id=new_agent_id,
        prompt_argv=(),
        prompt_file=str(prompt_path),
    )

    assert inline_result == file_result
    assert str(new_agent_id) in file_result
    assert str(director_id) in file_result


@pytest.mark.parametrize(
    ("scenario", "content"),
    [
        ("preserves_trailing_newline", "hello\n"),
        ("preserves_surrounding_whitespace", "   \n  hello world  \n   "),
    ],
)
def test_prompt_file__preserves_whitespace_verbatim(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    scenario,
    content,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text(content, encoding="utf-8")
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        prompt_file=str(prompt_path),
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][-1] == content


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_member_create__backend_spawn_argv_shape(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    coding_agent,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("fleet={fleet_id}", encoding="utf-8")
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent=coding_agent,
        prompt_file=str(prompt_path),
        name=f"Member-{coding_agent}",
    )
    assert result.exit_code == 0, result.output
    command = split_window_recorder[0]["command"]
    assert command[-1] == f"fleet={fleet_id}"
    if coding_agent == "claude":
        assert command[0] == "claude"
        # Member display name threaded through as --name <name>.
        assert "--name" in command
        name_index = command.index("--name")
        assert command[name_index + 1] == f"Member-{coding_agent}"
    else:
        assert command == [
            "codex",
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write",
            f"fleet={fleet_id}",
        ]
        # Codex has no --name analog.
        assert "--name" not in command
        assert f"Member-{coding_agent}" not in command


def test_member_create__codex_placement_records_codex(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent="codex",
        inline_prompt="hello",
        name="Codex-Member",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["placement"]["coding_agent"] == "codex"


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_member_create__binary_missing_exits_with_backend_specific_message(
    bootstrapped_fleet,
    split_window_recorder,
    monkeypatch,
    coding_agent,
):
    monkeypatch.setattr("cafleet.coding_agent.base.shutil.which", lambda _: None)
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent=coding_agent,
        inline_prompt="hello",
        name=coding_agent.capitalize(),
    )
    assert result.exit_code == 1, result.output
    assert f"binary {coding_agent} not found on PATH" in (result.output or "")
    assert split_window_recorder == []


def test_member_create__model_claude_tokens_between_name_and_prompt(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        inline_prompt="hello",
        name="Drafter",
        model="sonnet",
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"] == [
        "claude",
        "--permission-mode",
        "dontAsk",
        "--name",
        "Drafter",
        "--model",
        "sonnet",
        "hello",
    ]


def test_member_create__model_codex_tokens_between_sandbox_and_prompt(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent="codex",
        inline_prompt="hello",
        name="Codex-Member",
        model="gpt-5.4-mini",
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"] == [
        "codex",
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
        "--model",
        "gpt-5.4-mini",
        "hello",
    ]


def test_member_create__model_opencode_tokens_before_prompt_pair(
    tmp_path,
    monkeypatch,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # OpencodeAgent.ensure_available materializes ~/.opencode/agents/cafleet.md;
    # redirect HOME so the write stays inside tmp_path.
    monkeypatch.setenv("HOME", str(tmp_path))
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent="opencode",
        inline_prompt="hello",
        name="OC-Member",
        model="anthropic/claude-sonnet-4-6",
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"] == [
        "opencode",
        "--agent",
        "cafleet",
        "--model",
        "anthropic/claude-sonnet-4-6",
        "--prompt",
        "hello",
    ]


@pytest.mark.parametrize("coding_agent", ["claude", "codex", "opencode"])
def test_member_create__no_model_flag_emits_no_model_token(
    tmp_path,
    monkeypatch,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    coding_agent,
):
    monkeypatch.setenv("HOME", str(tmp_path))
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent=coding_agent,
        inline_prompt="hello",
        name="Plain-Member",
    )
    assert result.exit_code == 0, result.output
    assert "--model" not in split_window_recorder[0]["command"]


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_member_create__claude_codex_empty_model_passes_through(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    coding_agent,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent=coding_agent,
        inline_prompt="hello",
        name="Empty-Model",
        model="",
    )
    assert result.exit_code == 0, result.output
    command = split_window_recorder[0]["command"]
    model_index = command.index("--model")
    assert command[model_index + 1] == ""


def test_member_create__opencode_invalid_model_exits_2_with_no_side_effects(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent="opencode",
        inline_prompt="hello",
        name="Rejected-Member",
        model="no-slash",
    )
    assert result.exit_code == 2, result.output
    assert (
        "Error: --model for the opencode backend must be "
        "'<provider-id>/<model-id>' (got 'no-slash')." in result.output
    )
    # Validation precedes registration and any tmux call: nothing to roll back.
    assert split_window_recorder == []
    names = [agent["name"] for agent in broker.list_agents(fleet_id)]
    assert "Rejected-Member" not in names


def test_member_create__opencode_invalid_model_wins_over_missing_binary(
    bootstrapped_fleet,
    split_window_recorder,
    monkeypatch,
):
    # validate_model runs before ensure_available: with the binary absent AND
    # the model malformed, the model error (exit 2) is the one reported.
    monkeypatch.setattr("cafleet.coding_agent.base.shutil.which", lambda _: None)
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        coding_agent="opencode",
        inline_prompt="hello",
        name="Rejected-Member",
        model="no-slash",
    )
    assert result.exit_code == 2, result.output
    assert "--model for the opencode backend must be" in result.output
    assert "not found on PATH" not in result.output
    assert split_window_recorder == []


def test_member_create__claude_default_injects_dontask_permission_mode(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        inline_prompt="hello",
        name="Drafter",
    )
    assert result.exit_code == 0, result.output
    command = split_window_recorder[0]["command"]
    assert "--permission-mode" in command
    perm_index = command.index("--permission-mode")
    assert command[perm_index + 1] == "dontAsk"
    assert command[0] == "claude"
    assert "--disallowedTools" not in command
    assert "Bash" not in command
    name_index = command.index("--name")
    assert perm_index < name_index
