"""Slim broadcast echo and ``--quiet`` writes."""

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxMultiplexer

DIRECTOR_ID = 11
MEMBER_ID = 22
PANE_ID = "%7"


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture
def member_id():
    return 200


@pytest.fixture(autouse=True)
def _stub_verify(monkeypatch):
    monkeypatch.setattr(broker, "verify_member_fleet", lambda *_a, **_k: True)


def _typed_message(
    *,
    message_id: int = 5000,
    sender: int = 111,
    recipient: int = 222,
    text: str = "short body",
    type_: str = "unicast",
    origin_message_id: int | None = None,
    status_state: str = "input_required",
) -> dict:
    return {
        "message_id": message_id,
        "owner_member_id": recipient,
        "from_member_id": sender,
        "to_member_id": recipient,
        "type": type_,
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": status_state,
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_message_id": origin_message_id,
        "text": text,
    }


def _broadcast_summary_result(*, summary_id=7000, recipient_count=3):
    summary = _typed_message(
        message_id=summary_id,
        sender=111,
        recipient=0,
        text=f"Broadcast sent to {recipient_count} recipients",
        type_="broadcast_summary",
        origin_message_id=summary_id,
        status_state="completed",
    )
    summary["to_member_id"] = None
    return [
        {
            "message": summary,
            "recipients": recipient_count,
            "delivered": recipient_count,
        }
    ]


def _ping_setup(monkeypatch):
    placement = {
        "backend": "tmux",
        "mux_session": "main",
        "mux_window_id": "@3",
        "mux_pane_id": PANE_ID,
        "coding_agent": "claude",
        "created_at": "2026-04-16T08:00:00+00:00",
    }
    member = {
        "member_id": MEMBER_ID,
        "name": "Claude-B",
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "member",
        "placement": placement,
    }
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_k: member)
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)
    monkeypatch.setattr(
        TmuxMultiplexer, "send_poll_trigger", lambda self, **_k: True, raising=False
    )


def _setup_command(monkeypatch, command, fleet_id, member_id):
    if command == "broadcast":
        monkeypatch.setattr(
            broker,
            "broadcast_message",
            lambda *_a, **_k: _broadcast_summary_result(recipient_count=3),
        )
        return [
            "message",
            "broadcast",
            "--fleet-id",
            str(fleet_id),
            "--from-member-id",
            str(member_id),
            "--text",
            "hello",
        ]
    if command == "send":
        monkeypatch.setattr(
            broker,
            "send_message",
            lambda *_a, **_k: {
                "message": _typed_message(text="hello"),
                "notification_sent": True,
            },
        )
        return [
            "message",
            "send",
            "--fleet-id",
            str(fleet_id),
            "--from-member-id",
            str(member_id),
            "--to-member-id",
            "999",
            "--text",
            "hello",
        ]
    if command == "ack":
        monkeypatch.setattr(
            broker,
            "ack_message",
            lambda *_a, **_k: {
                "message": _typed_message(text="hello", status_state="completed"),
            },
        )
        return [
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            "5000",
        ]
    # ping
    _ping_setup(monkeypatch)
    return [
        "member",
        "ping",
        "--fleet-id",
        str(fleet_id),
        "--member-id",
        str(MEMBER_ID),
    ]


@pytest.mark.parametrize(
    ("command", "mode", "expect_oneline", "must_not_contain"),
    [
        # broadcast default IS one-line; --full is multi-line.
        ("broadcast", "default", True, []),
        ("broadcast", "full", False, []),
        # send / ack / ping default are multi-line; --quiet → one-line.
        ("send", "default", False, []),
        ("send", "quiet", True, []),
        ("ack", "default", False, []),
        ("ack", "quiet", True, []),
        ("ping", "default", False, []),
        ("ping", "quiet", True, ["dispatched", "Pinged"]),
    ],
)
def test_command_echo__one_line_vs_multi_line_shape(
    runner,
    fleet_id,
    member_id,
    monkeypatch,
    command,
    mode,
    expect_oneline,
    must_not_contain,
):
    args = _setup_command(monkeypatch, command, fleet_id, member_id)
    if mode == "quiet":
        args.append("--quiet")
    elif mode == "full":
        args.append("--full")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    if expect_oneline:
        assert "\n" not in out, f"expected one-line echo; got:\n{result.output}"
    else:
        # Default verbose echoes contain meaningful content beyond the bare id.
        assert out != "5000"
    for forbidden in must_not_contain:
        assert forbidden not in out


def test_broadcast_default__canonical_summary_pattern(
    runner, fleet_id, member_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(summary_id=7000, recipient_count=3),
    )
    result = runner.invoke(
        cli,
        [
            "message",
            "broadcast",
            "--fleet-id",
            str(fleet_id),
            "--from-member-id",
            str(member_id),
            "--text",
            "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    assert "broadcast" in out
    assert "id=7000" in out
    assert "recipients=3" in out


def test_broadcast_full__multi_line_per_recipient_envelopes(
    runner, fleet_id, member_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(recipient_count=3),
    )
    result = runner.invoke(
        cli,
        [
            "message",
            "broadcast",
            "--fleet-id",
            str(fleet_id),
            "--from-member-id",
            str(member_id),
            "--text",
            "hello",
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    for needle in ("from:", "type:", "text:"):
        assert needle in result.output


@pytest.mark.parametrize("command", ["send", "ack"])
def test_quiet__emits_only_message_id(
    runner, fleet_id, member_id, monkeypatch, command
):
    args = _setup_command(monkeypatch, command, fleet_id, member_id) + ["--quiet"]
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    assert result.output.strip() == "5000"
