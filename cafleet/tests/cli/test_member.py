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
from cafleet.broker import _shared
from cafleet.cli import cli
from cafleet.cli._prompt import resolve_prompt
from cafleet.db.models import Agent
from cafleet.multiplexer import MultiplexerContext as DirectorContext
from cafleet.multiplexer import tmux as multiplexer_tmux


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
    role: str | None = None,
):
    args: list[str] = []
    if json_output:
        args.append("--json")
    args.extend(
        [
            "member",
            "create",
            "--fleet-id",
            str(fleet_id),
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
    if role is not None:
        args.extend(["--role", role])
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


# --- member create --role (design 0000090 §5) ------------------------------


def _read_agent_card(new_agent_id: int) -> dict:
    with _shared.read_session() as s:
        return json.loads(s.get(Agent, new_agent_id).agent_card_json)


def test_member_create__role_monitor_sets_kind_and_enrolls(
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
        name="Watcher",
        role="monitor",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["agent_id"]

    # the kind marker is written into the agent card …
    assert _read_agent_card(new_id)["cafleet"]["kind"] == "monitoring-member"
    # … and the monitoring member is enrolled in monitor_config
    assert broker.get_monitor_config(fleet_id, new_id) is not None


def test_member_create__role_member_does_not_enroll(
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
        name="Ordinary",
        role="member",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["agent_id"]

    # an ordinary member carries no kind marker and is NOT enrolled
    assert "cafleet" not in _read_agent_card(new_id)
    assert broker.get_monitor_config(fleet_id, new_id) is None


def test_member_create__default_role_is_member_not_enrolled(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # omitting --role defaults to 'member': no kind marker, no enrollment
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        inline_prompt="hello",
        name="Plain",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["agent_id"]
    assert "cafleet" not in _read_agent_card(new_id)
    assert broker.get_monitor_config(fleet_id, new_id) is None


def test_member_create__second_role_monitor_rejected(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    first = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        inline_prompt="hi",
        name="Watcher-1",
        role="monitor",
    )
    assert first.exit_code == 0, first.output

    # only one monitoring member per fleet — the second --role monitor is rejected
    second = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        inline_prompt="hi",
        name="Watcher-2",
        role="monitor",
    )
    assert second.exit_code == 1, second.output
    assert "monitoring member" in second.output


def test_member_create__invalid_role_choice_rejected(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # --role is a Choice(['member', 'monitor']); any other value is a usage error
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        director_id,
        inline_prompt="hi",
        name="Bad-Role",
        role="director",
    )
    assert result.exit_code == 2, result.output
    assert "not one of" in result.output


# --- member nudge (design 0000092 §4, B5) ----------------------------------


@pytest.fixture
def inline_preview_recorder(monkeypatch):
    """Capture ``send_inline_preview`` calls and report success, so a nudge's
    best-effort notification fires deterministically without a real tmux pane."""
    calls: list[dict] = []

    def stub(self, **kwargs):
        calls.append(kwargs)
        return True

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_inline_preview", stub
    )
    return calls


def _invoke_member_nudge(
    runner: CliRunner,
    fleet_id: int,
    sender_id: int,
    target_id: int,
    text: str,
    *,
    json_output: bool = False,
):
    args: list[str] = []
    if json_output:
        args.append("--json")
    args.extend(
        [
            "member",
            "nudge",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(sender_id),
            "--member-id",
            str(target_id),
            "--text",
            text,
        ]
    )
    return runner.invoke(cli, args)


def _create_member(
    runner: CliRunner, fleet_id: int, director_id: int, name: str
) -> int:
    """Create an ordinary member via the CLI; return its agent id (pane %42)."""
    result = _invoke_member_create(
        runner, fleet_id, director_id, inline_prompt="hi", name=name, json_output=True
    )
    assert result.exit_code == 0, result.output
    return json.loads(result.output)["agent_id"]


def test_member_nudge__persists_unicast_task_and_fires_preview(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")

    # The monitoring member nudges the Director (a valid target with pane %0).
    result = _invoke_member_nudge(
        runner,
        fleet_id,
        sender_id,
        director_id,
        "2 un-acked items; alice stalled",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["member_agent_id"] == director_id
    assert payload["notification_sent"] is True
    task_id = payload["task_id"]

    # The persisted task is a unicast / input_required from sender → target.
    task = broker.get_task(fleet_id, task_id)["task"]
    assert task["type"] == "unicast"
    assert task["status_state"] == "input_required"
    assert task["from_agent_id"] == sender_id
    assert task["to_agent_id"] == director_id
    assert task["text"] == "2 un-acked items; alice stalled"

    # The hardened inline preview fired exactly once into the target's pane.
    assert len(inline_preview_recorder) == 1
    call = inline_preview_recorder[0]
    assert call["target_pane_id"] == "%0"
    assert call["task_id"] == task_id
    assert call["sender_id"] == sender_id
    assert call["text"] == "2 un-acked items; alice stalled"


def test_member_nudge__persisted_task_is_ackable_by_recipient(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")
    target_id = _create_member(runner, fleet_id, director_id, "Target")

    result = _invoke_member_nudge(
        runner, fleet_id, sender_id, target_id, "re-engage please", json_output=True
    )
    assert result.exit_code == 0, result.output
    task_id = json.loads(result.output)["task_id"]

    # The recipient sees the task in poll and can ack it.
    [polled] = broker.poll_tasks(target_id)
    assert polled["task_id"] == task_id
    assert polled["text"] == "re-engage please"
    acked = broker.ack_task(target_id, task_id)
    assert acked["task"]["status_state"] == "completed"
    # Once acked it no longer appears in poll.
    assert broker.poll_tasks(target_id) == []


@pytest.mark.parametrize("scenario", ["unknown", "inactive", "cross_fleet"])
def test_member_nudge__target_not_found_exits_1(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
    scenario,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")

    if scenario == "unknown":
        target_id = 999999
    elif scenario == "inactive":
        target_id = _create_member(runner, fleet_id, director_id, "Gone")
        broker.deregister_agent(target_id)
    else:  # cross_fleet
        other = broker.create_fleet(
            label=None,
            director_context=DirectorContext(
                session="main", window_id="@3", pane_id="%0"
            ),
            coding_agent="claude",
        )
        target_id = broker.register_agent(
            fleet_id=other["fleet_id"], name="outsider", description="cross-fleet"
        )["agent_id"]

    result = _invoke_member_nudge(runner, fleet_id, sender_id, target_id, "hello")
    assert result.exit_code == 1, result.output
    assert f"Agent {target_id} not found" in result.output
    # No preview fired — resolution rejects the target before the send path.
    assert inline_preview_recorder == []


@pytest.mark.parametrize("text", ["", "   ", "\t"])
def test_member_nudge__empty_text_rejected_exit_2(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
    text,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")
    result = _invoke_member_nudge(runner, fleet_id, sender_id, director_id, text)
    assert result.exit_code == 2, result.output
    assert "text may not be empty." in result.output
    assert inline_preview_recorder == []


def test_member_nudge__no_pane_target_still_queues_task(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")
    # A target with a placement row but no pane yet (tmux_pane_id=None).
    no_pane_id = broker.register_agent(
        fleet_id=fleet_id,
        name="PendingPane",
        description="placement but no pane",
        placement={
            "director_agent_id": director_id,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": None,
            "coding_agent": "claude",
        },
    )["agent_id"]

    result = _invoke_member_nudge(
        runner, fleet_id, sender_id, no_pane_id, "queued anyway", json_output=True
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    # The preview is a best-effort no-op (no live pane) but the task persists.
    assert payload["notification_sent"] is False
    task_id = payload["task_id"]

    [polled] = broker.poll_tasks(no_pane_id)
    assert polled["task_id"] == task_id
    assert polled["text"] == "queued anyway"
    assert inline_preview_recorder == []


def test_member_nudge__text_output_happy_and_no_pane_variants(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")

    # Happy path (target has pane %0): the dispatched-preview line.
    ok = _invoke_member_nudge(runner, fleet_id, sender_id, director_id, "wake up")
    assert ok.exit_code == 0, ok.output
    assert "Nudged" in ok.output
    assert "queued" in ok.output

    # No-pane target: the queued-without-pane variant.
    no_pane_id = broker.register_agent(
        fleet_id=fleet_id,
        name="PendingPane",
        description="placement but no pane",
        placement={
            "director_agent_id": director_id,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": None,
            "coding_agent": "claude",
        },
    )["agent_id"]
    no_pane = _invoke_member_nudge(
        runner, fleet_id, sender_id, no_pane_id, "still queued"
    )
    assert no_pane.exit_code == 0, no_pane.output
    assert "no pane" in no_pane.output
    assert "queued" in no_pane.output


def test_member_nudge__real_preview_keystroke_is_esc_first(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    monkeypatch,
):
    """End-to-end (design 0000092 §1/§4): the other C2 tests stub
    ``send_inline_preview``, so none prove that ``member nudge`` actually emits
    an Esc-first keystroke through the REAL helper. Drive the CLI with the real
    helper in play (capture ``_run`` argv, no method-level stub) and assert the
    target Director's pane receives `Escape` FIRST — so a Director parked on a
    pending permission-approval prompt has it dismissed before any payload
    character is typed and the trailing Enter can never confirm it. Mirrors
    tests/broker/test_inline_preview.py::test_send_message__real_inline_preview_keystroke_is_esc_first."""
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, director_id, "Watcher")

    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    captured: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )

    result = _invoke_member_nudge(
        runner, fleet_id, sender_id, director_id, "cancel it (Esc)", json_output=True
    )
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["notification_sent"] is True

    # The Director's pane (%0) receives Escape FIRST, then the literal payload,
    # then the trailing Enter submit — the hardened (esc_first) inline preview.
    pane_calls = [argv for argv in captured if argv[3:4] == ["%0"]]
    assert pane_calls[0] == ["tmux", "send-keys", "-t", "%0", "Escape"]
    assert pane_calls[1][:5] == ["tmux", "send-keys", "-t", "%0", "-l"]
    assert pane_calls[-1] == ["tmux", "send-keys", "-t", "%0", "Enter"]


def test_member_nudge__tmux_unavailable_exits_one(bootstrapped_fleet, monkeypatch):
    """`member nudge` enforces the same outside-a-tmux-session guard every other
    `member` subcommand does (Copilot round-2): `ensure_tmux_or_die` raising
    exits 1 with the member-subgroup tmux error. Mirrors the per-file
    `test_tmux_unavailable__tmux_not_available_exits_one` in test_member_ping.py
    / test_member_exec.py."""
    fleet_id, director_id, runner = bootstrapped_fleet

    def raise_unavailable(self):
        raise multiplexer_tmux.TmuxError(
            "cafleet member commands must be run inside a tmux session"
        )

    monkeypatch.setattr(
        multiplexer_tmux.TmuxMultiplexer, "ensure_available", raise_unavailable
    )
    result = _invoke_member_nudge(
        runner, fleet_id, director_id, director_id, "re-engage"
    )
    assert result.exit_code == 1, result.output
    assert "cafleet member commands must be run inside a tmux session" in (
        result.output or ""
    )
