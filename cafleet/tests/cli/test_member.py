"""Tests for ``cafleet member create``.

Covers the unified ``--text`` / ``--text-file`` input pair:
xor + exactly-one-required, inline placeholder substitution, the ``--text-file``
path / stdin / error matrix, spawn-argv shape per backend, permission-mode
injection, and ``--role`` handling.

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


@pytest.fixture
def bootstrapped_fleet(_mock_tmux_for_fleet_create):
    runner = CliRunner()
    create = runner.invoke(
        cli,
        [
            "fleet",
            "create",
            "--name",
            "test-fleet",
            "--coding-agent",
            "claude",
            "--json",
        ],
    )
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
    # `--text-file` resolves a relative path against the CWD.
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


# --- member create --role ------------------------------


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


# --- an omitted --coding-agent inherits the Director's backend ----------


@pytest.fixture
def make_bootstrapped_fleet(tmp_path, monkeypatch, _mock_tmux_for_fleet_create):
    """Bootstrap a fleet whose root Director runs on a chosen backend.

    Returns a factory ``make(coding_agent="claude") -> (fleet_id, director_id,
    runner)`` so inheritance tests can stand up a non-claude Director whose
    placement row records ``codex`` / ``opencode``.
    """
    # An inherited-opencode spawn's ensure_available materializes
    # ~/.opencode/agents/cafleet.md; redirect HOME so it stays in tmp_path.
    monkeypatch.setenv("HOME", str(tmp_path))

    def _make(coding_agent: str = "claude"):
        runner = CliRunner()
        create = runner.invoke(
            cli,
            [
                "fleet",
                "create",
                "--name",
                "test-fleet",
                "--coding-agent",
                coding_agent,
                "--json",
            ],
        )
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


@pytest.mark.parametrize("backend", ["codex", "opencode"])
def test_member_create__role_member_omitted_flag_inherits_director_backend(
    make_bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    backend,
):
    # An ordinary member with --coding-agent OMITTED inherits the Director's
    # backend: the spawned binary, the placement, and the rendered prompt's
    # CODING AGENT line all equal the Director's backend.
    fleet_id, director_id, runner = make_bootstrapped_fleet(backend)
    result = _invoke_member_create(
        runner,
        fleet_id,
        text="CODING AGENT: {coding_agent}",
        name="Ordinary",
        role="member",
        json_output=True,
    )
    assert result.exit_code == 0, result.output
    command = split_window_recorder[0]["command"]
    assert command[0] == backend
    assert command[-1] == f"CODING AGENT: {backend}"
    assert json.loads(result.output)["placement"]["coding_agent"] == backend


@pytest.mark.parametrize("role", ["member", "monitor"])
def test_member_create__omitted_flag_fail_loud_missing_placement(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    role,
):
    # A Director whose placement row is gone (corruption) is unresolvable for
    # backend inheritance — for every role: exit 1 with the "has no placement
    # row" message and no spawn.
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
        role=role,
    )
    assert result.exit_code == 1, result.output
    assert "cannot resolve the member's coding agent" in result.output
    assert "has no placement row" in result.output
    assert "Re-run with an explicit --coding-agent." in result.output
    assert split_window_recorder == []


@pytest.mark.parametrize("role", ["member", "monitor"])
def test_member_create__omitted_flag_fail_loud_director_not_found(
    bootstrapped_fleet,
    split_window_recorder,
    stub_coding_agent_binaries,
    role,
):
    # An auto-resolved Director that is no longer active (corruption) exercises
    # the director-is-None branch — for every role: exit 1 with the "not found
    # in fleet" message and no spawn.
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
        role=role,
    )
    assert result.exit_code == 1, result.output
    assert "cannot resolve the member's coding agent" in result.output
    assert "not found in fleet" in result.output
    assert "Re-run with an explicit --coding-agent." in result.output
    assert split_window_recorder == []
