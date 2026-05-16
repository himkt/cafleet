"""Tests for ``cafleet member capture`` defaults (design doc 0000049, Surface 9).

Per principle (iii) of design 0000061: per-flag fragmented assertions collapse
into a small set of parametrized "shape + behaviour" tests covering line
defaults, ANSI stripping, CR defragmentation, and JSON envelope parity.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import cli
from cafleet.tmux import DirectorContext

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def bootstrapped_member(tmp_path, monkeypatch, _reset_engine_singletons):
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _FAKE_DIRECTOR_CTX)

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    create = runner.invoke(cli, ["session", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    sid = data["session_id"]
    director_id = data["director"]["agent_id"]

    pane_id = "%17"
    agent = broker.register_agent(
        session_id=sid,
        name="capture-target",
        description="member to capture from",
        placement={
            "director_agent_id": director_id,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": "claude",
        },
    )
    return sid, director_id, agent["agent_id"], pane_id, runner


def _record_run(monkeypatch, *, returns: str = "") -> list[list[str]]:
    from cafleet import tmux

    calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        calls.append(list(args))
        return returns

    monkeypatch.setattr(tmux, "_run", mock_run)
    return calls


@pytest.mark.parametrize(
    ("scenario", "extra_args", "expected_argv_suffix"),
    [
        ("default_no_flag_is_30", [], ["-S", "-30"]),
        ("explicit_lines_overrides_default", ["--lines", "150"], ["-S", "-150"]),
        ("tail_alias_forwards_to_lines", ["--tail", "55"], ["-S", "-55"]),
    ],
)
def test_member_capture__default_lines_and_flag_aliases(
    bootstrapped_member, monkeypatch, scenario, extra_args, expected_argv_suffix
):
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    calls = _record_run(monkeypatch)
    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
            *extra_args,
        ],
    )
    assert result.exit_code == 0, result.output
    capture_call = next(call for call in calls if "capture-pane" in call)
    assert capture_call[-2:] == expected_argv_suffix


@pytest.mark.parametrize(
    ("scenario", "raw", "ansi_flag", "expect_in", "expect_not_in"),
    [
        (
            "default_strips_simple_ansi",
            "\x1b[31mhello\x1b[0m world\n",
            False,
            ["hello world"],
            ["\x1b["],
        ),
        (
            "default_strips_complex_ansi",
            "\x1b[2J\x1b[H\x1b[1;33mwarn\x1b[0m: \x1b[Kdone\n",
            False,
            ["warn: done"],
            ["\x1b"],
        ),
        (
            "ansi_flag_preserves_raw_escapes",
            "\x1b[31mhello\x1b[0m world\n",
            True,
            ["\x1b[31m", "\x1b[0m"],
            [],
        ),
    ],
)
def test_member_capture__ansi_handling(
    bootstrapped_member,
    monkeypatch,
    scenario,
    raw,
    ansi_flag,
    expect_in,
    expect_not_in,
):
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    _record_run(monkeypatch, returns=raw)
    args = [
        "--session-id",
        sid,
        "member",
        "capture",
        "--agent-id",
        director_id,
        "--member-id",
        member_id,
    ]
    if ansi_flag:
        args.append("--ansi")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    for needle in expect_in:
        assert needle in result.output
    for needle in expect_not_in:
        assert needle not in result.output


@pytest.mark.parametrize(
    ("scenario", "raw", "ansi_flag", "expect_in", "expect_not_in"),
    [
        (
            "default_single_redraw_collapses_to_final",
            "loading...\rdone\nnext line\n",
            False,
            ["done", "next line"],
            ["loading"],
        ),
        (
            "default_multiple_redraws_collapse_to_final",
            "10%\r50%\r90%\rfinal\nafter\n",
            False,
            ["final", "after"],
            ["10%", "50%", "90%"],
        ),
        (
            "ansi_flag_preserves_carriage_returns",
            "loading...\rdone\n",
            True,
            ["loading", "\r"],
            [],
        ),
    ],
)
def test_member_capture__cr_defragmentation(
    bootstrapped_member,
    monkeypatch,
    scenario,
    raw,
    ansi_flag,
    expect_in,
    expect_not_in,
):
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    _record_run(monkeypatch, returns=raw)
    args = [
        "--session-id",
        sid,
        "member",
        "capture",
        "--agent-id",
        director_id,
        "--member-id",
        member_id,
    ]
    if ansi_flag:
        args.append("--ansi")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    for needle in expect_in:
        assert needle in result.output
    for needle in expect_not_in:
        assert needle not in result.output


def test_member_capture__json_envelope_post_processed_and_lines_default(
    bootstrapped_member, monkeypatch
):
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "\x1b[32mhello\x1b[0m\rworld\n"
    _record_run(monkeypatch, returns=raw)
    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["lines"] == 30
    assert "\x1b" not in payload["content"]
    assert "hello" not in payload["content"]
    assert "world" in payload["content"]
