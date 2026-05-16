"""Surface 15 — broker-side wiring of ``send_inline_preview`` (design 0000049 Step 4).

Per principle (ii)/(iii) of design 0000061: per-kwarg fragmented assertions
collapse into per-behaviour parametrized tests.
"""

import pytest

from cafleet import broker, tmux
from cafleet.tmux import DirectorContext


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


@pytest.fixture
def inline_preview_calls(monkeypatch):
    captured: list[dict] = []

    def stub(*args, **kwargs):
        captured.append({"args": args, "kwargs": kwargs})
        return True

    monkeypatch.setattr(tmux, "send_inline_preview", stub)
    return captured


@pytest.fixture
def poll_trigger_call_count(monkeypatch):
    counter = {"n": 0}

    def stub(*_args, **_kwargs):
        counter["n"] += 1
        return True

    monkeypatch.setattr(tmux, "send_poll_trigger", stub)
    return counter


def _create_session():
    return broker.create_session(
        label=None,
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )


def _register_member(session_id, name, director_id, pane):
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=f"{name} description",
        placement={
            "director_agent_id": director_id,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane,
            "coding_agent": "claude",
        },
    )


def _setup_two_agents():
    s = _create_session()
    sid = s["session_id"]
    director_id = s["director"]["agent_id"]
    sender = _register_member(sid, "sender", director_id, "%1")
    recipient = _register_member(sid, "recipient", director_id, "%2")
    return sid, sender["agent_id"], recipient["agent_id"]


def test_send_message__auto_fire_invokes_inline_preview_with_full_kwargs(
    inline_preview_calls,
):
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    assert len(inline_preview_calls) == 1
    call = inline_preview_calls[0]
    kwargs = call["kwargs"]
    pane_id = kwargs.get("target_pane_id") or (
        call["args"][0] if call["args"] else None
    )
    assert isinstance(pane_id, str)
    assert pane_id
    assert kwargs.get("task_id_8") == result["task"]["task_id"][:8]
    assert kwargs.get("sender_8") == sender[:8]
    assert kwargs.get("ts") == result["task"]["status_timestamp"]
    assert kwargs.get("text") == "Did the API change?"


def test_send_message__sequential_sends_produce_distinct_previews_no_poll_trigger(
    inline_preview_calls,
    poll_trigger_call_count,
):
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")
    broker.send_message(sid, sender, recipient, "msg3")
    assert len(inline_preview_calls) == 3
    assert poll_trigger_call_count["n"] == 0
    task_id_8s = [c["kwargs"]["task_id_8"] for c in inline_preview_calls]
    assert len(set(task_id_8s)) == 3
    texts = [c["kwargs"]["text"] for c in inline_preview_calls]
    assert texts == ["msg1", "msg2", "msg3"]


def test_inline_preview_failure__message_persisted_and_notification_flag_false(
    monkeypatch, poll_trigger_call_count
):
    monkeypatch.setattr(tmux, "send_inline_preview", lambda *_a, **_k: False)
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "delivered despite tmux down")
    assert sent["notification_sent"] is False
    [polled] = broker.poll_tasks(recipient)
    assert polled["task_id"] == sent["task"]["task_id"]
    assert polled["text"] == "delivered despite tmux down"
    # Must NOT fall back to send_poll_trigger.
    assert poll_trigger_call_count["n"] == 0


def test_inline_preview_failure__subsequent_sends_still_attempt_preview(
    monkeypatch, poll_trigger_call_count
):
    attempts: list[bool] = []

    def stub(*_a, **_k):
        attempts.append(True)
        return False

    monkeypatch.setattr(tmux, "send_inline_preview", stub)
    sid, sender, recipient = _setup_two_agents()
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
        sid, sender, _recipient = _setup_two_agents()
        broker.send_message(sid, sender, sender, "self")
    else:
        s = _create_session()
        sid = s["session_id"]
        director_id = s["director"]["agent_id"]
        sender = _register_member(sid, "sender", director_id, "%1")
        recipient = broker.register_agent(
            session_id=sid,
            name="lonely",
            description="no placement",
        )
        sent = broker.send_message(
            sid,
            sender["agent_id"],
            recipient["agent_id"],
            "to placement-less peer",
        )
        # Message still landed in queue.
        [polled] = broker.poll_tasks(recipient["agent_id"])
        assert polled["task_id"] == sent["task"]["task_id"]
    # In both scenarios the preview is skipped.
    assert inline_preview_calls == []


def test_send_message__auto_fire_never_calls_send_poll_trigger_on_success(
    inline_preview_calls,
    poll_trigger_call_count,
):
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "hello")
    assert poll_trigger_call_count["n"] == 0
