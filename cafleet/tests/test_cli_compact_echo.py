"""Surface 3 — slim broadcast echo + ``--quiet`` writes (design 0000049 Step 6).

Covers two CLI surfaces:

1. ``cafleet message broadcast`` text echo
   - default: a single-line summary ``broadcast id=<id8> recipients=<count>``
     (no per-recipient envelopes)
   - ``--full``: per-recipient envelopes via ``format_indexed_list``

2. ``--quiet`` flag on ``message send`` / ``message ack`` / ``member ping``
   - ``message send --quiet``: emits ONLY the new task id 8-char prefix on a
     single line
   - ``message ack --quiet``: emits ONLY the acked task id 8-char prefix
   - ``member ping --quiet``: emits a single short line (no multi-line echo)
   - default (no ``--quiet``): the existing verbose echo

Compact pure-function tests for the helpers themselves live in
``test_output_compact_formatters.py``.
"""

import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli

LONG_BODY = "abcdefghijklmnopqrstuvwxyz"
TRUNCATED_BODY = "abcdefghij..."

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


# ---------------------------------------------------------------------------
# 1. message broadcast: default 1-line summary, --full per-recipient envelopes
# ---------------------------------------------------------------------------


def _broadcast_summary_result(
    *,
    summary_id: str = "abcdef0123456789-tail",
    recipient_count: int = 3,
) -> list[dict]:
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


def test_broadcast_default_echo__single_line_summary(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(
            summary_id="abcdef0123456789-tail", recipient_count=3
        ),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    # Single-line summary — exactly one line of meaningful content.
    assert "\n" not in out, (
        f"default broadcast echo must be a single line; got:\n{result.output}"
    )


def test_broadcast_default_echo__contains_summary_id8(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(
            summary_id="abcdef0123456789-tail", recipient_count=3
        ),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "abcdef01" in result.output


def test_broadcast_default_echo__contains_recipient_count(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(
            summary_id="abcdef0123456789-tail", recipient_count=3
        ),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "3" in result.output, (
        f"default broadcast echo must surface recipient count; got: {result.output!r}"
    )


def test_broadcast_default_echo__matches_canonical_summary_pattern(
    runner, session_id, agent_id, monkeypatch
):
    """Per Step 6 spec: ``broadcast id=<id8> recipients=<count>``."""
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(
            summary_id="abcdef0123456789-tail", recipient_count=3
        ),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    # The canonical compact form mentioned in the Step 6 task body.
    assert "broadcast" in out
    assert "id=abcdef01" in out
    assert "recipients=3" in out


def test_broadcast_full_echo__multi_line_per_recipient_envelopes(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_result(
            summary_id="abcdef0123456789-tail", recipient_count=3
        ),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            "hello",
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    # ``--full`` should expand into the legacy verbose layout. Look for
    # canonical legacy field labels.
    for needle in ("from:", "type:", "text:"):
        assert needle in result.output, (
            f"--full broadcast echo should include legacy field label "
            f"{needle!r}; got:\n{result.output}"
        )


# ---------------------------------------------------------------------------
# 2. --quiet on message send
# ---------------------------------------------------------------------------


def test_message_send_quiet__emits_only_task_id_8(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _typed_task(task_id="abcdef0123456789-tail", text="hello"),
            "notification_sent": True,
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "send",
            "--agent-id",
            agent_id,
            "--to",
            str(uuid.uuid4()),
            "--text",
            "hello",
            "--quiet",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    assert out == "abcdef01", (
        f"--quiet should emit only the 8-char task id; got: {result.output!r}"
    )


def test_message_send_default__multi_line_echo_present(
    runner, session_id, agent_id, monkeypatch
):
    """Without ``--quiet`` the verbose echo persists (regression guard)."""
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _typed_task(task_id="abcdef0123456789-tail", text="hello"),
            "notification_sent": True,
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "send",
            "--agent-id",
            agent_id,
            "--to",
            str(uuid.uuid4()),
            "--text",
            "hello",
        ],
    )
    assert result.exit_code == 0, result.output
    assert result.output.strip() != "abcdef01"
    assert "Message sent" in result.output


# ---------------------------------------------------------------------------
# 3. --quiet on message ack
# ---------------------------------------------------------------------------


def test_message_ack_quiet__emits_only_task_id_8(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "ack_task",
        lambda *_a, **_k: {
            "task": _typed_task(
                task_id="abcdef0123456789-tail",
                text="hello",
                status_state="completed",
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "ack",
            "--agent-id",
            agent_id,
            "--task-id",
            "abcdef0123456789-tail",
            "--quiet",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    assert out == "abcdef01", (
        f"--quiet should emit only the 8-char task id; got: {result.output!r}"
    )


def test_message_ack_default__multi_line_echo_present(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "ack_task",
        lambda *_a, **_k: {
            "task": _typed_task(
                task_id="abcdef0123456789-tail",
                text="hello",
                status_state="completed",
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "ack",
            "--agent-id",
            agent_id,
            "--task-id",
            "abcdef0123456789-tail",
        ],
    )
    assert result.exit_code == 0, result.output
    assert result.output.strip() != "abcdef01"
    assert "acknowledged" in result.output.lower()


# ---------------------------------------------------------------------------
# 4. --quiet on member ping
# ---------------------------------------------------------------------------


def test_member_ping_quiet__emits_single_short_line(runner, session_id, monkeypatch):
    """``--quiet`` collapses the multi-line ``Pinged member …`` echo to a
    single short line. We don't pin the exact content (no task id is created
    by ping) but we do require a single line with much less text than the
    default echo."""

    def _placement(**overrides):
        return {
            "director_agent_id": DIRECTOR_ID,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": PANE_ID,
            "coding_agent": "claude",
            "created_at": "2026-04-16T08:00:00+00:00",
            **overrides,
        }

    def _agent(**overrides):
        return {
            "agent_id": MEMBER_ID,
            "name": "Claude-B",
            "description": "Test member",
            "status": "active",
            "registered_at": "2026-04-16T08:00:00+00:00",
            "kind": "user",
            "placement": _placement(),
            **overrides,
        }

    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_k: _agent())
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "send_poll_trigger", lambda **_k: True, raising=False)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "ping",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            "--quiet",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output.strip()
    assert "\n" not in out, (
        f"--quiet member ping must be a single line; got:\n{result.output}"
    )
    # The default echo carries words like "Pinged" / "dispatched"; --quiet should not.
    assert "dispatched" not in out
    assert "Pinged" not in out


def test_member_ping_default__multi_word_echo_present(runner, session_id, monkeypatch):
    def _placement(**overrides):
        return {
            "director_agent_id": DIRECTOR_ID,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": PANE_ID,
            "coding_agent": "claude",
            "created_at": "2026-04-16T08:00:00+00:00",
            **overrides,
        }

    def _agent(**overrides):
        return {
            "agent_id": MEMBER_ID,
            "name": "Claude-B",
            "description": "Test member",
            "status": "active",
            "registered_at": "2026-04-16T08:00:00+00:00",
            "kind": "user",
            "placement": _placement(),
            **overrides,
        }

    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_k: _agent())
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "send_poll_trigger", lambda **_k: True, raising=False)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "ping",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
        ],
    )
    assert result.exit_code == 0, result.output
    # Default echo already contains "Pinged member" per the existing helper.
    assert "Pinged" in result.output
