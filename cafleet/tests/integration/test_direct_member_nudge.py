"""End-to-end contracts for the direct member ping path.

Real fleet/member registry rows drive the CLI: the fixed ``member ping``
keystroke against a live pane, the pending-placement no-op skip, and the
``monitor capture`` pending-placement hard error.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxMultiplexer
from tests.broker._helpers import (
    _create_fleet,
    _member_placement,
    _register_member,
)


def _ordinary(
    fleet: dict,
    *,
    name: str = "member",
    pane_id: str | None = "%5",
) -> dict:
    return _register_member(
        fleet["fleet_id"],
        name=name,
        placement=_member_placement(pane_id),
    )


@pytest.fixture
def poll_calls(monkeypatch):
    calls: list[dict] = []
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_poll_trigger",
        lambda self, **kwargs: calls.append(kwargs) or True,
    )
    return calls


def test_member_ping_dispatches_poll_trigger_to_live_pane(broker_session, poll_calls):
    fleet = _create_fleet()
    member = _ordinary(fleet)

    result = CliRunner().invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(member["member_id"]),
        ],
    )

    assert result.exit_code == 0, result.output
    assert poll_calls == [
        {
            "target_pane_id": "%5",
            "fleet_id": fleet["fleet_id"],
            "member_id": member["member_id"],
        }
    ]


def test_member_ping_pending_placement_skips_without_keystroke(
    broker_session, poll_calls
):
    fleet = _create_fleet()
    pending = _ordinary(fleet, name="pending", pane_id=None)

    result = CliRunner().invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(pending["member_id"]),
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload == {
        "member_id": pending["member_id"],
        "pane_id": None,
        "skipped": True,
    }
    assert poll_calls == []


def test_member_ping_skip_then_pane_bind_dispatches_normally(
    broker_session, poll_calls
):
    """The skip is a no-op, not a terminal state: once the placement binds a
    pane, the same member is pinged normally."""
    from cafleet import broker

    fleet = _create_fleet()
    pending = _ordinary(fleet, name="late-binder", pane_id=None)

    argv = [
        "member",
        "ping",
        "--fleet-id",
        str(fleet["fleet_id"]),
        "--member-id",
        str(pending["member_id"]),
        "--json",
    ]
    skipped = CliRunner().invoke(cli, argv)
    assert skipped.exit_code == 0, skipped.output
    assert json.loads(skipped.output)["skipped"] is True
    assert poll_calls == []

    broker.update_placement_pane_id(pending["member_id"], "%9")
    dispatched = CliRunner().invoke(cli, argv)

    assert dispatched.exit_code == 0, dispatched.output
    payload = json.loads(dispatched.output)
    assert payload == {
        "member_id": pending["member_id"],
        "pane_id": "%9",
        "skipped": False,
    }
    assert [call["target_pane_id"] for call in poll_calls] == ["%9"]


def test_monitor_capture_pending_placement_hard_errors(broker_session, monkeypatch):
    fleet = _create_fleet()
    pending = _ordinary(fleet, name="pending-capture", pane_id=None)
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)

    result = CliRunner().invoke(
        cli,
        [
            "monitor",
            "capture",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(pending["member_id"]),
        ],
    )

    assert result.exit_code == 1, result.output
    assert (
        f"member {pending['member_id']} has no pane yet (pending placement) "
        f"— nothing to capture." in result.output
    )
