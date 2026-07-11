"""Tests for ``cafleet member create`` and ``cafleet member nudge``.

Covers the unified ``--text`` / ``--text-file`` input pair (design 0000112 §3):
xor + exactly-one-required, inline placeholder substitution, the ``--text-file``
path / stdin / error matrix, spawn-argv shape per backend, permission-mode
injection, ``--role`` handling, and the ``member nudge`` body-input surface.

Body resolution + validation live in the shared ``read_text_input`` helper
(unit-tested in ``test_text_input.py``); ``member create`` additionally runs
``substitute_spawn_placeholders`` over the resolved body. These tests exercise
both through the real CLI.
"""

import json
import os
import sys

import pytest
from click.testing import CliRunner
from sqlalchemy import delete, update

from cafleet import broker
from cafleet.broker import _shared
from cafleet.cli import cli
from cafleet.db.models import Member, MemberPlacement
from cafleet.multiplexer import MultiplexerContext as DirectorContext
from cafleet.multiplexer import tmux as multiplexer_tmux


@pytest.fixture
def bootstrapped_fleet(_mock_tmux_for_fleet_create):
    runner = CliRunner()
    create = runner.invoke(cli, ["fleet", "create", "--name", "test-fleet", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    return data["fleet_id"], data["director"]["member_id"], runner


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
    *,
    coding_agent: str = "claude",
    text: str | None = None,
    text_file: str | None = None,
    positional: str | None = None,
    name: str = "Member",
    json_output: bool = False,
    model: str | None = None,
    role: str | None = None,
    stdin: bytes | None = None,
):
    args: list[str] = [
        "member",
        "create",
        "--fleet-id",
        str(fleet_id),
        "--name",
        name,
        "--description",
        f"{name} for tests",
    ]
    if coding_agent != "claude":
        args.extend(["--coding-agent", coding_agent])
    if model is not None:
        args.extend(["--model", model])
    if role is not None:
        args.extend(["--role", role])
    if text_file is not None:
        args.extend(["--text-file", text_file])
    if text is not None:
        args.extend(["--text", text])
    if json_output:
        args.append("--json")
    if positional is not None:
        args.extend(["--", positional])
    if stdin is not None:
        return runner.invoke(cli, args, input=stdin)
    return runner.invoke(cli, args)


# --- member create: unified --text / --text-file surface (§3) --------------


def test_member_create__neither_flag_is_usage_error(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # Removing the default template makes a bare `member create` (neither flag)
    # a usage error resolved by the shared helper — no spawn.
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(runner, fleet_id)
    assert result.exit_code == 2, result.output
    assert "Provide exactly one of --text or --text-file." in result.output
    assert split_window_recorder == []


def test_member_create__positional_prompt_no_longer_parses(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # `member create` takes no positional argument — a bare positional after
    # the options is a parse error.
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(runner, fleet_id, positional="hello positional")
    assert result.exit_code == 2, result.output
    assert "extra argument" in result.output.lower()
    assert split_window_recorder == []


def test_member_create__text_and_text_file_mutually_exclusive(
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
        text="hello inline",
        text_file=str(prompt_path),
    )
    assert result.exit_code == 2, result.output
    assert "--text and --text-file are mutually exclusive." in result.output
    assert split_window_recorder == []


def test_member_create__inline_text_substitutes_placeholders(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # --text is placeholder-substituted before it becomes the spawn prompt (§2).
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="member={member_id} director={director_member_id}",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["member_id"]
    assert (
        split_window_recorder[0]["command"][-1]
        == f"member={new_id} director={director_id}"
    )


def test_member_create__text_file_relative_path_accepted(
    tmp_path,
    monkeypatch,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # `--text-file` resolves a relative path against the CWD (design 0000112 §1).
    fleet_id, director_id, runner = bootstrapped_fleet
    (tmp_path / "prompt.md").write_text("relative body", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    result = _invoke_member_create(runner, fleet_id, text_file="prompt.md")
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][-1] == "relative body"


def test_member_create__text_file_dash_reads_stdin(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # `--text-file -` reads the spawn prompt from stdin, then substitutes.
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        text_file="-",
        stdin=b"spawn body from stdin",
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][-1] == "spawn body from stdin"


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
def test_member_create__text_file_error_variants(
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
    else:  # unknown_placeholder — read succeeds, substitution rejects it (exit 2)
        target = tmp_path / "prompt.md"
        target.write_text("hi {unknown}", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        fleet_id,
        text_file=str(target),
    )
    assert result.exit_code == expected_exit, result.output
    assert expected_substring in result.output
    assert split_window_recorder == []


@pytest.mark.skipif(
    sys.platform == "win32" or (hasattr(os, "geteuid") and os.geteuid() == 0),
    reason="POSIX-only test; root bypasses the read-permission check",
)
def test_member_create__text_file_not_readable_exits_with_message(
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
            text_file=str(unreadable_file),
        )
        assert result.exit_code == 1, result.output
        assert "file is not readable" in result.output
        assert str(unreadable_file) in result.output
        assert split_window_recorder == []
    finally:
        unreadable_file.chmod(0o644)


@pytest.mark.parametrize(
    ("scenario", "content"),
    [
        ("preserves_trailing_newline", "hello\n"),
        ("preserves_surrounding_whitespace", "   \n  hello world  \n   "),
    ],
)
def test_member_create__text_file_preserves_whitespace_verbatim(
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
        text_file=str(prompt_path),
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
        coding_agent=coding_agent,
        text_file=str(prompt_path),
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
        coding_agent="codex",
        text="hello",
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
        coding_agent=coding_agent,
        text="hello",
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
        text="hello",
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
        coding_agent="codex",
        text="hello",
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
        coding_agent="opencode",
        text="hello",
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
        coding_agent=coding_agent,
        text="hello",
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
        coding_agent=coding_agent,
        text="hello",
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
        coding_agent="opencode",
        text="hello",
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
    names = [member["name"] for member in broker.list_roster(fleet_id)]
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
        coding_agent="opencode",
        text="hello",
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
        text="hello",
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


def _read_member_card(new_member_id: int) -> dict:
    with _shared.read_session() as s:
        return json.loads(s.get(Member, new_member_id).member_card_json)


def test_member_create__role_monitor_sets_kind_not_enrolled(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="hello",
        name="Watcher",
        role="monitor",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["member_id"]

    # the kind marker is written into the member card …
    assert _read_member_card(new_id)["cafleet"]["kind"] == "monitoring-member"
    # … but the monitoring member is the unenrolled watcher (no monitor_config row)
    assert broker.get_monitor_config(fleet_id, new_id) is None


def test_member_create__role_member_enrolls_720(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="hello",
        name="Ordinary",
        role="member",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["member_id"]

    # an ordinary member carries no kind marker and is enrolled @720
    assert "cafleet" not in _read_member_card(new_id)
    cfg = broker.get_monitor_config(fleet_id, new_id)
    assert cfg is not None
    assert cfg["interval_seconds"] == 720


def test_member_create__default_role_is_member_enrolled_720(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # omitting --role defaults to 'member': no kind marker, enrolled @720
    fleet_id, director_id, runner = bootstrapped_fleet
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="hello",
        name="Plain",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    new_id = json.loads(result.output)["member_id"]
    assert "cafleet" not in _read_member_card(new_id)
    cfg = broker.get_monitor_config(fleet_id, new_id)
    assert cfg is not None
    assert cfg["interval_seconds"] == 720


def test_member_create__second_role_monitor_rejected(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    first = _invoke_member_create(
        runner,
        fleet_id,
        text="hi",
        name="Watcher-1",
        role="monitor",
    )
    assert first.exit_code == 0, first.output

    # only one monitoring member per fleet — the second --role monitor is rejected
    second = _invoke_member_create(
        runner,
        fleet_id,
        text="hi",
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
        text="hi",
        name="Bad-Role",
        role="director",
    )
    assert result.exit_code == 2, result.output
    assert "not one of" in result.output


# --- member nudge (design 0000092 §4, B5; 0000112 §3 text-input) -----------


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
    text: str | None = None,
    *,
    text_file: str | None = None,
    json_output: bool = False,
    stdin: bytes | None = None,
):
    args: list[str] = [
        "member",
        "nudge",
        "--fleet-id",
        str(fleet_id),
        "--from-member-id",
        str(sender_id),
        "--to-member-id",
        str(target_id),
    ]
    if text_file is not None:
        args.extend(["--text-file", text_file])
    if text is not None:
        args.extend(["--text", text])
    if json_output:
        args.append("--json")
    if stdin is not None:
        return runner.invoke(cli, args, input=stdin)
    return runner.invoke(cli, args)


def _create_member(runner: CliRunner, fleet_id: int, name: str) -> int:
    """Create an ordinary member via the CLI; return its member id (pane %42)."""
    result = _invoke_member_create(
        runner, fleet_id, text="hi", name=name, json_output=True
    )
    assert result.exit_code == 0, result.output
    return json.loads(result.output)["member_id"]


def test_member_nudge__persists_unicast_task_and_fires_preview(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")

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
    assert payload["member_id"] == director_id
    assert payload["notification_sent"] is True
    task_id = payload["task_id"]

    # The persisted task is a unicast / input_required from sender → target.
    task = broker.get_task(fleet_id, task_id)["task"]
    assert task["type"] == "unicast"
    assert task["status_state"] == "input_required"
    assert task["from_member_id"] == sender_id
    assert task["to_member_id"] == director_id
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
    sender_id = _create_member(runner, fleet_id, "Watcher")
    target_id = _create_member(runner, fleet_id, "Target")

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


def test_member_nudge__text_file_body_reaches_target(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    # member nudge gains the same --text-file input as the other commands; the
    # file body is delivered verbatim (no placeholder substitution).
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")
    body_file = tmp_path / "nudge.md"
    body_file.write_text("long re-engage body {member_id}", encoding="utf-8")

    result = _invoke_member_nudge(
        runner,
        fleet_id,
        sender_id,
        director_id,
        text_file=str(body_file),
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    task_id = json.loads(result.output)["task_id"]
    task = broker.get_task(fleet_id, task_id)["task"]
    # Verbatim — the {member_id} token is NOT substituted for nudge bodies.
    assert task["text"] == "long re-engage body {member_id}"


def test_member_nudge__stdin_body_reaches_target(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")

    result = _invoke_member_nudge(
        runner,
        fleet_id,
        sender_id,
        director_id,
        text_file="-",
        json_output=True,
        stdin=b"piped nudge body",
    )
    assert result.exit_code == 0, result.output
    task_id = json.loads(result.output)["task_id"]
    assert broker.get_task(fleet_id, task_id)["task"]["text"] == "piped nudge body"


@pytest.mark.parametrize("scenario", ["unknown", "inactive", "cross_fleet"])
def test_member_nudge__target_not_found_exits_1(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
    scenario,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")

    if scenario == "unknown":
        target_id = 999999
    elif scenario == "inactive":
        target_id = _create_member(runner, fleet_id, "Gone")
        broker.deregister_member(target_id)
    else:  # cross_fleet
        other = broker.create_fleet(
            name=None,
            director_context=DirectorContext(
                session="main", window_id="@3", pane_id="%0"
            ),
            coding_agent="claude",
            backend="tmux",
        )
        target_id = broker.register_member(
            fleet_id=other["fleet_id"], name="outsider", description="cross-fleet"
        )["member_id"]

    result = _invoke_member_nudge(runner, fleet_id, sender_id, target_id, "hello")
    assert result.exit_code == 1, result.output
    assert f"Member {target_id} not found" in result.output
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
    sender_id = _create_member(runner, fleet_id, "Watcher")
    result = _invoke_member_nudge(runner, fleet_id, sender_id, director_id, text)
    assert result.exit_code == 2, result.output
    assert "text may not be empty." in result.output
    assert inline_preview_recorder == []


def test_member_nudge__empty_text_file_rejected_exit_1(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")
    empty_file = tmp_path / "empty.md"
    empty_file.write_bytes(b"")
    result = _invoke_member_nudge(
        runner, fleet_id, sender_id, director_id, text_file=str(empty_file)
    )
    assert result.exit_code == 1, result.output
    assert f"--text-file {empty_file}: file is empty." in result.output
    assert inline_preview_recorder == []


def test_member_nudge__neither_flag_rejected_exit_2(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")
    result = _invoke_member_nudge(runner, fleet_id, sender_id, director_id)
    assert result.exit_code == 2, result.output
    assert "Provide exactly one of --text or --text-file." in result.output
    assert inline_preview_recorder == []


def test_member_nudge__both_flags_rejected_exit_2(
    tmp_path,
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")
    body_file = tmp_path / "nudge.md"
    body_file.write_text("file body", encoding="utf-8")
    result = _invoke_member_nudge(
        runner,
        fleet_id,
        sender_id,
        director_id,
        "inline body",
        text_file=str(body_file),
    )
    assert result.exit_code == 2, result.output
    assert "--text and --text-file are mutually exclusive." in result.output
    assert inline_preview_recorder == []


def test_member_nudge__no_pane_target_still_queues_task(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    inline_preview_recorder,
):
    fleet_id, director_id, runner = bootstrapped_fleet
    sender_id = _create_member(runner, fleet_id, "Watcher")
    # A target with a placement row but no pane yet (mux_pane_id=None).
    no_pane_id = broker.register_member(
        fleet_id=fleet_id,
        name="PendingPane",
        description="placement but no pane",
        placement={
            "backend": "tmux",
            "mux_session": "main",
            "mux_window_id": "@3",
            "mux_pane_id": None,
            "coding_agent": "claude",
        },
    )["member_id"]

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
    sender_id = _create_member(runner, fleet_id, "Watcher")

    # Happy path (target has pane %0): the dispatched-preview line.
    ok = _invoke_member_nudge(runner, fleet_id, sender_id, director_id, "wake up")
    assert ok.exit_code == 0, ok.output
    assert "Nudged" in ok.output
    assert "queued" in ok.output

    # No-pane target: the queued-without-pane variant.
    no_pane_id = broker.register_member(
        fleet_id=fleet_id,
        name="PendingPane",
        description="placement but no pane",
        placement={
            "backend": "tmux",
            "mux_session": "main",
            "mux_window_id": "@3",
            "mux_pane_id": None,
            "coding_agent": "claude",
        },
    )["member_id"]
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
    sender_id = _create_member(runner, fleet_id, "Watcher")

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
    `member` subcommand does: `ensure_tmux_or_die` raising
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


# --- monitor inherits the Director's coding agent (design 0000101) ----------


@pytest.fixture
def make_bootstrapped_fleet(tmp_path, monkeypatch, _mock_tmux_for_fleet_create):
    """Bootstrap a fleet whose root Director runs on a chosen backend.

    Returns a factory ``make(coding_agent="claude") -> (fleet_id, director_id,
    runner)`` so monitor-inheritance tests can stand up a non-claude Director
    whose placement row records ``codex`` / ``opencode``.
    """
    # An inherited-opencode spawn's ensure_available materializes
    # ~/.opencode/agents/cafleet.md; redirect HOME so it stays in tmp_path.
    monkeypatch.setenv("HOME", str(tmp_path))

    def _make(coding_agent: str = "claude"):
        runner = CliRunner()
        args = ["fleet", "create", "--name", "test-fleet", "--json"]
        if coding_agent != "claude":
            args += ["--coding-agent", coding_agent]
        create = runner.invoke(cli, args)
        assert create.exit_code == 0, create.output
        data = json.loads(create.output)
        return data["fleet_id"], data["director"]["member_id"], runner

    return _make


@pytest.mark.parametrize("backend", ["codex", "opencode"])
def test_member_create__role_monitor_inherits_director_backend(
    make_bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    backend,
):
    # On a non-claude Director, --role monitor with --coding-agent OMITTED makes
    # the spawned binary, the monitor's placement, and the rendered prompt's
    # CODING AGENT line all equal the Director's backend.
    fleet_id, director_id, runner = make_bootstrapped_fleet(backend)
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="CODING AGENT: {coding_agent}",
        name="Watcher",
        role="monitor",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    command = split_window_recorder[0]["command"]
    assert command[0] == backend
    assert command[-1] == f"CODING AGENT: {backend}"
    assert json.loads(result.output)["placement"]["coding_agent"] == backend


def test_member_create__role_monitor_explicit_coding_agent_wins(
    make_bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # An explicit --coding-agent claude beats the codex Director's backend.
    # Build the argv directly: _invoke_member_create omits --coding-agent when
    # its value is claude (the omit-for-claude default the other tests rely on),
    # so proving "explicit claude wins" requires actually sending the flag.
    fleet_id, director_id, runner = make_bootstrapped_fleet("codex")
    result = runner.invoke(
        cli,
        [
            "member",
            "create",
            "--fleet-id",
            str(fleet_id),
            "--name",
            "Watcher",
            "--description",
            "Watcher for tests",
            "--role",
            "monitor",
            "--coding-agent",
            "claude",
            "--text",
            "hello",
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][0] == "claude"
    assert json.loads(result.output)["placement"]["coding_agent"] == "claude"


def test_member_create__role_member_omitted_flag_stays_claude(
    make_bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # Scope guard: an ordinary member on a codex Director still defaults to
    # claude when --coding-agent is omitted — inheritance is monitor-only.
    fleet_id, director_id, runner = make_bootstrapped_fleet("codex")
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="hello",
        name="Ordinary",
        role="member",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][0] == "claude"
    assert json.loads(result.output)["placement"]["coding_agent"] == "claude"


def test_member_create__role_monitor_fail_loud_missing_placement(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # A Director whose placement row is gone (corruption) is unresolvable for
    # monitor backend inheritance: exit 1 with the "has no placement row"
    # message and no spawn.
    fleet_id, director_id, runner = bootstrapped_fleet
    with _shared.write_session() as s:
        s.execute(
            delete(MemberPlacement).where(MemberPlacement.member_id == director_id)
        )
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="hello",
        name="Watcher",
        role="monitor",
    )
    assert result.exit_code == 1, result.output
    assert "has no placement row" in result.output
    assert split_window_recorder == []


def test_member_create__role_monitor_fail_loud_director_not_found(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    # An auto-resolved Director that is no longer active (corruption) exercises
    # the director-is-None branch: exit 1 with the "not found in fleet" message
    # and no spawn.
    fleet_id, director_id, runner = bootstrapped_fleet
    with _shared.write_session() as s:
        s.execute(
            update(Member)
            .where(Member.member_id == director_id)
            .values(status="deregistered")
        )
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="hello",
        name="Watcher",
        role="monitor",
    )
    assert result.exit_code == 1, result.output
    assert "not found in fleet" in result.output
    assert split_window_recorder == []
