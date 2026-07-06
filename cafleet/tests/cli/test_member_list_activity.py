"""Tests for ``cafleet member list --activity``.

The CLI surfaces the new aggregation via an opt-in ``--activity`` flag on
``cafleet member list``. Two requirements:

* When the flag is OMITTED, the existing wire shape is preserved verbatim
  (no ``last_sent`` / ``last_recv`` / ``last_ack`` / ``idle`` keys leak into
  the JSON response). This protects every existing JSON consumer from a
  silent shape change.
* When the flag is SET, the four activity keys appear on every row in
  ``--json`` mode, and the text-mode rendering includes column labels for
  the four fields (rendered via the new ``output.format_member_list_activity``
  formatter).
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli


@pytest.fixture
def bootstrapped_team(_mock_tmux_for_fleet_create):
    """Fresh fleet + 3 registered members. Returns ``(sid, director,
    [member_ids], runner)``.

    Members are registered via ``broker.register_agent`` (placement supplied)
    so the test does not have to spin up real tmux panes via ``member create``.
    """
    runner = CliRunner()

    create = runner.invoke(cli, ["fleet", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]

    members: list[str] = []
    for i, name in enumerate(("alice", "bob", "carol")):
        agent = broker.register_agent(
            fleet_id=sid,
            name=name,
            description=f"member {name}",
            placement={
                "director_agent_id": director_id,
                "backend": "tmux",
                "mux_session": "main",
                "mux_window_id": "@3",
                "mux_pane_id": f"%{10 + i}",
                "coding_agent": "claude",
            },
        )
        members.append(agent["agent_id"])

    return sid, director_id, members, runner


# --- baseline: no flag ---


def test_member_list_no_activity_flag__omits_activity_keys(bootstrapped_team):
    """Default ``cafleet member list`` MUST keep the baseline JSON shape — no
    ``last_sent`` / ``last_recv`` / ``last_ack`` / ``idle`` keys. This guards
    against accidental wire-shape regressions for downstream JSON consumers."""
    sid, _director_id, _members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "--json",
            "member",
            "list",
            "--fleet-id",
            str(sid),
        ],
    )
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)

    for row in rows:
        assert "last_sent" not in row
        assert "last_recv" not in row
        assert "last_ack" not in row
        assert "idle" not in row


def test_member_list__scoped_by_fleet_id_lists_members_excludes_root(bootstrapped_team):
    """``member list`` takes only the global ``--fleet-id`` (no ``--agent-id``):
    it lists every member of the fleet and never surfaces the root Director."""
    sid, director_id, members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "--json",
            "member",
            "list",
            "--fleet-id",
            str(sid),
        ],
    )
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)
    listed_ids = {row["agent_id"] for row in rows}
    assert listed_ids == set(members)
    assert director_id not in listed_ids


def test_member_list__agent_id_flag_removed(bootstrapped_team):
    """``member list`` no longer accepts ``--agent-id`` — Click rejects it with
    its standard 'no such option' error (exit 2)."""
    sid, _director_id, _members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "member",
            "list",
            "--fleet-id",
            str(sid),
            "--agent-id",
            "999",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()


# --- with --activity ---


def test_member_list_activity_flag__json_emits_activity_keys(bootstrapped_team):
    sid, _director_id, _members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "--json",
            "member",
            "list",
            "--fleet-id",
            str(sid),
            "--activity",
        ],
    )
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)
    assert len(rows) == 3

    for row in rows:
        assert "last_sent" in row
        assert "last_recv" in row
        assert "last_ack" in row
        assert "idle" in row


def test_member_list_activity_flag__none_for_silent_members(bootstrapped_team):
    """A member with zero send/receive history MUST have all four activity
    fields rendered as ``null`` in JSON mode (not omitted, not the empty
    string) so downstream parsers can apply uniform null-checks."""
    sid, _director_id, _members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "--json",
            "member",
            "list",
            "--fleet-id",
            str(sid),
            "--activity",
        ],
    )
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)

    for row in rows:
        assert row["last_sent"] is None
        assert row["last_recv"] is None
        assert row["last_ack"] is None
        assert row["idle"] is None


def test_member_list_activity_flag__activity_visible_after_send(bootstrapped_team):
    """After alice sends to bob, alice's ``last_sent`` and bob's ``last_recv``
    transition from null to a non-null timestamp string. The exact value is
    derived from ``status_timestamp`` (broker-controlled) so we only assert
    presence + non-null."""
    sid, _director_id, members, runner = bootstrapped_team
    alice, bob, _carol = members

    sent = broker.send_message(sid, alice, bob, "hello")

    result = runner.invoke(
        cli,
        [
            "--json",
            "member",
            "list",
            "--fleet-id",
            str(sid),
            "--activity",
        ],
    )
    assert result.exit_code == 0, result.output
    rows = {row["agent_id"]: row for row in json.loads(result.output)}

    assert rows[alice]["last_sent"] == sent["task"]["status_timestamp"]
    assert rows[bob]["last_recv"] == sent["task"]["status_timestamp"]
    assert rows[alice]["idle"] is not None
    assert rows[bob]["idle"] is not None


def test_member_list_activity_flag__text_mode_includes_activity_columns(
    bootstrapped_team,
):
    """Text-mode rendering with ``--activity`` MUST advertise the four
    activity columns in the header so an operator scanning ``cafleet member
    list --activity`` immediately understands what they're looking at."""
    sid, _director_id, _members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "member",
            "list",
            "--fleet-id",
            str(sid),
            "--activity",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output

    assert "last_sent" in out
    assert "last_recv" in out
    assert "last_ack" in out
    assert "idle" in out


def test_member_list_activity_flag__text_mode_default_omits_activity_columns(
    bootstrapped_team,
):
    """Text-mode rendering WITHOUT ``--activity`` MUST keep the existing
    column set — no ``last_sent`` / ``last_recv`` / ``last_ack`` / ``idle``
    headers. This guards against the formatter being switched globally."""
    sid, _director_id, _members, runner = bootstrapped_team

    result = runner.invoke(
        cli,
        [
            "member",
            "list",
            "--fleet-id",
            str(sid),
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output

    assert "last_sent" not in out
    assert "last_recv" not in out
    assert "last_ack" not in out
