"""Tests for ``_resolve_prompt`` and ``cafleet member create``.

Covers placeholder substitution (default + custom + doubled brace + error
branches), `--prompt-file` validation matrix, spawn-argv shape per backend,
permission-mode injection, and binary-missing exit messages.
"""

import json
import os
import sys
import uuid

import click
import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import _resolve_prompt, cli
from cafleet.tmux import DirectorContext


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def director_agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def new_agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def ctx(session_id):
    command = click.Command("member-create")
    context = click.Context(command)
    context.obj = {"session_id": session_id, "json_output": False}
    return context


@pytest.fixture
def mock_get_agent(monkeypatch):
    def fake_get_agent(agent_id, session_id):
        return {"agent_id": agent_id, "name": "Director-X"}

    monkeypatch.setattr(broker, "get_agent", fake_get_agent)
    return fake_get_agent


_CLI_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def bootstrapped_session(tmp_path, monkeypatch, _reset_engine_singletons):
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _CLI_FAKE_DIRECTOR_CTX)

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    create = runner.invoke(cli, ["session", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    return data["session_id"], data["director"]["agent_id"], runner


@pytest.fixture
def split_window_recorder(monkeypatch):
    calls: list[dict] = []

    def fake_split_window(**kwargs):
        calls.append(kwargs)
        return "%42"

    monkeypatch.setattr("cafleet.tmux.split_window", fake_split_window)
    monkeypatch.setattr("cafleet.tmux.select_layout", lambda **_: None)
    monkeypatch.setattr("cafleet.tmux.send_exit", lambda **_: None, raising=False)
    return calls


@pytest.fixture
def stub_coding_agent_binaries(monkeypatch):
    monkeypatch.setattr("cafleet.cli.shutil.which", lambda _: "/usr/bin/stub")


def _invoke_member_create(
    runner: CliRunner,
    session_id: str,
    director_id: str,
    *,
    coding_agent: str = "claude",
    prompt_file: str | None = None,
    inline_prompt: str | None = None,
    name: str = "Member",
    json_output: bool = False,
):
    args = ["--session-id", session_id]
    if json_output:
        args.append("--json")
    args.extend([
        "member",
        "create",
        "--agent-id",
        director_id,
        "--name",
        name,
        "--description",
        f"{name} for tests",
    ])
    if coding_agent != "claude":
        args.extend(["--coding-agent", coding_agent])
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
        ("custom_path_no_placeholders_passthrough", ("no", "placeholders", "here"), "passthrough"),
        (
            "doubled_brace_collapses_to_single",
            ("data", "is", "{{not", "a", "placeholder}}", "closed"),
            "doubled_brace",
        ),
    ],
)
def test_resolve_prompt__substitution_matrix(
    ctx, director_agent_id, new_agent_id, session_id, mock_get_agent,
    scenario, prompt_argv, asserts,
):
    result = _resolve_prompt(
        ctx,
        director_agent_id=director_agent_id,
        new_agent_id=new_agent_id,
        prompt_argv=prompt_argv,
    )
    if asserts == "default":
        assert session_id in result
        assert new_agent_id in result
        assert director_agent_id in result
        for raw in ("{session_id}", "{agent_id}", "{director_agent_id}"):
            assert raw not in result
    elif asserts == "agent_id_only":
        assert result == f"message for {new_agent_id}"
    elif asserts == "passthrough":
        assert result == "no placeholders here"
    else:
        assert result == "data is {not a placeholder} closed"
        assert new_agent_id not in result
        assert director_agent_id not in result


@pytest.mark.parametrize(
    ("scenario", "prompt_argv", "expect_message_contains"),
    [
        ("unknown_placeholder", ("hello", "{foo}"), ("foo", "{session_id}", "{agent_id}")),
        ("unmatched_brace", ("hello", "{unclosed"), ("{{", "}}")),
        ("attribute_access", ("hello", "{agent_id.foo}"), ("{{", "}}")),
    ],
)
def test_resolve_prompt__malformed_raises_usage_error(
    ctx, director_agent_id, new_agent_id, mock_get_agent,
    scenario, prompt_argv, expect_message_contains,
):
    with pytest.raises(click.UsageError) as exc_info:
        _resolve_prompt(
            ctx,
            director_agent_id=director_agent_id,
            new_agent_id=new_agent_id,
            prompt_argv=prompt_argv,
        )
    message = str(exc_info.value)
    for needle in expect_message_contains:
        assert needle in message


def test_prompt_file__relative_path_rejected(
    bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    result = _invoke_member_create(
        runner, session_id, director_id, prompt_file="./foo.md",
    )
    assert result.exit_code == 2, result.output
    assert "--prompt-file requires an absolute path" in result.output
    assert "./foo.md" in result.output
    assert split_window_recorder == []


@pytest.mark.parametrize(
    ("scenario", "fixture_setup", "expected_exit", "expected_substring"),
    [
        ("not_found", "missing", 1, "file does not exist or is not a regular file"),
        ("directory_not_regular", "directory", 1, "file does not exist or is not a regular file"),
        ("empty_zero_bytes", "empty", 1, "file is empty"),
        ("empty_whitespace_only", "whitespace", 1, "file is empty"),
        ("invalid_utf8", "bad_utf8", 1, "file is not valid UTF-8"),
        ("unknown_placeholder", "unknown_placeholder", 2, "Unknown placeholder"),
    ],
)
def test_prompt_file__error_variants(
    tmp_path, bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
    scenario, fixture_setup, expected_exit, expected_substring,
):
    session_id, director_id, runner = bootstrapped_session
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
        runner, session_id, director_id, prompt_file=str(target),
    )
    assert result.exit_code == expected_exit, result.output
    assert expected_substring in result.output
    assert split_window_recorder == []


@pytest.mark.skipif(
    sys.platform == "win32" or (hasattr(os, "geteuid") and os.geteuid() == 0),
    reason="POSIX-only test; root bypasses the read-permission check",
)
def test_prompt_file__not_readable_exits_with_message(
    tmp_path, bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    unreadable_file = tmp_path / "unreadable.md"
    unreadable_file.write_text("hello", encoding="utf-8")
    unreadable_file.chmod(0o000)
    try:
        result = _invoke_member_create(
            runner, session_id, director_id, prompt_file=str(unreadable_file),
        )
        assert result.exit_code == 1, result.output
        assert "file is not readable" in result.output
        assert str(unreadable_file) in result.output
        assert split_window_recorder == []
    finally:
        unreadable_file.chmod(0o644)


def test_prompt_file__mutually_exclusive_with_positional(
    tmp_path, bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("hello", encoding="utf-8")
    result = _invoke_member_create(
        runner, session_id, director_id,
        prompt_file=str(prompt_path), inline_prompt="hello positional",
    )
    assert result.exit_code == 2, result.output
    assert (
        "--prompt-file and the positional prompt argument are mutually exclusive"
        in result.output
    )
    assert split_window_recorder == []


def test_prompt_file__parity_with_positional_form(tmp_path):
    session_id = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    new_agent_id = str(uuid.uuid4())
    template = "hello {agent_id} from director {director_agent_id}"

    command = click.Command("member-create")
    ctx = click.Context(command)
    ctx.obj = {"session_id": session_id, "json_output": False}

    inline_result = _resolve_prompt(
        ctx, director_agent_id=director_id, new_agent_id=new_agent_id,
        prompt_argv=tuple(template.split(" ")), prompt_file=None,
    )
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text(template, encoding="utf-8")
    file_result = _resolve_prompt(
        ctx, director_agent_id=director_id, new_agent_id=new_agent_id,
        prompt_argv=(), prompt_file=str(prompt_path),
    )

    assert inline_result == file_result
    assert new_agent_id in file_result
    assert director_id in file_result


@pytest.mark.parametrize(
    ("scenario", "content"),
    [
        ("preserves_trailing_newline", "hello\n"),
        ("preserves_surrounding_whitespace", "   \n  hello world  \n   "),
    ],
)
def test_prompt_file__preserves_whitespace_verbatim(
    tmp_path, bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
    scenario, content,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text(content, encoding="utf-8")
    result = _invoke_member_create(
        runner, session_id, director_id, prompt_file=str(prompt_path),
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][-1] == content


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_member_create__backend_spawn_argv_shape(
    tmp_path, bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
    coding_agent,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text(f"session={{session_id}}", encoding="utf-8")
    result = _invoke_member_create(
        runner, session_id, director_id, coding_agent=coding_agent,
        prompt_file=str(prompt_path), name=f"Member-{coding_agent}",
    )
    assert result.exit_code == 0, result.output
    command = split_window_recorder[0]["command"]
    assert command[-1] == f"session={session_id}"
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
            f"session={session_id}",
        ]
        # Codex has no --name analog.
        assert "--name" not in command
        assert f"Member-{coding_agent}" not in command


def test_member_create__codex_placement_records_codex(
    bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    result = _invoke_member_create(
        runner, session_id, director_id,
        coding_agent="codex", inline_prompt="hello",
        name="Codex-Member", json_output=True,
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["placement"]["coding_agent"] == "codex"


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_member_create__binary_missing_exits_with_backend_specific_message(
    bootstrapped_session, split_window_recorder, monkeypatch, coding_agent,
):
    monkeypatch.setattr("cafleet.cli.shutil.which", lambda _: None)
    session_id, director_id, runner = bootstrapped_session
    result = _invoke_member_create(
        runner, session_id, director_id,
        coding_agent=coding_agent, inline_prompt="hello", name=coding_agent.capitalize(),
    )
    assert result.exit_code == 1, result.output
    assert f"binary {coding_agent} not found on PATH" in (result.output or "")
    assert split_window_recorder == []


def test_member_create__claude_default_injects_dontask_permission_mode(
    bootstrapped_session, split_window_recorder, stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    result = _invoke_member_create(
        runner, session_id, director_id, inline_prompt="hello", name="Drafter",
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
