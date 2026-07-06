"""CLI tests for ``cafleet member list --all`` (design 0000116).

``--all`` lists every active agent of the fleet — root Director,
Administrator, monitoring member, ordinary members, placementless rows —
with a ``kind`` column. The default (no ``--all``) output keeps today's
members-only view untouched, and ``--all`` is mutually exclusive with
``--activity``.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from tests.broker._helpers import _member_placement


@pytest.fixture
def bootstrapped_roster(_mock_tmux_for_fleet_create):
    """Fresh fleet + monitoring member + one ordinary member.

    The roster is exactly 4 active agents: root Director (placed),
    Administrator (placementless), monitor, and alice. Returns
    ``(sid, director_id, admin_id, monitor_id, alice_id, runner)``.
    """
    runner = CliRunner()
    create = runner.invoke(cli, ["fleet", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    admin_id = data["administrator_agent_id"]

    monitor = broker.register_agent(
        fleet_id=sid,
        name="monitor",
        description="Dedicated monitoring member",
        placement=_member_placement(director_id, "%5"),
        kind="monitoring-member",
    )
    alice = broker.register_agent(
        fleet_id=sid,
        name="alice",
        description="Ordinary member",
        placement=_member_placement(director_id, "%7"),
    )
    return sid, director_id, admin_id, monitor["agent_id"], alice["agent_id"], runner


def _list(runner, sid, *extra):
    return runner.invoke(
        cli,
        [
            *(("--json",) if "--json" in extra else ()),
            "member",
            "list",
            "--fleet-id",
            str(sid),
            *(a for a in extra if a != "--json"),
        ],
    )


# --- --all: full roster with kind derivation ---


def test_member_list_all__json_lists_every_active_agent_with_kind(
    bootstrapped_roster,
):
    sid, director_id, admin_id, monitor_id, alice_id, runner = bootstrapped_roster
    result = _list(runner, sid, "--all", "--json")
    assert result.exit_code == 0, result.output
    rows = {row["agent_id"]: row for row in json.loads(result.output)}
    assert set(rows) == {director_id, admin_id, monitor_id, alice_id}
    assert rows[director_id]["kind"] == "director"
    assert rows[admin_id]["kind"] == "administrator"
    assert rows[monitor_id]["kind"] == "monitor"
    assert rows[alice_id]["kind"] == "member"


def test_member_list_all__json_row_shape_and_null_placement(bootstrapped_roster):
    sid, director_id, admin_id, _monitor_id, alice_id, runner = bootstrapped_roster
    result = _list(runner, sid, "--all", "--json")
    assert result.exit_code == 0, result.output
    rows = {row["agent_id"]: row for row in json.loads(result.output)}

    for row in rows.values():
        assert set(row) == {
            "agent_id",
            "name",
            "description",
            "status",
            "registered_at",
            "placement",
            "kind",
        }
    # Placementless row → placement: null.
    assert rows[admin_id]["placement"] is None
    # Placed rows keep the placement sub-dict.
    assert rows[alice_id]["placement"]["mux_pane_id"] == "%7"
    assert rows[director_id]["placement"]["director_agent_id"] is None


def test_member_list_all__text_header_kind_column_and_dash_cells(
    bootstrapped_roster,
):
    sid, _d, admin_id, _m, _al, runner = bootstrapped_roster
    result = _list(runner, sid, "--all")
    assert result.exit_code == 0, result.output
    out = result.output

    assert "4 agents:" in out
    header = next(line for line in out.splitlines() if "agent_id" in line)
    assert "kind" in header
    # Placementless rows render "-" in every placement column.
    admin_line = next(line for line in out.splitlines() if "Administrator" in line)
    assert admin_line.split() == [
        str(admin_id),
        "Administrator",
        "active",
        "administrator",
        "-",
        "-",
        "-",
        "-",
        "-",
    ]


def test_member_list_all__text_placed_rows_render_placement(bootstrapped_roster):
    sid, director_id, _a, _m, _al, runner = bootstrapped_roster
    result = _list(runner, sid, "--all")
    assert result.exit_code == 0, result.output
    director_line = next(
        line for line in result.output.splitlines() if "Director" in line
    )
    cells = director_line.split()
    assert cells[0] == str(director_id)
    assert "director" in cells
    assert "claude" in cells
    assert "%0" in cells


# --- mutual exclusivity with --activity ---


def test_member_list_all__all_and_activity_mutually_exclusive(bootstrapped_roster):
    sid, _d, _a, _m, _al, runner = bootstrapped_roster
    result = _list(runner, sid, "--all", "--activity")
    assert result.exit_code == 2, result.output
    assert "--all and --activity are mutually exclusive." in (result.output or "")


# --- default (no --all) output unchanged ---


def test_member_list_default__members_only_shape_unchanged(bootstrapped_roster):
    """No ``--all`` → today's members-only view: root Director and the
    placementless Administrator excluded, no ``kind`` key in JSON rows."""
    sid, director_id, admin_id, monitor_id, alice_id, runner = bootstrapped_roster
    result = _list(runner, sid, "--json")
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)
    listed_ids = {row["agent_id"] for row in rows}
    assert listed_ids == {monitor_id, alice_id}
    assert director_id not in listed_ids
    assert admin_id not in listed_ids
    for row in rows:
        assert "kind" not in row


def test_member_list_default__text_header_and_columns_unchanged(bootstrapped_roster):
    sid, _d, _a, _m, _al, runner = bootstrapped_roster
    result = _list(runner, sid)
    assert result.exit_code == 0, result.output
    out = result.output
    assert "2 members:" in out
    assert "agents:" not in out
    header = next(line for line in out.splitlines() if "agent_id" in line)
    assert "kind" not in header
