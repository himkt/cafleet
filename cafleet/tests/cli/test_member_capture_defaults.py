"""Tests for ``cafleet member capture`` defaults."""

import hashlib
import json
from datetime import UTC, datetime

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from tests.broker._helpers import _member_placement


@pytest.fixture
def bootstrapped_member(_mock_tmux_for_fleet_create):
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
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]

    pane_id = "%17"
    member = broker.register_member(
        fleet_id=sid,
        name="capture-target",
        description="member to capture from",
        placement=_member_placement(pane_id),
    )
    # Stringify ids here: this module feeds them only into CLI argv (which must
    # be strings under ``--fleet-id``/``--member-id`` ``type=int``) and never
    # compares them as integers.
    return str(sid), str(director_id), str(member["member_id"]), pane_id, runner


def _record_run(monkeypatch, *, returns: str = "") -> list[list[str]]:
    from cafleet.multiplexer import tmux as multiplexer_tmux

    calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        calls.append(list(args))
        return returns

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    return calls


@pytest.mark.parametrize(
    ("scenario", "extra_args", "expected_argv_suffix"),
    [
        ("default_no_flag_is_20", [], ["-S", "-20"]),
        ("explicit_lines_overrides_default", ["--lines", "150"], ["-S", "-150"]),
    ],
)
def test_member_capture__default_lines_and_explicit_flag(
    bootstrapped_member, monkeypatch, scenario, extra_args, expected_argv_suffix
):
    sid, _director_id, member_id, _pane_id, runner = bootstrapped_member
    calls = _record_run(monkeypatch)
    result = runner.invoke(
        cli,
        [
            "member",
            "capture",
            "--fleet-id",
            sid,
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
    sid, _director_id, member_id, _pane_id, runner = bootstrapped_member
    _record_run(monkeypatch, returns=raw)
    args = [
        "member",
        "capture",
        "--fleet-id",
        sid,
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
    sid, _director_id, member_id, _pane_id, runner = bootstrapped_member
    _record_run(monkeypatch, returns=raw)
    args = [
        "member",
        "capture",
        "--fleet-id",
        sid,
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
    sid, _director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "\x1b[32mhello\x1b[0m\rworld\n"
    _record_run(monkeypatch, returns=raw)
    result = runner.invoke(
        cli,
        [
            "member",
            "capture",
            "--fleet-id",
            sid,
            "--member-id",
            member_id,
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["lines"] == 20
    assert "\x1b" not in payload["content"]
    assert "hello" not in payload["content"]
    assert "world" in payload["content"]


@pytest.mark.parametrize(
    ("ansi_flag", "raw", "expected_content"),
    [
        pytest.param(
            False,
            "\x1b[31mloading\rready\x1b[0m\n",
            "ready\n",
            id="no-ansi-hashes-emitted-defragmented-content",
        ),
        pytest.param(
            True,
            "\x1b[31mloading\rready\x1b[0m\n",
            "\x1b[31mloading\rready\x1b[0m\n",
            id="ansi-hashes-emitted-raw-content",
        ),
    ],
)
def test_member_capture__json_stamps_and_hashes_exact_emitted_content(
    bootstrapped_member,
    monkeypatch,
    ansi_flag,
    raw,
    expected_content,
):
    sid, _director_id, member_id, _pane_id, runner = bootstrapped_member
    _record_run(monkeypatch, returns=raw)
    args = [
        "member",
        "capture",
        "--fleet-id",
        sid,
        "--member-id",
        member_id,
        "--json",
    ]
    if ansi_flag:
        args.append("--ansi")
    before = datetime.now(UTC)

    result = runner.invoke(cli, args)

    after = datetime.now(UTC)
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert list(payload) == [
        "member_id",
        "pane_id",
        "lines",
        "content",
        "captured_at",
        "content_sha256",
    ]
    assert payload["content"] == expected_content
    captured_at = datetime.fromisoformat(payload["captured_at"])
    assert captured_at.tzinfo is not None
    assert captured_at.utcoffset() == UTC.utcoffset(None)
    assert before <= captured_at <= after
    assert (
        payload["content_sha256"]
        == hashlib.sha256(expected_content.encode("utf-8")).hexdigest()
    )
