"""Surface 15 — ``tmux.send_inline_preview`` keystroke helper (design 0000049 Step 4).

A new tmux helper that injects a recipient-facing inline preview of a freshly
delivered message: the bracketed envelope ``[cafleet msg <id8> from <from8>
<ts>]`` on its own line, followed by the message body, then ``Enter`` to
submit. Replaces the auto-fire ``send_poll_trigger`` invocation made by
``broker._try_notify_recipient`` (Step 4, broker-side wiring tested in
``test_broker_inline_preview.py``).

Design constraints exercised here:

- New helper, NOT a reuse of ``send_freetext_and_submit`` (which prepends
  the literal ``"4"`` keystroke for the AskUserQuestion option-4 freetext
  semantics — using it would corrupt the recipient's input box).
- Uses the literal-text-plus-Enter pattern from ``send_poll_trigger`` (so
  shell meta, key-name lookalikes, and multi-byte chars pass through as
  literal input) plus the ``_SUBMIT_DELAY`` between the literal-text send
  and the Enter send (codex bracketed-paste finalisation).
- Best-effort: returns ``False`` on any tmux failure or when the binary is
  missing; never raises.
- ``send_poll_trigger`` is preserved unchanged for ``cafleet member ping``.
"""

import pytest

from cafleet import tmux


# Spec-canonical bracketed envelope shape — used by every assertion below.
# The literal payload sent via ``send-keys -l`` must contain this prefix.
ENVELOPE_PREFIX = "[cafleet msg "


# ---------------------------------------------------------------------------
# 1. Happy-path keystroke shape
# ---------------------------------------------------------------------------


def _capture_run(monkeypatch) -> list[list[str]]:
    """Stub ``tmux._run`` and ``shutil.which`` and return a list of captured argv."""
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    return captured


def test_send_inline_preview__success_returns_true(monkeypatch):
    _capture_run(monkeypatch)
    result = tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="ffffffff",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="Hello world",
    )
    assert result is True


def test_send_inline_preview__envelope_prefix_appears_in_literal_payload(monkeypatch):
    """The bracketed envelope ``[cafleet msg <id8> from <from8> <ts>]`` must
    appear in some ``-l`` literal argument supplied to tmux."""
    captured = _capture_run(monkeypatch)
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="any body",
    )
    literal_payloads = _literal_payloads(captured)
    joined = "\n".join(literal_payloads)
    assert ENVELOPE_PREFIX in joined, (
        f"envelope prefix {ENVELOPE_PREFIX!r} missing from literal payloads;"
        f" got: {literal_payloads!r}"
    )


def test_send_inline_preview__envelope_carries_task_id_8(monkeypatch):
    captured = _capture_run(monkeypatch)
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    joined = "\n".join(_literal_payloads(captured))
    assert "abcdef01" in joined


def test_send_inline_preview__envelope_carries_sender_8(monkeypatch):
    captured = _capture_run(monkeypatch)
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    joined = "\n".join(_literal_payloads(captured))
    assert "zyxwvuts" in joined


def test_send_inline_preview__envelope_carries_ts(monkeypatch):
    captured = _capture_run(monkeypatch)
    ts = "2026-05-05T12:00:00.123456+00:00"
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts=ts,
        text="body",
    )
    joined = "\n".join(_literal_payloads(captured))
    assert ts in joined


def test_send_inline_preview__body_text_appears_in_literal_payload(monkeypatch):
    captured = _capture_run(monkeypatch)
    body = "Did the API schema change?"
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text=body,
    )
    joined = "\n".join(_literal_payloads(captured))
    assert body in joined


def test_send_inline_preview__final_call_submits_with_enter(monkeypatch):
    """After all literal sends the helper finishes with an explicit Enter
    keystroke — a separate ``send-keys`` call (no ``-l``)."""
    captured = _capture_run(monkeypatch)
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    last = captured[-1]
    assert last == ["tmux", "send-keys", "-t", "%7", "Enter"]


def test_send_inline_preview__targets_correct_pane_in_every_call(monkeypatch):
    captured = _capture_run(monkeypatch)
    tmux.send_inline_preview(
        target_pane_id="%9",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    for argv in captured:
        # Each tmux call must include "-t %9".
        assert "-t" in argv
        idx = argv.index("-t")
        assert argv[idx + 1] == "%9", f"call {argv!r} did not target %9"


def test_send_inline_preview__inserts_submit_delay_before_enter(monkeypatch):
    """``time.sleep`` called between the last literal-send and the Enter send.

    ``send_poll_trigger`` documents a ~120 ms sleep so the codex TUI's
    bracketed-paste finalises before Enter; ``send_inline_preview`` shares
    that helper's literal-text-plus-Enter design and MUST honour the same
    pause to remain backend-agnostic.
    """
    captured = _capture_run(monkeypatch)
    sleep_calls: list[float] = []
    monkeypatch.setattr("time.sleep", lambda secs: sleep_calls.append(secs))
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    # At least one positive sleep is required between literal payload send
    # and the Enter submit.
    assert any(s > 0 for s in sleep_calls), (
        f"expected a positive sleep before Enter; sleep_calls={sleep_calls!r}, "
        f"captured={captured!r}"
    )


# ---------------------------------------------------------------------------
# 2. Best-effort failure modes
# ---------------------------------------------------------------------------


def test_send_inline_preview__pane_not_found_returns_false(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

    def mock_run(args, **_kwargs):
        raise tmux.TmuxError(
            "tmux command failed: tmux send-keys -t %99\n"
            "stderr: can't find pane: %99"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_inline_preview(
        target_pane_id="%99",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    assert result is False


def test_send_inline_preview__tmux_binary_missing_returns_false(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: None)
    run_called = False

    def mock_run(args, **_kwargs):
        nonlocal run_called
        run_called = True
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    assert result is False
    assert not run_called


def test_send_inline_preview__never_raises_on_tmux_error(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

    def mock_run(args, **_kwargs):
        raise tmux.TmuxError("tmux command failed: server exited unexpectedly")

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    assert result is False


# ---------------------------------------------------------------------------
# 3. Anti-regression: NOT a reuse of ``send_freetext_and_submit``
# ---------------------------------------------------------------------------


def test_send_inline_preview__does_not_prepend_literal_4_keystroke(monkeypatch):
    """Surface 15 explicitly forbids reusing ``send_freetext_and_submit``,
    which prepends a literal ``"4"`` to route the AskUserQuestion option-4
    freetext slot. ``send_inline_preview`` must NOT inject that prefix —
    typing a stray ``4`` into the recipient's input box would corrupt it
    when the recipient is not on an AskUserQuestion frame.
    """
    captured = _capture_run(monkeypatch)
    tmux.send_inline_preview(
        target_pane_id="%7",
        task_id_8="abcdef01",
        sender_8="zyxwvuts",
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    # No call should be a bare-digit ``"4"`` keystroke (the AskUserQuestion
    # option-4 routing call). ``send_freetext_and_submit`` issues exactly:
    #   ["tmux", "send-keys", "-t", <pane>, "4"]
    # before its literal+Enter pair. Assert no such call here.
    for argv in captured:
        if argv[:4] == ["tmux", "send-keys", "-t", "%7"] and len(argv) == 5:
            assert argv[4] != "4", (
                "send_inline_preview must NOT issue a bare '4' keystroke "
                "(that would route to AskUserQuestion option 4). "
                f"Captured: {captured!r}"
            )


# ---------------------------------------------------------------------------
# 4. send_poll_trigger preservation guard
# ---------------------------------------------------------------------------


def test_send_poll_trigger__still_exists_post_surface_15():
    """``cafleet member ping`` (the manual Director re-poke primitive)
    continues to depend on ``send_poll_trigger``. Surface 15 must NOT
    delete or rename this helper."""
    assert hasattr(tmux, "send_poll_trigger")
    assert callable(tmux.send_poll_trigger)


def test_send_poll_trigger__keystroke_shape_unchanged(monkeypatch):
    """Surface 15 must not regress ``send_poll_trigger``'s observable
    keystroke contract — ``cafleet member ping`` still injects the
    same poll command literal + Enter pair."""
    captured = _capture_run(monkeypatch)
    ok = tmux.send_poll_trigger(
        target_pane_id="%5",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        agent_id="7ba91234-5678-90ab-cdef-112233445566",
    )
    assert ok is True
    assert len(captured) == 2
    keystroke = captured[0][-1]
    assert keystroke.startswith("cafleet --session-id ")
    assert "message poll --agent-id" in keystroke
    assert captured[1] == ["tmux", "send-keys", "-t", "%5", "Enter"]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _literal_payloads(captured: list[list[str]]) -> list[str]:
    """Extract the literal-text payloads (the value following ``-l``) from a
    list of captured ``tmux send-keys`` argvs."""
    payloads: list[str] = []
    for argv in captured:
        if "-l" in argv:
            idx = argv.index("-l")
            if idx + 1 < len(argv):
                payloads.append(argv[idx + 1])
    return payloads
