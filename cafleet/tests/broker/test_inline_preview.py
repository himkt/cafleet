"""Broker-side wiring of ``send_inline_preview``."""

import pytest

from cafleet import broker
from cafleet.multiplexer import tmux as multiplexer_tmux
from cafleet.multiplexer.tmux import TmuxMultiplexer
from tests.broker._helpers import _create_fleet, _member_placement


@pytest.fixture
def inline_preview_calls(monkeypatch):
    captured: list[dict] = []

    def stub(self, *args, **kwargs):
        captured.append({"args": args, "kwargs": kwargs})
        return True

    monkeypatch.setattr(TmuxMultiplexer, "send_inline_preview", stub)
    return captured


@pytest.fixture
def poll_trigger_call_count(monkeypatch):
    counter = {"n": 0}

    def stub(self, *_args, **_kwargs):
        counter["n"] += 1
        return True

    monkeypatch.setattr(TmuxMultiplexer, "send_poll_trigger", stub)
    return counter


def _register_placed_member(fleet_id, name, pane):
    return broker.register_member(
        fleet_id=fleet_id,
        name=name,
        description=f"{name} description",
        placement=_member_placement(pane),
    )


def _setup_two_members():
    s = _create_fleet()
    sid = s["fleet_id"]
    sender = _register_placed_member(sid, "sender", "%1")
    recipient = _register_placed_member(sid, "recipient", "%2")
    return sid, sender["member_id"], recipient["member_id"]


def test_send_message__auto_fire_invokes_inline_preview_with_full_kwargs(
    inline_preview_calls,
):
    sid, sender, recipient = _setup_two_members()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    assert len(inline_preview_calls) == 1
    call = inline_preview_calls[0]
    kwargs = call["kwargs"]
    pane_id = kwargs.get("target_pane_id") or (
        call["args"][0] if call["args"] else None
    )
    assert isinstance(pane_id, str)
    assert pane_id
    assert kwargs.get("message_id") == result["message"]["message_id"]
    assert kwargs.get("sender_id") == sender
    assert kwargs.get("ts") == result["message"]["status_timestamp"]
    assert kwargs.get("text") == "Did the API change?"


def test_send_message__sequential_sends_produce_distinct_previews_no_poll_trigger(
    inline_preview_calls,
    poll_trigger_call_count,
):
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")
    broker.send_message(sid, sender, recipient, "msg3")
    assert len(inline_preview_calls) == 3
    assert poll_trigger_call_count["n"] == 0
    message_ids = [c["kwargs"]["message_id"] for c in inline_preview_calls]
    assert len(set(message_ids)) == 3
    texts = [c["kwargs"]["text"] for c in inline_preview_calls]
    assert texts == ["msg1", "msg2", "msg3"]


def test_inline_preview_failure__message_persisted_and_notification_flag_false(
    monkeypatch, poll_trigger_call_count
):
    monkeypatch.setattr(
        TmuxMultiplexer, "send_inline_preview", lambda self, *_a, **_k: False
    )
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "delivered despite tmux down")
    assert sent["notification_sent"] is False
    [polled] = broker.poll_messages(recipient)
    assert polled["message_id"] == sent["message"]["message_id"]
    assert polled["text"] == "delivered despite tmux down"
    # Must NOT fall back to send_poll_trigger.
    assert poll_trigger_call_count["n"] == 0


def test_inline_preview_failure__subsequent_sends_still_attempt_preview(
    monkeypatch, poll_trigger_call_count
):
    attempts: list[bool] = []

    def stub(self, *_a, **_k):
        attempts.append(True)
        return False

    monkeypatch.setattr(TmuxMultiplexer, "send_inline_preview", stub)
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    broker.send_message(sid, sender, recipient, "third")
    assert len(attempts) == 3
    assert poll_trigger_call_count["n"] == 0


@pytest.mark.parametrize(
    "scenario",
    ["self_send", "recipient_without_placement"],
)
def test_send_message__skip_conditions(inline_preview_calls, scenario):
    if scenario == "self_send":
        sid, sender, _recipient = _setup_two_members()
        broker.send_message(sid, sender, sender, "self")
    else:
        s = _create_fleet()
        sid = s["fleet_id"]
        sender = _register_placed_member(sid, "sender", "%1")
        recipient = broker.register_member(
            fleet_id=sid,
            name="lonely",
            description="no placement",
        )
        sent = broker.send_message(
            sid,
            sender["member_id"],
            recipient["member_id"],
            "to placement-less peer",
        )
        # Message still landed in queue.
        [polled] = broker.poll_messages(recipient["member_id"])
        assert polled["message_id"] == sent["message"]["message_id"]
    # In both scenarios the preview is skipped.
    assert inline_preview_calls == []


def test_send_message__auto_fire_never_calls_send_poll_trigger_on_success(
    inline_preview_calls,
    poll_trigger_call_count,
):
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "hello")
    assert poll_trigger_call_count["n"] == 0


def test_send_message__real_inline_preview_keystroke_is_esc_first(monkeypatch):
    """End-to-end through ``broker.send_message`` (design 0000092 §1): with the
    REAL ``send_inline_preview`` helper in play (no method-level stub), the
    keystroke delivered to the recipient's pane leads with `Escape` so a
    recipient parked on a pending permission-approval prompt dismisses it before
    any payload character is typed and the trailing Enter can never confirm it.

    Guards that the §1 hardening reaches every ``message send`` recipient over
    the broker path — not just at the unit-level helper. This is exactly the
    Background incident's failure path (a nudge confirming the Director's
    ``git push`` prompt), now closed."""
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    captured: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )

    sid, sender, recipient = _setup_two_members()  # recipient pane is "%2"
    result = broker.send_message(sid, sender, recipient, "cancel it (Esc)")
    assert result["notification_sent"] is True

    # Only the recipient's pane (%2) is keystroked; it receives Escape FIRST,
    # then the literal payload, then the trailing Enter submit.
    pane_calls = [argv for argv in captured if argv[3:4] == ["%2"]]
    assert pane_calls[0] == ["tmux", "send-keys", "-t", "%2", "Escape"]
    assert pane_calls[1][:5] == ["tmux", "send-keys", "-t", "%2", "-l"]
    assert pane_calls[-1] == ["tmux", "send-keys", "-t", "%2", "Enter"]
