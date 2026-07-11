"""CLI tests for ``cafleet member show`` (design 0000116).

``member show`` is a pure registry read: it takes a bare ``--member-id``
target (no requester identity flag, no fleet-membership gate), requires no
tmux, and renders detail for any active in-fleet member — placed or
placementless, root Director and Administrator included.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer
from tests.broker._helpers import _member_placement


@pytest.fixture
def bootstrapped_fleet(_mock_tmux_for_fleet_create):
    """Fresh fleet + one placed member + the monitoring member.

    Returns ``(sid, director_id, admin_id, alice_id, monitor_id, runner)``.
    Members are registered via ``broker.register_member`` (internal machinery)
    so no real tmux pane is spawned.
    """
    runner = CliRunner()
    create = runner.invoke(cli, ["fleet", "create", "--name", "test-fleet", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    admin_id = data["administrator_member_id"]

    alice = broker.register_member(
        fleet_id=sid,
        name="alice",
        description="Test member",
        placement=_member_placement("%10"),
    )
    monitor = broker.register_member(
        fleet_id=sid,
        name="monitor",
        description="Dedicated monitoring member",
        placement=_member_placement("%11"),
        kind="monitoring-member",
    )
    return sid, director_id, admin_id, alice["member_id"], monitor["member_id"], runner


def _show(runner, sid, member_id, *extra):
    return runner.invoke(
        cli,
        [
            "member",
            "show",
            "--fleet-id",
            str(sid),
            "--member-id",
            str(member_id),
            *extra,
        ],
    )


# --- default text: compact one-line row ---


def test_member_show__default_text_is_compact_one_line_row(bootstrapped_fleet):
    sid, _d, _a, alice_id, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, alice_id)
    assert result.exit_code == 0, result.output
    assert result.output.strip() == f"{alice_id} alice active"


def test_member_show__placementless_administrator_default_row(bootstrapped_fleet):
    sid, _d, admin_id, _al, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, admin_id)
    assert result.exit_code == 0, result.output
    assert result.output.strip() == f"{admin_id} Administrator active"


# --- not-found boundary (cross-fleet / unknown / inactive) ---


def test_member_show__unknown_target_exits_one_not_found(bootstrapped_fleet):
    sid, _d, _a, _al, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, 999999)
    assert result.exit_code == 1, result.output
    assert "Error: Member 999999 not found" in (result.output or "")


def test_member_show__cross_fleet_target_exits_one_not_found(bootstrapped_fleet):
    sid, _d, _a, alice_id, _m, runner = bootstrapped_fleet
    other = runner.invoke(cli, ["fleet", "create", "--name", "test-fleet", "--json"])
    assert other.exit_code == 0, other.output
    other_sid = json.loads(other.output)["fleet_id"]
    assert other_sid != sid

    result = _show(runner, other_sid, alice_id)
    assert result.exit_code == 1, result.output
    assert f"Error: Member {alice_id} not found" in (result.output or "")


def test_member_show__inactive_target_exits_one_not_found(bootstrapped_fleet):
    sid, _d, _a, alice_id, _m, runner = bootstrapped_fleet
    assert broker.deregister_member(alice_id) is True
    result = _show(runner, sid, alice_id)
    assert result.exit_code == 1, result.output
    assert f"Error: Member {alice_id} not found" in (result.output or "")


# --- --full text: labeled block ---


def test_member_show__full_block_placementless_administrator(bootstrapped_fleet):
    sid, _d, admin_id, _al, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, admin_id, "--full")
    assert result.exit_code == 0, result.output
    out = result.output
    assert f"  member_id:   {admin_id}" in out
    assert "  name:        Administrator" in out
    assert f"  description: Built-in administrator for fleet {sid}" in out
    assert "  status:      active" in out
    assert "  kind:        administrator" in out
    assert "  skills:      -" in out
    assert "  placement:   none" in out


def test_member_show__full_block_root_director_placement_sub_block(bootstrapped_fleet):
    sid, director_id, _a, _al, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, director_id, "--full")
    assert result.exit_code == 0, result.output
    out = result.output
    assert f"  member_id:   {director_id}" in out
    assert "  name:        Director" in out
    assert "  description: Root Director for this fleet" in out
    assert "  status:      active" in out
    assert "  kind:        director" in out
    assert "  skills:      -" in out
    assert "  placement:\n" in out
    assert "    backend:    claude" in out
    assert "    session:    main" in out
    assert "    window_id:  @3" in out
    assert "    pane_id:    %0" in out
    assert "    created_at: " in out
    assert "  placement:   none" not in out


def test_member_show__full_block_member_kind_and_pane(bootstrapped_fleet):
    sid, _director_id, _a, alice_id, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, alice_id, "--full")
    assert result.exit_code == 0, result.output
    out = result.output
    assert "  kind:        member" in out
    assert "    pane_id:    %10" in out


# --- --json: the broker get_member dict, unprojected ---


def test_member_show__json_shape_placed_member(bootstrapped_fleet):
    sid, _director_id, _a, alice_id, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, alice_id, "--json")
    assert result.exit_code == 0, result.output
    row = json.loads(result.output)
    assert set(row) == {
        "member_id",
        "name",
        "description",
        "status",
        "registered_at",
        "kind",
        "skills",
        "placement",
    }
    assert row["member_id"] == alice_id
    assert row["name"] == "alice"
    assert row["status"] == "active"
    assert row["kind"] == "member"
    assert row["skills"] == []
    assert row["placement"]["coding_agent"] == "claude"
    assert row["placement"]["mux_pane_id"] == "%10"


def test_member_show__json_placementless_is_null_placement(bootstrapped_fleet):
    sid, _d, admin_id, _al, _m, runner = bootstrapped_fleet
    result = _show(runner, sid, admin_id, "--json")
    assert result.exit_code == 0, result.output
    row = json.loads(result.output)
    assert row["member_id"] == admin_id
    assert row["kind"] == "administrator"
    assert row["placement"] is None


@pytest.mark.parametrize(
    ("target", "expected_kind"),
    [("director", "director"), ("monitor", "monitor")],
)
def test_member_show__json_kind_derivation(bootstrapped_fleet, target, expected_kind):
    sid, director_id, _a, _al, monitor_id, runner = bootstrapped_fleet
    member_id = director_id if target == "director" else monitor_id
    result = _show(runner, sid, member_id, "--json")
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["kind"] == expected_kind


def test_member_show__json_identical_with_and_without_full(bootstrapped_fleet):
    """``--full`` affects text mode only; the JSON payload is unprojected
    either way."""
    sid, _d, _a, alice_id, _m, runner = bootstrapped_fleet
    plain = _show(runner, sid, alice_id, "--json")
    full = _show(runner, sid, alice_id, "--json", "--full")
    assert plain.exit_code == 0, plain.output
    assert full.exit_code == 0, full.output
    assert json.loads(plain.output) == json.loads(full.output)


# --- no tmux requirement ---


def test_member_show__succeeds_without_tmux(bootstrapped_fleet, monkeypatch):
    """``member show`` is a registry read — it MUST NOT gate on tmux."""
    sid, _d, _a, alice_id, _m, runner = bootstrapped_fleet

    def _no_tmux(self):
        raise TmuxError("tmux binary not found")

    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", _no_tmux)
    result = _show(runner, sid, alice_id)
    assert result.exit_code == 0, result.output
    assert result.output.strip() == f"{alice_id} alice active"
