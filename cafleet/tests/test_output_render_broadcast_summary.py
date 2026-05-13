"""Surface 4 — broadcast summary already omits ``recipient_ids`` (design 0000049 Step 7).

Post-Surface-14 the persisted broadcast-summary row dropped the
``recipient_ids`` field entirely. Two count signals remain:

* ``result["notifications_sent_count"]`` — wrapper-level integer returned
  by ``broker.broadcast_message`` for the "did it go out?" check.
* ``result["task"]["text"]`` — the broker persists
  ``"Broadcast sent to N recipients"`` directly into ``Task.text`` (see
  ``broker.broadcast_message`` ≈ line 723), so the human-readable count
  is on the typed-column row itself, not computed client-side.

Surface 4's "render-time omission" of ``recipient_ids`` is therefore a
no-op in practice — there is nothing to strip. These tests are a
forward-looking contract guard: if a future change re-introduces
``recipient_ids`` (or its legacy camelCase form) into the broadcast
summary, the regression must trip here. The ``--full`` recipient_ids
restoration is explicitly out of scope (the Director's clarification).
"""

import pytest

from cafleet import broker
from tests._broker_helpers import _create_session, _register_agent


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


# ---------------------------------------------------------------------------
# 1. broker.broadcast_message returns a summary with NO recipient_ids field
# ---------------------------------------------------------------------------


def test_broadcast_summary_task__has_no_recipient_ids_field():
    """The persisted broadcast-summary task dict must not carry
    ``recipient_ids``. Post-Surface-14 the typed-column shape excludes it
    entirely."""
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi all")
    summary = result["task"]
    assert "recipient_ids" not in summary, (
        f"broadcast summary should not carry recipient_ids; got keys: "
        f"{sorted(summary.keys())}"
    )


def test_broadcast_summary_task__has_no_recipientids_camelcase_either():  # noqa: N802 - asserts on the legacy camelCase key spelling
    """A defensive guard against the legacy camelCase form leaking back."""
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi")
    summary = result["task"]
    assert "recipientIds" not in summary
    # The legacy ``metadata`` wrapper would carry the camelCase form via the
    # pre-Surface-14 shape — assert that's gone too (already covered in
    # test_broker_typed_columns.py but worth a regression-anchor here).
    assert "metadata" not in summary


# ---------------------------------------------------------------------------
# 2. Wrapper-level count is the source of truth for "did it go out?"
# ---------------------------------------------------------------------------


def test_broadcast_message__wrapper_carries_notifications_sent_count():
    """The wrapper around the broadcast result includes the count for
    'did it go out?' verification — the consumer no longer needs
    recipient_ids since the count is exposed directly."""
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi")
    assert "notifications_sent_count" in result
    assert isinstance(result["notifications_sent_count"], int)


def test_broadcast_summary_text__describes_recipient_count_human_readably():
    """The human-readable summary text remains the canonical free-form
    description. This is the second source of count info (the first being
    the wrapper-level integer)."""
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi")
    summary = result["task"]
    assert summary["text"].startswith("Broadcast sent to ")
    assert "recipients" in summary["text"]


def test_broadcast_summary_task__keeps_only_typed_column_keys():
    """The summary task carries exactly the typed-column field set; no
    extra ``recipient_count`` / ``recipient_ids`` fields snuck back in."""
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi")
    summary = result["task"]
    expected_keys = {
        "task_id",
        "context_id",
        "from_agent_id",
        "to_agent_id",
        "type",
        "created_at",
        "status_state",
        "status_timestamp",
        "origin_task_id",
        "text",
    }
    extra = set(summary.keys()) - expected_keys
    assert not extra, (
        f"broadcast summary must carry exactly the typed-column keys; "
        f"unexpected extras: {sorted(extra)}"
    )
