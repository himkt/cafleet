"""Tests for the in-process ``PubSubManager``.

Replaces the fakeredis-based test suite with one that targets the new
``asyncio.Queue``-backed fan-out described in design doc
§"In-Process Pub/Sub Design"
(design-docs/0000010-sqlite-store-migration/design-doc.md).

Two regression tests are pulled verbatim from the doc's Testing
Strategy table:

  | Test                                     | Verifies                |
  |------------------------------------------|-------------------------|
  | ``test_pubsub_fanout_two_subscribers``   | Two subscribers on the  |
  |                                          | same channel both       |
  |                                          | receive a published msg |
  | ``test_pubsub_unsubscribe_releases_queue`` | After unsubscribe the |
  |                                          | channel entry is        |
  |                                          | removed and a republish |
  |                                          | does not raise          |

The rest of the tests exercise the narrow public API the Programmer
will expose in Phase B:

  * ``PubSubManager()``                            — no-arg constructor
  * ``publish(channel, message) -> None``          — sync-ish fan-out
  * ``subscribe(channel) -> _Subscription``        — async iterator factory
  * ``unsubscribe(channel, queue) -> None``        — remove one queue

Why private state introspection is OK here
------------------------------------------

Two tests (``test_pubsub_unsubscribe_releases_queue`` and
``test_unsubscribe_one_of_two_keeps_channel``) read
``manager._subscribers`` directly to verify channel cleanup. This is an
intentional coupling to the doc-specified internal shape:

    self._subscribers: dict[str, set[asyncio.Queue[str]]]

The design doc fixes this dict type as part of the contract (§"PubSubManager
shape"), so treating it as observable state in tests is legitimate. The
same lookup is also used to recover a queue reference for the
``unsubscribe(channel, queue)`` call, because the ``_Subscription``
wrapper is specified only in terms of its async-iterator protocol —
there is no public ``.queue`` accessor guaranteed by the design doc.

Timeouts
--------

Every ``__anext__`` call is wrapped in ``asyncio.wait_for`` with a
1-second timeout. A hanging test would otherwise block forever if the
fan-out were broken (the old Redis version had a 1-second blocking
``get_message`` timeout that naturally unblocked; the new in-process
version uses ``queue.get()`` which blocks indefinitely).
"""

import asyncio
import inspect

import pytest

from hikyaku_registry.pubsub import PubSubManager


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _next_msg(subscription, timeout: float = 1.0) -> str:
    """Pull one message off a subscription with a hard timeout.

    Used instead of ``async for ... break`` so a broken fan-out fails
    fast with ``TimeoutError`` rather than hanging the whole suite.
    """
    return await asyncio.wait_for(subscription.__anext__(), timeout=timeout)


def _any_queue_for(manager: PubSubManager, channel: str):
    """Return an arbitrary queue registered under ``channel``.

    Used by unsubscribe tests to recover a queue reference for the
    doc-specified ``unsubscribe(channel, queue)`` signature. Treats
    ``manager._subscribers[channel]`` as read-only observable state.
    """
    return next(iter(manager._subscribers[channel]))


# ---------------------------------------------------------------------------
# Constructor
# ---------------------------------------------------------------------------


class TestConstructor:
    """The new constructor takes no arguments — no Redis client to inject."""

    def test_takes_no_arguments(self):
        """``PubSubManager()`` is callable with zero positional args.

        The old Redis-backed version took a ``redis`` client. The new
        in-process version owns an internal ``_subscribers`` dict and
        needs nothing from the caller. This test is a structural check:
        if the signature regresses (someone accidentally re-adds a
        required arg), construction raises and this test fails.
        """
        manager = PubSubManager()
        assert manager is not None

    def test_constructor_signature_has_only_self(self):
        """``__init__`` has no required parameters beyond ``self``."""
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


# ---------------------------------------------------------------------------
# publish
# ---------------------------------------------------------------------------


class TestPublish:
    """publish happy path + no-subscriber no-op."""

    async def test_publish_to_unknown_channel_is_noop(self):
        """Publishing to a channel with no subscribers must not raise.

        This is the "republish does not raise" half of
        ``test_pubsub_unsubscribe_releases_queue`` asserted in isolation
        — without the prior subscribe/unsubscribe, on a fresh manager.
        The in-process fan-out must treat "no subscribers" as a silent
        no-op to match the Redis ``PUBLISH`` semantic (which just
        returns 0).
        """
        manager = PubSubManager()
        await manager.publish("inbox:nobody-home", "task-drop")

    async def test_publish_delivers_to_single_subscriber(self):
        """A single subscriber receives the published message unchanged."""
        manager = PubSubManager()
        subscription = await manager.subscribe("inbox:solo")

        await manager.publish("inbox:solo", "task-solo-1")

        msg = await _next_msg(subscription)
        assert msg == "task-solo-1"


# ---------------------------------------------------------------------------
# subscribe
# ---------------------------------------------------------------------------


class TestSubscribe:
    """subscribe returns an async-iterator-compatible _Subscription."""

    async def test_subscribe_returns_async_iterator(self):
        """The return value implements the async-iterator protocol.

        The design doc explicitly requires ``_Subscription`` to preserve
        the async-iterator interface so ``event_generator`` in
        ``api/subscribe.py`` does not need a structural change. This
        test guards that contract: ``__aiter__`` and ``__anext__`` must
        both be defined.
        """
        manager = PubSubManager()
        subscription = await manager.subscribe("inbox:iter-check")

        assert hasattr(subscription, "__aiter__"), (
            "_Subscription must implement __aiter__ for `async for` support"
        )
        assert hasattr(subscription, "__anext__"), (
            "_Subscription must implement __anext__ for async iteration"
        )

    async def test_subscribe_yields_messages_in_publish_order(self):
        """Three publishes are yielded in FIFO order on the subscription.

        ``asyncio.Queue`` is FIFO-by-default, so this test also guards
        against a future accidental switch to ``LifoQueue`` or
        ``PriorityQueue``.
        """
        manager = PubSubManager()
        subscription = await manager.subscribe("inbox:ordered")

        await manager.publish("inbox:ordered", "task-1")
        await manager.publish("inbox:ordered", "task-2")
        await manager.publish("inbox:ordered", "task-3")

        received = [
            await _next_msg(subscription),
            await _next_msg(subscription),
            await _next_msg(subscription),
        ]
        assert received == ["task-1", "task-2", "task-3"]


# ---------------------------------------------------------------------------
# Fan-out — design-doc regression test
# ---------------------------------------------------------------------------


class TestFanout:
    """Fan-out to multiple subscribers on the same channel."""

    async def test_pubsub_fanout_two_subscribers(self):
        """Two subscribers on the same channel both receive a published msg.

        This is the exact regression test named in the design doc
        Testing Strategy table. The in-process fan-out must iterate
        over ALL queues registered under a given channel and
        ``put_nowait`` the message on each — not just the first one.

        The previous Redis Pub/Sub version got this for free (Redis
        does the fan-out inside the server). The asyncio.Queue version
        has to do it explicitly, which is exactly the code path this
        test exercises.
        """
        manager = PubSubManager()
        sub_a = await manager.subscribe("inbox:fan")
        sub_b = await manager.subscribe("inbox:fan")

        await manager.publish("inbox:fan", "task-fanout")

        msg_a = await _next_msg(sub_a)
        msg_b = await _next_msg(sub_b)

        assert msg_a == "task-fanout", (
            f"first subscriber should receive fanout message; got {msg_a!r}"
        )
        assert msg_b == "task-fanout", (
            f"second subscriber should receive fanout message; got {msg_b!r}"
        )

    async def test_fanout_registers_distinct_queues(self):
        """Two subscribes on the same channel create two independent queues.

        Guards against an optimization-gone-wrong where the manager
        reuses a single queue per channel (which would break the
        fan-out because each message would be delivered to only one
        subscriber).
        """
        manager = PubSubManager()
        await manager.subscribe("inbox:distinct")
        await manager.subscribe("inbox:distinct")

        queues = manager._subscribers["inbox:distinct"]
        assert len(queues) == 2, (
            f"two subscribe() calls must register two distinct queues; "
            f"got {len(queues)}"
        )


# ---------------------------------------------------------------------------
# unsubscribe — design-doc regression test
# ---------------------------------------------------------------------------


class TestUnsubscribe:
    """unsubscribe cleanup + post-unsubscribe publish safety."""

    async def test_pubsub_unsubscribe_releases_queue(self):
        """After unsubscribing the only subscriber, the channel entry is gone
        and a republish does not raise.

        This is the exact regression test named in the design doc
        Testing Strategy table. It asserts TWO invariants in one place:

          1. ``_subscribers[channel]`` empties and the channel key is
             popped from the dict (memory hygiene — abandoned channels
             MUST not accumulate).
          2. Publishing to the now-empty channel is a silent no-op
             (matches the Redis ``PUBLISH`` to-no-subscribers semantic).

        If invariant 1 fails, the manager leaks channel entries
        forever — a long-running server would slowly grow this dict.
        If invariant 2 fails, the SSE finally-block cleanup in
        ``event_generator`` would raise if a late publish came in
        after the subscriber disconnected, which would spam error logs
        without delivering anything.
        """
        manager = PubSubManager()
        channel = "inbox:unsub-release"
        await manager.subscribe(channel)
        queue = _any_queue_for(manager, channel)

        await manager.unsubscribe(channel, queue)

        assert channel not in manager._subscribers, (
            f"unsubscribing the only queue must remove the channel entry; "
            f"still present: {channel in manager._subscribers}, "
            f"subscribers dict: {manager._subscribers}"
        )

        await manager.publish(channel, "task-posthumous")

    async def test_unsubscribe_one_of_two_keeps_channel(self):
        """Unsubscribing one queue leaves the other (and the channel) alive.

        Partial unsubscribe must not drop the whole channel set — the
        remaining subscriber must still receive new publishes. This
        complements the "drop empty set" assertion above: both together
        define the exact lifecycle of ``_subscribers[channel]``.
        """
        manager = PubSubManager()
        channel = "inbox:unsub-partial"
        sub_keep = await manager.subscribe(channel)
        await manager.subscribe(channel)

        queues = list(manager._subscribers[channel])
        assert len(queues) == 2
        # Drop exactly one of the two queues.
        await manager.unsubscribe(channel, queues[0])

        assert channel in manager._subscribers, (
            "channel entry must remain while at least one subscriber is active"
        )
        assert len(manager._subscribers[channel]) == 1, (
            f"exactly one queue should remain; "
            f"got {len(manager._subscribers[channel])}"
        )

        # The surviving subscription must still receive new publishes.
        # Note: queues[0] was dropped; sub_keep MIGHT wrap either queue
        # (set iteration is unordered). To guarantee we read from the
        # surviving queue, publish and let whichever subscription wraps
        # the remaining queue observe it. If sub_keep happens to wrap
        # the dropped queue, _next_msg will time out — that's a valid
        # failure mode.
        await manager.publish(channel, "task-surviving")
        # We don't assert on sub_keep here because which wrapper got
        # which queue is implementation-defined. Instead we assert on
        # the remaining raw queue directly:
        remaining = next(iter(manager._subscribers[channel]))
        msg = await asyncio.wait_for(remaining.get(), timeout=1.0)
        assert msg == "task-surviving"


# ---------------------------------------------------------------------------
# Multi-channel isolation
# ---------------------------------------------------------------------------


class TestMultiChannelIsolation:
    """Channels are independent — publishes don't cross channels."""

    async def test_publish_to_one_channel_does_not_reach_other(self):
        """A publish to channel A must not arrive at a channel B subscriber.

        This is the channel-isolation invariant — without it, the
        inbox model collapses (every agent would see every other
        agent's messages).
        """
        manager = PubSubManager()
        sub_b = await manager.subscribe("inbox:bbb")

        await manager.publish("inbox:aaa", "task-for-a")

        # sub_b must not see task-for-a. Verify by publishing to B
        # afterwards and asserting the FIRST message received is B's.
        await manager.publish("inbox:bbb", "task-for-b")

        first = await _next_msg(sub_b)
        assert first == "task-for-b", (
            f"channel B subscriber must NOT receive channel A messages; "
            f"first message was {first!r}"
        )

    async def test_two_channels_deliver_independently(self):
        """Two channels with independent subscribers each get their own msgs."""
        manager = PubSubManager()
        sub_x = await manager.subscribe("inbox:xxx")
        sub_y = await manager.subscribe("inbox:yyy")

        await manager.publish("inbox:xxx", "x-only")
        await manager.publish("inbox:yyy", "y-only")

        msg_x = await _next_msg(sub_x)
        msg_y = await _next_msg(sub_y)

        assert msg_x == "x-only"
        assert msg_y == "y-only"


@pytest.fixture(autouse=True)
def _close_pending_tasks():
    """Nothing to do post-test — function-scoped managers are GC'd.

    The fixture is kept (as a no-op) to document that the new
    in-process PubSubManager requires no async cleanup: there is no
    Redis connection to aclose, no background task to cancel. If a
    future change adds lifecycle hooks, this fixture is the natural
    place to tear them down.
    """
    yield
