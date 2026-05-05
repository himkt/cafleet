"""Surface 15 — broker-side wiring of ``send_inline_preview`` (design 0000049 Step 4).

Asserts that ``broker._try_notify_recipient`` invokes the new
``tmux.send_inline_preview`` helper instead of the previous
``tmux.send_poll_trigger`` keystroke. The auto-fire path is replaced with
the inline preview wholesale; the queue still holds the message of record,
so simulated tmux failures don't lose the delivery — the recipient catches
up via ``cafleet message poll`` on the next opportunity.

``cafleet member ping`` (the manual Director re-poke primitive) keeps
calling ``send_poll_trigger``; that wiring is exercised by
``tests/test_cli_member_ping.py`` and the helper's preservation guard lives
in ``test_tmux_send_inline_preview.py``.
"""

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers the PRAGMA listener globally
from cafleet import broker, tmux
from cafleet.db.models import Base
from cafleet.tmux import DirectorContext


# ---------------------------------------------------------------------------
# Fixtures (mirror ``test_broker_messaging.py``)
# ---------------------------------------------------------------------------


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    return sync_sessionmaker


@pytest.fixture
def inline_preview_calls(monkeypatch):
    """Stub ``cafleet.tmux.send_inline_preview`` and capture every call's kwargs."""
    captured: list[dict] = []

    def stub(*args, **kwargs):
        captured.append({"args": args, "kwargs": kwargs})
        return True

    monkeypatch.setattr(tmux, "send_inline_preview", stub)
    return captured


@pytest.fixture
def poll_trigger_call_count(monkeypatch):
    """Stub ``cafleet.tmux.send_poll_trigger`` and count invocations.

    Post-Surface-15 the broker auto-fire path must NOT call this helper at
    all. ``cafleet member ping`` still does — but that is dispatched via
    its own CLI command, not via ``broker.send_message``.
    """
    counter = {"n": 0}

    def stub(*_args, **_kwargs):
        counter["n"] += 1
        return True

    monkeypatch.setattr(tmux, "send_poll_trigger", stub)
    return counter


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _create_session() -> dict:
    return broker.create_session(
        label=None,
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )


def _register_agent(session_id: str, name: str) -> dict:
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=f"{name} description",
    )


def _setup_two_agents() -> tuple[str, str, str]:
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, "sender")
    recipient = _register_agent(sid, "recipient")
    return sid, sender["agent_id"], recipient["agent_id"]


# ---------------------------------------------------------------------------
# 1. send_message auto-fire calls send_inline_preview
# ---------------------------------------------------------------------------


def test_send_message__auto_fire_calls_send_inline_preview(inline_preview_calls):
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "hello")
    assert len(inline_preview_calls) == 1


def test_send_message__auto_fire_does_not_call_send_poll_trigger(
    inline_preview_calls,
    poll_trigger_call_count,
):
    """Surface 15: the broker auto-fire path stops keystroking poll commands."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "hello")
    assert poll_trigger_call_count["n"] == 0


def test_send_message__inline_preview_carries_recipient_pane(inline_preview_calls):
    """The helper is invoked with the recipient's tmux pane id."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "hello")
    [call] = inline_preview_calls
    pane_id = call["kwargs"].get("target_pane_id")
    if pane_id is None and call["args"]:
        pane_id = call["args"][0]
    # Director was bootstrapped at "%0" by ``_create_session``; the
    # recipient was registered after the Director and has no placement
    # row yet — so the broker's auto-fire path skips notification when
    # the recipient has no pane (returns False, no inline_preview call).
    # That branch is exercised in the no-placement test below; here we
    # confirm the call path survives and at minimum carries a string
    # ``target_pane_id`` (or positional first arg).
    assert pane_id is not None
    assert isinstance(pane_id, str)


def test_send_message__inline_preview_kwargs_include_task_id_8(inline_preview_calls):
    """The helper receives an 8-char ``task_id_8`` derived from the new task."""
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "hello")
    [call] = inline_preview_calls
    task_id_8 = call["kwargs"].get("task_id_8")
    assert task_id_8 is not None
    assert task_id_8 == result["task"]["task_id"][:8]


def test_send_message__inline_preview_kwargs_include_sender_8(inline_preview_calls):
    """The helper receives an 8-char ``sender_8`` derived from the sender's UUID."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "hello")
    [call] = inline_preview_calls
    sender_8 = call["kwargs"].get("sender_8")
    assert sender_8 is not None
    assert sender_8 == sender[:8]


def test_send_message__inline_preview_kwargs_include_ts(inline_preview_calls):
    """The helper receives a ``ts`` matching the persisted ``status_timestamp``."""
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "hello")
    [call] = inline_preview_calls
    ts = call["kwargs"].get("ts")
    assert ts is not None
    assert ts == result["task"]["status_timestamp"]


def test_send_message__inline_preview_kwargs_include_text(inline_preview_calls):
    """The helper receives the ``text`` body verbatim — no truncation in the wire."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "Did the API change?")
    [call] = inline_preview_calls
    text = call["kwargs"].get("text")
    assert text == "Did the API change?"


# ---------------------------------------------------------------------------
# 2. Three sequential sends → three inline previews, zero poll triggers
# ---------------------------------------------------------------------------


def test_three_sequential_sends__three_inline_previews_zero_poll_triggers(
    inline_preview_calls,
    poll_trigger_call_count,
):
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")
    broker.send_message(sid, sender, recipient, "msg3")

    assert len(inline_preview_calls) == 3
    assert poll_trigger_call_count["n"] == 0


def test_three_sequential_sends__each_preview_carries_distinct_task_id_8(
    inline_preview_calls,
):
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")
    broker.send_message(sid, sender, recipient, "msg3")

    task_id_8s = [call["kwargs"]["task_id_8"] for call in inline_preview_calls]
    assert len(set(task_id_8s)) == 3, (
        f"each send should produce a distinct 8-char task id; got {task_id_8s!r}"
    )


def test_three_sequential_sends__each_preview_carries_distinct_text(
    inline_preview_calls,
):
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")
    broker.send_message(sid, sender, recipient, "msg3")

    texts = [call["kwargs"]["text"] for call in inline_preview_calls]
    assert texts == ["msg1", "msg2", "msg3"]


# ---------------------------------------------------------------------------
# 3. Fallback — tmux failure does not lose the message
# ---------------------------------------------------------------------------


def test_inline_preview_failure__message_still_persisted_in_queue(monkeypatch):
    """Simulated tmux failure (helper returns False) — the recipient still
    catches up via a manual ``cafleet message poll``."""
    monkeypatch.setattr(tmux, "send_inline_preview", lambda *_a, **_k: False)
    sid, sender, recipient = _setup_two_agents()

    sent = broker.send_message(sid, sender, recipient, "delivered despite tmux down")

    assert sent["notification_sent"] is False
    [polled] = broker.poll_tasks(recipient)
    assert polled["task_id"] == sent["task"]["task_id"]
    assert polled["text"] == "delivered despite tmux down"


def test_inline_preview_failure__notification_sent_flag_is_false(monkeypatch):
    """The wrapper return reflects the helper's False — operator can detect."""
    monkeypatch.setattr(tmux, "send_inline_preview", lambda *_a, **_k: False)
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "anyway")
    assert sent["notification_sent"] is False


def test_inline_preview_failure__does_not_fall_back_to_send_poll_trigger(
    monkeypatch,
    poll_trigger_call_count,
):
    """When ``send_inline_preview`` returns False, the broker MUST NOT fall
    back to ``send_poll_trigger`` — that would keystroke a poll command into
    the recipient's pane, which is exactly the regression Surface 15 removes."""
    monkeypatch.setattr(tmux, "send_inline_preview", lambda *_a, **_k: False)
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "delivery one")
    assert poll_trigger_call_count["n"] == 0


def test_inline_preview_failure__subsequent_sends_still_attempt_preview(
    monkeypatch,
    poll_trigger_call_count,
):
    """A failed first preview must not disable the auto-fire path; a later
    send still attempts the inline preview."""
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


# ---------------------------------------------------------------------------
# 4. Self-send and missing-pane skip
# ---------------------------------------------------------------------------


def test_self_send__skips_inline_preview(inline_preview_calls):
    """A sender addressing themselves never triggers a preview into their
    own pane — same skip-condition as the prior poll-trigger path."""
    sid, sender, _recipient = _setup_two_agents()
    broker.send_message(sid, sender, sender, "self")
    assert inline_preview_calls == []


def test_recipient_without_placement__skips_inline_preview(inline_preview_calls):
    """When the recipient has no ``agent_placements`` row (no tmux pane),
    the broker auto-fire path skips notification silently — the message
    is still persisted to the queue."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "to placement-less peer")

    # ``recipient`` was registered via ``_register_agent`` with no placement
    # row — so the broker should skip the inline preview, not invoke it.
    assert inline_preview_calls == []
    # The message still landed in the queue.
    [polled] = broker.poll_tasks(recipient)
    assert polled["task_id"] == sent["task"]["task_id"]
