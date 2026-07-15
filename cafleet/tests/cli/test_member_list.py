"""CLI tests for the single ``cafleet member list`` output shape.

``member list`` takes only ``--fleet-id`` (plus the shared trailing
``--json``) and lists every **active** registry entry of the fleet — root
Director, monitoring member, ordinary members, placementless rows. Text mode
renders exactly the ``member_id`` / ``name`` / ``kind`` / ``backend`` /
``pane_id`` / ``idle`` columns (placementless cells ``-``, a placed row with
no pane ``(pending)``, ``idle`` humanized ``Ns``/``Nm``/``Nh`` or ``-``); JSON
mode emits one dict per row with ``member_id`` / ``name`` / ``kind`` /
``placement`` (sub-dict or ``null``) and the activity fields ``last_sent`` /
``last_recv`` / ``last_ack`` / ``idle``.
"""

import json
import re

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer import MultiplexerContext
from tests.broker._helpers import _member_placement

_ROW_KEYS = {
    "member_id",
    "name",
    "kind",
    "placement",
    "last_sent",
    "last_recv",
    "last_ack",
    "idle",
}

_TEXT_COLUMNS = ["member_id", "name", "kind", "backend", "pane_id", "idle"]


@pytest.fixture
def bootstrapped_roster(_mock_tmux_for_fleet_create):
    """Fresh fleet + monitor + placed, placementless, and pending members.

    Exactly 5 active registry entries: root Director (placed, pane ``%0``),
    monitoring member, alice (placed, pane ``%7``), standalone (no placement
    row), pending (placement row with ``mux_pane_id`` ``None``). Returns
    ``(sid, ids, runner)`` where ``ids`` maps fixture names to member ids.
    """
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

    monitor = broker.register_member(
        fleet_id=sid,
        name="monitor",
        description="Dedicated monitoring member",
        placement=_member_placement("%5"),
        kind="monitoring-member",
    )
    alice = broker.register_member(
        fleet_id=sid,
        name="alice",
        description="Ordinary member",
        placement=_member_placement("%7"),
    )
    standalone = broker.register_member(
        fleet_id=sid,
        name="standalone",
        description="Placementless member",
    )
    pending = broker.register_member(
        fleet_id=sid,
        name="pending",
        description="Placement row without a pane",
        placement=_member_placement(None),
    )
    ids = {
        "director": data["director"]["member_id"],
        "monitor": monitor["member_id"],
        "alice": alice["member_id"],
        "standalone": standalone["member_id"],
        "pending": pending["member_id"],
    }
    return sid, ids, runner


def _list(runner, sid, *extra):
    return runner.invoke(
        cli,
        [
            "member",
            "list",
            "--fleet-id",
            str(sid),
            *extra,
        ],
    )


def _row_for(output: str, name: str) -> list[str]:
    line = next(line for line in output.splitlines() if f" {name} " in f" {line} ")
    return line.split()


# --- JSON shape ---


def test_member_list__json_lists_every_active_member_with_kind(bootstrapped_roster):
    sid, ids, runner = bootstrapped_roster
    result = _list(runner, sid, "--json")
    assert result.exit_code == 0, result.output
    rows = {row["member_id"]: row for row in json.loads(result.output)}
    assert set(rows) == set(ids.values())
    assert rows[ids["director"]]["kind"] == "director"
    assert rows[ids["monitor"]]["kind"] == "monitor"
    assert rows[ids["alice"]]["kind"] == "member"
    assert rows[ids["standalone"]]["kind"] == "member"
    assert rows[ids["pending"]]["kind"] == "member"


def test_member_list__json_row_key_set_and_placement(bootstrapped_roster):
    sid, ids, runner = bootstrapped_roster
    result = _list(runner, sid, "--json")
    assert result.exit_code == 0, result.output
    rows = {row["member_id"]: row for row in json.loads(result.output)}

    for row in rows.values():
        assert set(row) == _ROW_KEYS
    # Placementless row → placement: null.
    assert rows[ids["standalone"]]["placement"] is None
    # Placed rows keep the placement sub-dict; pending keeps a null pane id.
    assert rows[ids["alice"]]["placement"]["mux_pane_id"] == "%7"
    assert rows[ids["director"]]["placement"]["mux_pane_id"] == "%0"
    assert rows[ids["pending"]]["placement"]["mux_pane_id"] is None


def test_member_list__json_excludes_deregistered_and_other_fleets(
    bootstrapped_roster,
):
    sid, ids, runner = bootstrapped_roster
    broker.deregister_member(ids["alice"])
    other = broker.create_fleet(
        name=None,
        director_context=MultiplexerContext(
            session="main", window_id="@3", pane_id="%0"
        ),
        coding_agent="claude",
        backend="tmux",
    )

    result = _list(runner, sid, "--json")
    assert result.exit_code == 0, result.output
    listed = {row["member_id"] for row in json.loads(result.output)}
    assert ids["alice"] not in listed
    assert other["director"]["member_id"] not in listed


def test_member_list__json_activity_null_for_silent_members(bootstrapped_roster):
    sid, _ids, runner = bootstrapped_roster
    result = _list(runner, sid, "--json")
    assert result.exit_code == 0, result.output
    for row in json.loads(result.output):
        assert row["last_sent"] is None
        assert row["last_recv"] is None
        assert row["last_ack"] is None
        assert row["idle"] is None


def test_member_list__json_activity_visible_after_send(bootstrapped_roster):
    sid, ids, runner = bootstrapped_roster
    sent = broker.send_message(sid, ids["alice"], ids["monitor"], "hello")

    result = _list(runner, sid, "--json")
    assert result.exit_code == 0, result.output
    rows = {row["member_id"]: row for row in json.loads(result.output)}

    assert rows[ids["alice"]]["last_sent"] == sent["message"]["status_timestamp"]
    assert rows[ids["monitor"]]["last_recv"] == sent["message"]["status_timestamp"]
    assert rows[ids["alice"]]["idle"] is not None
    assert rows[ids["monitor"]]["idle"] is not None


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_member_list__json_bootstrap_only_lists_the_director():
    runner = CliRunner()
    create = runner.invoke(
        cli, ["fleet", "create", "--name", "bare", "--coding-agent", "claude", "--json"]
    )
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)

    result = _list(runner, data["fleet_id"], "--json")
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)
    assert [row["member_id"] for row in rows] == [data["director"]["member_id"]]
    assert rows[0]["kind"] == "director"


# --- text shape ---


def test_member_list__text_header_is_exactly_the_six_columns(bootstrapped_roster):
    sid, _ids, runner = bootstrapped_roster
    result = _list(runner, sid)
    assert result.exit_code == 0, result.output
    header = next(line for line in result.output.splitlines() if "member_id" in line)
    assert header.split() == _TEXT_COLUMNS


def test_member_list__text_placementless_row_renders_dash_cells(bootstrapped_roster):
    sid, ids, runner = bootstrapped_roster
    result = _list(runner, sid)
    assert result.exit_code == 0, result.output
    assert _row_for(result.output, "standalone") == [
        str(ids["standalone"]),
        "standalone",
        "member",
        "-",
        "-",
        "-",
    ]


def test_member_list__text_pending_pane_renders_pending(bootstrapped_roster):
    sid, ids, runner = bootstrapped_roster
    result = _list(runner, sid)
    assert result.exit_code == 0, result.output
    assert _row_for(result.output, "pending") == [
        str(ids["pending"]),
        "pending",
        "member",
        "claude",
        "(pending)",
        "-",
    ]


def test_member_list__text_placed_rows_render_backend_pane_and_kind(
    bootstrapped_roster,
):
    sid, ids, runner = bootstrapped_roster
    result = _list(runner, sid)
    assert result.exit_code == 0, result.output
    assert _row_for(result.output, "alice") == [
        str(ids["alice"]),
        "alice",
        "member",
        "claude",
        "%7",
        "-",
    ]
    director_row = _row_for(result.output, "Director")
    assert director_row[0] == str(ids["director"])
    assert "director" in director_row
    assert "%0" in director_row


def test_member_list__text_idle_humanized_after_send(bootstrapped_roster):
    sid, ids, runner = bootstrapped_roster
    broker.send_message(sid, ids["alice"], ids["monitor"], "hello")

    result = _list(runner, sid)
    assert result.exit_code == 0, result.output
    alice_row = _row_for(result.output, "alice")
    # A just-sent message puts alice's idle in the seconds bucket (``Ns``).
    assert re.fullmatch(r"\d+s", alice_row[-1])
    # A silent member keeps the ``-`` idle cell.
    assert _row_for(result.output, "standalone")[-1] == "-"
