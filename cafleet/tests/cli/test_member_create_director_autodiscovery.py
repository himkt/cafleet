"""Tests for ``cafleet member create`` Director auto-discovery (issue #185).

``member create`` takes no Director-identity flag: it resolves the Director
from ``fleets.director_member_id`` first thing — before any multiplexer or
registration side effect — and fails loudly per the design 0000127
§ Issue #185 error table.
"""

import json
import sqlite3

import pytest
from click.testing import CliRunner

from cafleet.cli import cli


def _invoke_member_create(runner: CliRunner, fleet_id: int):
    return runner.invoke(
        cli,
        [
            "member",
            "create",
            "--fleet-id",
            str(fleet_id),
            "--name",
            "Member",
            "--description",
            "Member for tests",
            "--text",
            "do work",
        ],
    )


def _db_execute(db_path, sql: str, params: tuple):
    conn = sqlite3.connect(db_path)
    try:
        with conn:
            conn.execute(sql, params)
    finally:
        conn.close()


def _fetch_director_member_id(db_path, fleet_id: int) -> int:
    conn = sqlite3.connect(db_path)
    try:
        row = conn.execute(
            "SELECT director_member_id FROM fleets WHERE fleet_id = ?", (fleet_id,)
        ).fetchone()
    finally:
        conn.close()
    assert row is not None
    return row[0]


@pytest.fixture
def bootstrapped_fleet(_cli_registry, _mock_tmux_for_fleet_create):
    """(fleet_id, runner, db_path) for a freshly bootstrapped fleet."""
    runner = CliRunner()
    create = runner.invoke(cli, ["fleet", "create", "--name", "test-fleet", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    return data["fleet_id"], runner, _cli_registry


def test_member_create__missing_fleet_is_usage_error_exit_2():
    # Resolution happens first thing, so no tmux or binary stubs are needed.
    runner = CliRunner()
    result = _invoke_member_create(runner, 999)
    assert result.exit_code == 2, result.output
    assert "Fleet '999' not found." in result.output


def test_member_create__deleted_fleet_exits_1(bootstrapped_fleet):
    fleet_id, runner, db_path = bootstrapped_fleet
    _db_execute(
        db_path,
        "UPDATE fleets SET deleted_at = ? WHERE fleet_id = ?",
        ("2026-07-11T00:00:00+00:00", fleet_id),
    )
    result = _invoke_member_create(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert f"fleet {fleet_id} is deleted" in result.output


def test_member_create__null_director_exits_1(bootstrapped_fleet):
    # Mid-bootstrap corruption: the fleet row exists but the Director back-fill
    # never landed.
    fleet_id, runner, db_path = bootstrapped_fleet
    _db_execute(
        db_path,
        "UPDATE fleets SET director_member_id = NULL WHERE fleet_id = ?",
        (fleet_id,),
    )
    result = _invoke_member_create(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert (
        f"fleet {fleet_id} has no root Director recorded; "
        "re-create the fleet with 'cafleet fleet create'." in result.output
    )


def test_member_create__inactive_root_director_exits_1(bootstrapped_fleet, monkeypatch):
    fleet_id, runner, db_path = bootstrapped_fleet
    director_id = _fetch_director_member_id(db_path, fleet_id)
    _db_execute(
        db_path,
        "UPDATE members SET status = 'deregistered', deregistered_at = ? "
        "WHERE member_id = ?",
        ("2026-07-11T00:00:00+00:00", director_id),
    )

    # The invariant guard fires inside register_member, past the multiplexer
    # and binary availability checks (the tmux stubs from the bootstrap
    # fixture are still active) — stub the binary lookup and record that no
    # pane is ever split.
    monkeypatch.setattr(
        "cafleet.coding_agent.base.shutil.which", lambda _: "/usr/bin/stub"
    )
    split_calls: list[dict] = []
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.split_window",
        lambda self, **kwargs: split_calls.append(kwargs) or "%42",
    )

    result = _invoke_member_create(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert (
        f"fleet {fleet_id}'s root Director (member {director_id}) is not active."
        in result.output
    )
    assert split_calls == []
