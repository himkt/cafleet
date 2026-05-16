"""Surface 3 — slim broadcast echo + ``--quiet`` writes (design 0000049 Step 6).

Per principle (iii) of design 0000061: per-command + per-mode fragmentation
collapses to a small set of parametrized tests.
"""

import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli

DIRECTOR_ID = "11111111-1111-1111-1111-111111111111"
MEMBER_ID = "22222222-2222-2222-2222-222222222222"
PANE_ID = "%7"


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def agent_id():
    return str(uuid.uuid4())


@pytest.fixture(autouse=True)
def _stub_verify(monkeypatch):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *_a, **_k: True)


def _typed_task(
    *,
    task_id: str = "abcdef0123456789-tail",
    sender: str = "ffffffff11112222-tail",
    recipient: str = "rrrrrrrr11112222-tail",
    text: str = "short body",
    type_: str = "unicast",
    origin_task_id: str | None = None,
    status_state: str = "input_required",
) -> dict:
    return {
        "task_id": task_id,
        "context_id": recipient,
        "from_agent_id": sender,
        "to_agent_id": recipient,
        "type": type_,
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": status_state,
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": origin_task_id,
        "text": text,
    }


def _broadcast_summary_result(*, summary_id="abcdef0123456789-tail", recipient_count=3):
    summary = _typed_task(
        task_id=summary_id,
        sender="ffffffff11112222-tail",
        recipient="",
        text=f"Broadcast sent to {recipient_count} recipients",
        type_="broadcast_summary",
        origin_task_id=summary_id,
        status_state="completed",
    )
    return [{"task": summary, "notifications_sent_count": recipient_count}]


def _ping_setup(monkeypatch):
    placement = {
        "director_agent_id": DIRECTOR_ID,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": PANE_ID,
        "coding_agent": "claude",
        "created_at": "2026-04-16T08:00:00+00:00",
    }
    agent = {
        "agent_id": MEMBER_ID,
        "name": "Claude-B",
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": placement,
    }
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_k: agent)
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "send_poll_trigger", lambda **_k: True, raising=False)


def _setup_command(monkeypatch, command, session_id, agent_id):
    if command == "broadcast":
        monkeypatch.setattr(
            broker, "broadcast_message",
            lambda *_a, **_k: _broadcast_summary_result(recipient_count=3),
        )
        return ["message", "broadcast", "--agent-id", agent_id, "--text", "hello"]
    if command == "send":
        monkeypatch.setattr(
            broker, "send_message",
            lambda *_a, **_k: {
                "task": _typed_task(text="hello"),
                "notification_sent": True,
            },
        )
        return [
            "message", "send", "--agent-id", agent_id,
            "--to", str(uuid.uuid4()), "--text", "hello",
        ]
    if command == "ack":
        monkeypatch.setattr(
            broker, "ack_task",
            lambda *_a, **_k: {
                "task": _typed_task(text="hello", status_state="completed"),
            },
        )
        return [
            "message", "ack", "--agent-id", agent_id,
            "--task-id", "abcdef0123456789-tail",
        ]
    # ping
    _ping_setup(monkeypatch)
    return ["member", "ping", "--agent-id", DIRECTOR_ID, "--member-id", MEMBER_ID]


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
    runner, session_id, agent_id, monkeypatch,
    command, mode, expect_oneline, must_not_contain,
):
    args = _setup_command(monkeypatch, command, session_id, agent_id)
    if mode == "quiet":
        args.append("--quiet")
    elif mode == "full":
        args.append("--full")
    result = runner.invoke(cli, ["--session-id", session_id, *args])
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    if expect_oneline:
        assert "\n" not in out, f"expected one-line echo; got:\n{result.output}"
    else:
        # Default verbose echoes contain meaningful content beyond an 8-char id.
        assert out != "abcdef01"
    for forbidden in must_not_contain:
        assert forbidden not in out


def test_broadcast_default__canonical_summary_pattern(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker, "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(
            summary_id="abcdef0123456789-tail", recipient_count=3
        ),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id", session_id, "message", "broadcast",
            "--agent-id", agent_id, "--text", "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    assert "broadcast" in out
    assert "id=abcdef01" in out
    assert "recipients=3" in out


def test_broadcast_full__multi_line_per_recipient_envelopes(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker, "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(recipient_count=3),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id", session_id, "message", "broadcast",
            "--agent-id", agent_id, "--text", "hello", "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    for needle in ("from:", "type:", "text:"):
        assert needle in result.output


@pytest.mark.parametrize("command", ["send", "ack"])
def test_quiet__emits_only_task_id_8(
    runner, session_id, agent_id, monkeypatch, command
):
    args = _setup_command(monkeypatch, command, session_id, agent_id) + ["--quiet"]
    result = runner.invoke(cli, ["--session-id", session_id, *args])
    assert result.exit_code == 0, result.output
    assert result.output.strip() == "abcdef01"
