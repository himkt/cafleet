"""Tests for server.py — _handle_send_message() response format.

Design doc 0000020 Step 4: notification_sent (unicast) and
notifications_sent_count (broadcast) extracted from task metadata
and included at the response top level.
"""

import uuid
from datetime import UTC, datetime
from unittest.mock import patch

import pytest
from httpx import ASGITransport, AsyncClient
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from cafleet.db.models import Session as SessionModel
from cafleet.models import PlacementCreate
from cafleet.registry_store import RegistryStore
from cafleet.server import create_app


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _create_test_session(
    db_sessionmaker: async_sessionmaker[AsyncSession],
    *,
    session_id: str | None = None,
) -> str:
    if session_id is None:
        session_id = str(uuid.uuid4())
    created_at = datetime.now(UTC).isoformat()
    async with db_sessionmaker() as session:
        async with session.begin():
            session.add(
                SessionModel(
                    session_id=session_id, label=None, created_at=created_at
                )
            )
    return session_id


def _auth(session_id: str, agent_id: str) -> dict:
    return {"X-Session-Id": session_id, "X-Agent-Id": agent_id}


def _send_payload(destination: str, text: str = "Hello") -> dict:
    return {
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": str(uuid.uuid4()),
                "role": "user",
                "parts": [{"kind": "text", "text": text}],
                "metadata": {"destination": destination},
            },
        },
        "id": str(uuid.uuid4()),
    }


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
async def notify_server_env(db_sessionmaker, store: RegistryStore):
    """ASGI app with agents that have varying placement states.

    - sender: plain agent (no placement)
    - recipient_with_pane: has placement with tmux_pane_id="%7"
    - recipient_no_placement: no placement row
    """
    session_id = await _create_test_session(db_sessionmaker)
    app = create_app(sessionmaker=db_sessionmaker)
    transport = ASGITransport(app=app)

    sender = await store.create_agent(
        name="Sender",
        description="Sender agent",
        session_id=session_id,
    )

    recipient_with_pane = await store.create_agent_with_placement(
        name="Recipient-Pane",
        description="Has pane",
        session_id=session_id,
        placement=PlacementCreate(
            director_agent_id=sender["agent_id"],
            tmux_session="main",
            tmux_window_id="@0",
            tmux_pane_id="%7",
        ),
    )

    recipient_no_placement = await store.create_agent(
        name="Recipient-NoPl",
        description="No placement",
        session_id=session_id,
    )

    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "session_id": session_id,
            "sender": sender,
            "recipient_with_pane": recipient_with_pane,
            "recipient_no_placement": recipient_no_placement,
        }


# ---------------------------------------------------------------------------
# Unicast response format
# ---------------------------------------------------------------------------


class TestUnicastResponseFormat:
    """Verify _handle_send_message() includes notification_sent at top level."""

    async def test_notification_sent_true_at_top_level(self, notify_server_env):
        """Unicast to agent with pane → response has notification_sent=true."""
        env = notify_server_env

        with patch("cafleet.tmux.send_poll_trigger", return_value=True):
            resp = await env["client"].post(
                "/",
                json=_send_payload(env["recipient_with_pane"]["agent_id"]),
                headers=_auth(env["session_id"], env["sender"]["agent_id"]),
            )

        assert resp.status_code == 200
        data = resp.json()
        result = data["result"]
        assert "task" in result
        assert result["notification_sent"] is True

    async def test_notification_sent_false_at_top_level(self, notify_server_env):
        """Unicast to agent without placement → response has notification_sent=false."""
        env = notify_server_env

        with patch("cafleet.tmux.send_poll_trigger", return_value=True):
            resp = await env["client"].post(
                "/",
                json=_send_payload(env["recipient_no_placement"]["agent_id"]),
                headers=_auth(env["session_id"], env["sender"]["agent_id"]),
            )

        assert resp.status_code == 200
        data = resp.json()
        result = data["result"]
        assert "task" in result
        assert result["notification_sent"] is False


# ---------------------------------------------------------------------------
# Broadcast response format
# ---------------------------------------------------------------------------


class TestBroadcastResponseFormat:
    """Verify _handle_send_message() includes notifications_sent_count at top level."""

    async def test_notifications_sent_count_at_top_level(self, notify_server_env):
        """Broadcast → response has notifications_sent_count with correct count.

        Env has 3 agents: sender (excluded), recipient_with_pane (notified=1),
        recipient_no_placement (not notified). Expected count: 1.
        """
        env = notify_server_env

        with patch("cafleet.tmux.send_poll_trigger", return_value=True):
            resp = await env["client"].post(
                "/",
                json=_send_payload("*"),
                headers=_auth(env["session_id"], env["sender"]["agent_id"]),
            )

        assert resp.status_code == 200
        data = resp.json()
        result = data["result"]
        assert "task" in result
        assert result["notifications_sent_count"] == 1


# ---------------------------------------------------------------------------
# Backward compatibility
# ---------------------------------------------------------------------------


class TestResponseBackwardCompatibility:
    """Verify responses without notification metadata omit notification fields.

    ACK (multi-turn SendMessage) produces a completed task via _handle_ack(),
    which does NOT add notification metadata. The server's _handle_send_message()
    should omit notification_sent / notifications_sent_count from the response.
    """

    async def test_ack_response_omits_notification_fields(self, notify_server_env):
        """ACK response has no notification_sent or notifications_sent_count."""
        env = notify_server_env

        # First send a unicast to create a task
        with patch("cafleet.tmux.send_poll_trigger", return_value=False):
            send_resp = await env["client"].post(
                "/",
                json=_send_payload(env["recipient_no_placement"]["agent_id"]),
                headers=_auth(env["session_id"], env["sender"]["agent_id"]),
            )
        assert send_resp.status_code == 200
        task_id = send_resp.json()["result"]["task"]["id"]

        # ACK the task (multi-turn SendMessage with taskId, no destination)
        ack_payload = {
            "jsonrpc": "2.0",
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": str(uuid.uuid4()),
                    "role": "user",
                    "taskId": task_id,
                    "parts": [{"kind": "text", "text": "ack"}],
                },
            },
            "id": str(uuid.uuid4()),
        }
        ack_resp = await env["client"].post(
            "/",
            json=ack_payload,
            headers=_auth(
                env["session_id"], env["recipient_no_placement"]["agent_id"]
            ),
        )

        assert ack_resp.status_code == 200
        data = ack_resp.json()
        result = data["result"]
        assert "task" in result
        assert result["task"]["status"]["state"] == "completed"
        assert "notification_sent" not in result
        assert "notifications_sent_count" not in result
