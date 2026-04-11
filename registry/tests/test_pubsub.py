"""Tests for the in-process ``PubSubManager``.

Replaces the fakeredis-based test suite with one that targets the new
``asyncio.Queue``-backed fan-out described in design doc
§"In-Process Pub/Sub Design"
(design-docs/0000010-sqlite-store-migration/design-doc.md).

Two regression tests are pulled verbatim from the doc's Testing
Strategy table (design-doc.md line 557-558):

  | Test                                        | Verifies                |
  |---------------------------------------------|-------------------------|
  | ``test_pubsub_fanout_two_subscribers``      | Two subscribers on the  |
  |                                             | same channel both       |
  |                                             | receive a published msg |
  | ``test_pubsub_unsubscribe_releases_queue``  | After unsubscribe the   |
  |                                             | subscriber's queue is   |
  |                                             | detached from publish   |

The full public API the Programmer will expose in Phase B:

    PubSubManager()                              # no-arg constructor
    await manager.publish(channel, message)      # fan-out to all queues
    await manager.subscribe(channel)             # -> _Subscription
    await manager.unsubscribe(channel, sub)      # detach by _Subscription

The ``unsubscribe`` second arg is a **_Subscription**, not a bare
queue — the caller never holds a raw queue reference. After
``unsubscribe``, the iterator's next ``__anext__`` either raises
``StopAsyncIteration`` (clean terminate) or blocks/times-out (idle
queue). Either outcome is acceptable per the doc — the invariant that
matters is that the detached subscriber must NEVER yield a message
published after the unsubscribe call.

Timeouts everywhere
-------------------

Every ``__anext__`` call is wrapped in ``asyncio.wait_for``. The new
``asyncio.Queue.get()`` blocks indefinitely if no message arrives, so
a broken fan-out would hang the whole suite without wait_for. One
second is plenty for an in-process put/get round-trip.
"""

import asyncio
import inspect

import pytest

from hikyaku_registry.pubsub import PubSubManager


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------


@pytest.fixture
def manager() -> PubSubManager:
    """A fresh ``PubSubManager`` per test.

    Sync fixture because the constructor is no-arg and synchronous.
    Function-scoped so tests are fully isolated — no cross-test state
    leakage through a shared ``_subscribers`` dict.
    """
    return PubSubManager()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _next_msg(subscription, timeout: float = 1.0) -> str:
    """Pull one message off a subscription with a hard timeout.

    Wraps ``__anext__`` in ``asyncio.wait_for`` so a broken fan-out
    fails with ``TimeoutError`` rather than hanging the suite.
    """
    return await asyncio.wait_for(subscription.__anext__(), timeout=timeout)


# ---------------------------------------------------------------------------
# Constructor
# ---------------------------------------------------------------------------


def test_manager_no_args_constructor():
    """``PubSubManager()`` accepts zero arguments.

    The Redis-backed predecessor took ``PubSubManager(redis)``; the
    new in-process version owns its state and needs nothing from the
    caller. This test guards against an accidental re-introduction of
    a required parameter.
    """
    manager = PubSubManager()
    assert manager is not None


def test_constructor_signature_has_no_required_params():
    """``__init__`` signature has no required params beyond ``self``.

    Stronger than the call-site check above: introspects the signature
    so that even an optional-but-renamed parameter (e.g. ``def __init__
    (self, redis=None)``) would pass the no-args call but still be
    visible here as a design drift.
    """
    sig = inspect.signature(PubSubManager.__init__)
    required = [
        p
        for p in sig.parameters.values()
        if p.name != "self"
        and p.default is inspect.Parameter.empty
        and p.kind
        in (
            inspect.Parameter.POSITIONAL_ONLY,
            inspect.Parameter.POSITIONAL_OR_KEYWORD,
            inspect.Parameter.KEYWORD_ONLY,
        )
    ]
    assert required == [], (
        f"PubSubManager.__init__ must accept no required args; "
        f"got required params: {[p.name for p in required]}"
    )


def test_multiple_managers_are_independent():
    """Two ``PubSubManager`` instances do not share state.

    Critical for test isolation: if two managers ever shared a
    class-level ``_subscribers`` dict (e.g., accidentally defined
    as a class attribute instead of an instance attribute), a
    subscribe on manager A would leak into manager B and every test
    would become order-dependent. This test catches that at structure
    level.
    """
    a = PubSubManager()
    b = PubSubManager()
    assert a is not b
    assert a._subscribers is not b._subscribers, (
        "two PubSubManager instances must own independent _subscribers "
        "dicts (did you define _subscribers as a class attribute?)"
    )


# ---------------------------------------------------------------------------
# publish — no-subscribers no-op
# ---------------------------------------------------------------------------


async def test_publish_without_subscribers_is_noop(manager):
    """Publishing on a channel with zero subscribers is a silent no-op.

    Matches Redis ``PUBLISH`` semantics (returns 0, no exception). The
    SSE endpoint's finally-block cleanup ordering means the server can
    publish after the last subscriber has already unsubscribed; that
    must not raise or the error log will fill up with benign races.
    """
    await manager.publish("inbox:nobody-home", "task-drop")


# ---------------------------------------------------------------------------
# Fan-out — design-doc regression test
# ---------------------------------------------------------------------------


async def test_pubsub_fanout_two_subscribers(manager):
    """Two subscribers on the same channel each receive every published msg.

    This is the exact regression test named in the design doc Testing
    Strategy table (design-doc.md line 557). The in-process fan-out
    must iterate over ALL queues registered under a given channel and
    put the message on each. A single-queue-per-channel optimization
    would break this test because each message would be delivered to
    only one subscriber.

    Uses ``asyncio.gather`` over per-subscriber ``collect`` coroutines
    so both subscribers drain concurrently. This flushes out any
    accidental serialization between publish and fan-out
    (e.g., a lock that forces subscribers to drain one-at-a-time).
    """
    channel = "inbox:agent-x"
    sub1 = await manager.subscribe(channel)
    sub2 = await manager.subscribe(channel)

    for msg_id in ["t-1", "t-2", "t-3"]:
        await manager.publish(channel, msg_id)

    async def collect(sub):
        return [await _next_msg(sub) for _ in range(3)]

    received1, received2 = await asyncio.gather(collect(sub1), collect(sub2))

    assert received1 == ["t-1", "t-2", "t-3"], (
        f"first subscriber should receive all 3 messages in order; got {received1}"
    )
    assert received2 == ["t-1", "t-2", "t-3"], (
        f"second subscriber should receive all 3 messages in order; got {received2}"
    )


# ---------------------------------------------------------------------------
# Unsubscribe — design-doc regression test
# ---------------------------------------------------------------------------


async def test_pubsub_unsubscribe_releases_queue(manager):
    """After unsubscribe, the subscriber is detached from the publish path.

    The exact regression test named in the design doc Testing Strategy
    table (design-doc.md line 558). Asserts the two invariants that
    together define the unsubscribe contract:

      1. Behavioral: a message published AFTER unsubscribe must not
         reach the detached subscriber. If it does, the subscription
         is still on the channel's publish list → memory leak + wrong
         delivery.
      2. Termination: the iterator does not deadlock. Either
         ``StopAsyncIteration`` is raised (clean terminate) or the
         next ``__anext__`` just times out (idle queue). Both are
         acceptable per the design doc — the invariant is "no
         deadlock + no stale delivery".

    If this test fails with the iterator yielding ``"t-2-after-unsub"``,
    ``unsubscribe`` is not actually removing the queue from
    ``_subscribers[channel]`` — the subscriber is still being fanned
    out to.
    """
    channel = "inbox:unsub-release"
    sub = await manager.subscribe(channel)

    await manager.publish(channel, "t-1")
    first = await _next_msg(sub)
    assert first == "t-1"

    await manager.unsubscribe(channel, sub)

    await manager.publish(channel, "t-2-after-unsub")

    with pytest.raises((StopAsyncIteration, asyncio.TimeoutError)):
        await asyncio.wait_for(sub.__anext__(), timeout=0.3)


async def test_unsubscribe_of_nonexistent_subscription_is_noop(manager):
    """Unsubscribing a subscription that's already detached must not raise.

    Two scenarios this guards against:

      1. Double-unsubscribe: the SSE handler's finally-block may run
         after an upstream error already cleaned up. Calling
         ``unsubscribe`` twice with the same subscription must be a
         no-op.
      2. Channel never had this subscription: a defensive safety net
         for racy cleanup paths that might pass a stale reference.

    Either case must silently succeed — not raise ``KeyError`` or
    ``ValueError``.
    """
    channel = "inbox:double-unsub"
    sub = await manager.subscribe(channel)
    await manager.unsubscribe(channel, sub)
    # Second call on the same subscription — must be a no-op.
    await manager.unsubscribe(channel, sub)

    # Also: unsubscribe a subscription from a channel the manager has
    # never heard of.
    other_sub = await manager.subscribe("inbox:known")
    await manager.unsubscribe("inbox:never-seen", other_sub)


# ---------------------------------------------------------------------------
# Delivery ordering
# ---------------------------------------------------------------------------


async def test_fifo_order_within_single_subscriber(manager):
    """Messages are delivered in publish order to a single subscriber.

    ``asyncio.Queue`` is FIFO by default; this test guards against an
    accidental switch to ``LifoQueue`` or ``PriorityQueue`` and against
    any ordering bug in the publish-side iteration over subscriber
    queues.
    """
    channel = "inbox:fifo"
    sub = await manager.subscribe(channel)

    for i in range(5):
        await manager.publish(channel, f"task-{i}")

    received = [await _next_msg(sub) for _ in range(5)]
    assert received == [
        "task-0",
        "task-1",
        "task-2",
        "task-3",
        "task-4",
    ]


# ---------------------------------------------------------------------------
# Channel isolation
# ---------------------------------------------------------------------------


async def test_different_channels_are_isolated(manager):
    """A publish on channel A must not reach a subscriber on channel B.

    Without this invariant the inbox model collapses — every agent
    would see every other agent's messages. This test is the tenant
    firewall.
    """
    sub_b = await manager.subscribe("inbox:bbb")

    await manager.publish("inbox:aaa", "task-for-a")
    await manager.publish("inbox:bbb", "task-for-b")

    # The first (and only) message sub_b sees must be task-for-b.
    first = await _next_msg(sub_b)
    assert first == "task-for-b", (
        f"channel B subscriber must NOT see channel A messages; "
        f"first message was {first!r}"
    )


async def test_subscribe_to_multiple_channels(manager):
    """One caller can hold subscriptions on multiple channels independently.

    Each channel's subscription has its own queue, so messages on
    channel A do not appear in channel B's iterator (and vice versa).
    This complements ``test_different_channels_are_isolated`` by
    verifying from the subscriber side rather than the publisher side.
    """
    sub_inbox = await manager.subscribe("inbox:multi")
    sub_control = await manager.subscribe("control:multi")

    await manager.publish("inbox:multi", "inbox-payload")
    await manager.publish("control:multi", "control-payload")

    msg_inbox = await _next_msg(sub_inbox)
    msg_control = await _next_msg(sub_control)

    assert msg_inbox == "inbox-payload"
    assert msg_control == "control-payload"


# ---------------------------------------------------------------------------
# Private-state introspection for channel cleanup
# ---------------------------------------------------------------------------


async def test_unsubscribe_last_subscriber_drops_channel_entry(manager):
    """Unsubscribing the sole subscriber removes the channel key entirely.

    Memory hygiene: a long-running server that accumulates subscribe/
    unsubscribe cycles must not grow ``_subscribers`` without bound.
    The design doc's pseudocode (§"PubSubManager shape") explicitly
    pops the channel entry when its subscriber set becomes empty:

        if not subs:
            self._subscribers.pop(channel, None)

    Reads ``manager._subscribers`` as observable state — legitimate
    coupling because the dict type is pinned by the design doc
    contract ``dict[str, set[asyncio.Queue[str]]]``.
    """
    channel = "inbox:drop-entry"
    sub = await manager.subscribe(channel)
    assert channel in manager._subscribers

    await manager.unsubscribe(channel, sub)
    assert channel not in manager._subscribers, (
        f"channel entry must be dropped once the last subscriber leaves; "
        f"still present in {manager._subscribers}"
    )


async def test_unsubscribe_one_of_two_keeps_channel(manager):
    """Unsubscribing one of two subscribers keeps the channel + other sub alive.

    Complements the "drop empty set" assertion above: both together
    define the exact lifecycle of ``_subscribers[channel]``.
    Partial unsubscribe must not accidentally drop the whole channel
    entry — the surviving subscriber has to still receive new publishes.
    """
    channel = "inbox:partial-unsub"
    sub_drop = await manager.subscribe(channel)
    sub_keep = await manager.subscribe(channel)

    await manager.unsubscribe(channel, sub_drop)

    assert channel in manager._subscribers, (
        "channel entry must remain while at least one subscriber is active"
    )

    # The surviving subscriber must still receive new publishes.
    await manager.publish(channel, "task-after-partial-unsub")
    msg = await _next_msg(sub_keep)
    assert msg == "task-after-partial-unsub"
