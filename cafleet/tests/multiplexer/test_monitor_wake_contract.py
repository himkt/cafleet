"""Exact cross-backend contract for the synchronized monitoring-member wake."""

import pytest

from cafleet.multiplexer import herdr as multiplexer_herdr
from cafleet.multiplexer import tmux as multiplexer_tmux
from cafleet.multiplexer.herdr import HerdrMultiplexer
from cafleet.multiplexer.tmux import TmuxMultiplexer


def _member(
    member_id: int = 336,
    *,
    name: str = "alice",
    coding_agent: str = "claude",
    is_director: bool = False,
    wake_reasons: list[str] | None = None,
) -> dict:
    return {
        "member_id": member_id,
        "name": name,
        "coding_agent": coding_agent,
        "is_director": is_director,
        "wake_reasons": wake_reasons or ["interval", "stall-check", "unacked"],
    }


def _director(
    member_id: int = 332,
    *,
    coding_agent: str = "opencode",
) -> dict:
    return {"member_id": member_id, "coding_agent": coding_agent}


EXPECTED_WAKE_PAYLOAD = (
    "[monitor] wake: 1 member due — member 336 "
    "(alice; coding_agent=claude) [interval,stall-check,unacked]. "
    "Capture every named pane and the initial Director 332 "
    "(coding_agent=opencode) at --lines 120 --no-ansi --json; apply each "
    "target's coding_agent overlay. Treat unacked only as context on an "
    "already-due member; it never authorizes an action. Classify capture "
    "content only in this precedence: awaiting_user, unknown, finished, "
    "working, stall_candidate. Backend-overlay active tool, stream, "
    "generation, working, ambiguous, or truncated cues force working; only "
    "quiet non-finished content with no prompt or active-work cue is a "
    "stall_candidate. Never classify stalled yourself or remember hashes in "
    "process. Query monitor stall pending before ordinary observations, "
    "including durable disabled or dead reports absent from this batch. "
    "Submit every named ordinary observation through monitor stall observe "
    "with the captured_at and content_sha256 from that same capture; add "
    "--stall-check only for that reason, and use loss-tolerant unknown without "
    "capture fields when unreadable. working is always non-actionable, "
    "including when tagged unacked or byte-identical. Run cafleet member ping "
    "only when observe atomically returns action=ping, then immediately record "
    "ping-result --success or --failure; never retry a claimed, nudged, or "
    "pending episode. A failed ping queues ping_failed immediately; an "
    "unchanged next synchronized capture after a successful nudge queues "
    "unchanged_after_nudge exactly once. Restart from durable broker state; "
    "lifecycle cleanup "
    "preserves sticky escalation_pending and resets non-pending disabled, "
    "dead, or placement-pending episodes. The Director being awaiting_user, "
    "working, unknown, dead, or unreadable suppresses only the final "
    "aggregate, never an eligible ordinary-member ping. After all ordinary "
    "actions, recapture Director 332 (coding_agent=opencode) and submit "
    "--director-gate; only finished or broker-resolved stalled after two "
    "byte-identical captures separated by a full stall interval returns a "
    "token, and Director observation never authorizes ping. With that fresh "
    "token, immediately call monitor report-batch exactly once with collected "
    "finished IDs and no intervening command, even when no new entry is known; "
    "without a token make no Director-targeting call. report-batch is the sole "
    "Director-delivery path; it collects every durable pending or newly queued "
    "escalation plus this wake's finished IDs, applies one-open backpressure, "
    "and retries the "
    "same message ID at most once this wake; a surviving open aggregate leaves "
    "new escalations pending and finished IDs ephemeral. The Director must "
    "retrieve an aggregate with message show --full before acting and ACK it "
    "once. Never call message send, message broadcast, or member prompt this "
    "wake; attach no task text or arbitrary instruction, and take no ordinary "
    "action except the fixed member ping. finished is report-only; the Director "
    "alone judges whether assigned work remains."
)


def _capture_tmux_payload(monkeypatch, *, due_members=None, director=None) -> str:
    calls: list[list[str]] = []
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    monkeypatch.setattr("time.sleep", lambda _seconds: None)
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kwargs: calls.append(list(args)) or "",
    )

    result = TmuxMultiplexer().send_wake_trigger(
        target_pane_id="%7",
        due_members=due_members or [_member()],
        director=director or _director(),
    )

    assert result is True
    assert calls[0][:5] == ["tmux", "send-keys", "-t", "%7", "-l"]
    assert calls[1] == ["tmux", "send-keys", "-t", "%7", "Enter"]
    assert all("Escape" not in call for call in calls)
    return calls[0][5]


def test_send_wake_trigger__payload_exact_text(monkeypatch):
    assert _capture_tmux_payload(monkeypatch) == EXPECTED_WAKE_PAYLOAD


def test_send_wake_trigger__working_and_unacked_never_authorize_action(monkeypatch):
    payload = _capture_tmux_payload(monkeypatch)
    assert "Treat unacked only as context on an already-due member" in payload
    assert "working is always non-actionable" in payload
    assert "including when tagged unacked or byte-identical" in payload
    assert "ambiguous, or truncated cues force working" in payload


def test_send_wake_trigger__director_state_does_not_suppress_ordinary_ping(
    monkeypatch,
):
    payload = _capture_tmux_payload(monkeypatch)
    assert (
        "The Director being awaiting_user, working, unknown, dead, or "
        "unreadable suppresses only the final aggregate, never an eligible "
        "ordinary-member ping."
    ) in payload
    assert "unknown, disabled, dead" not in payload
    assert (
        "Run cafleet member ping only when observe atomically returns action=ping"
        in (payload)
    )


def test_send_wake_trigger__broker_owns_spacing_claim_and_restart_state(monkeypatch):
    payload = _capture_tmux_payload(monkeypatch)
    assert "Never classify stalled yourself or remember hashes in process" in payload
    assert "two byte-identical captures separated by a full stall interval" in payload
    assert "Restart from durable broker state" in payload
    assert "never retry a claimed, nudged, or pending episode" in payload
    assert "A failed ping queues ping_failed immediately" in payload
    assert "queues unchanged_after_nudge exactly once" in payload
    assert "lifecycle cleanup preserves sticky escalation_pending" in payload


def test_send_wake_trigger__pending_first_and_loss_tolerant_unknown(monkeypatch):
    payload = _capture_tmux_payload(monkeypatch)
    assert "Query monitor stall pending before ordinary observations" in payload
    assert "disabled or dead reports absent from this batch" in payload
    assert "use loss-tolerant unknown without capture fields when unreadable" in payload


def test_send_wake_trigger__fresh_gate_one_batch_and_same_id_recovery(monkeypatch):
    payload = _capture_tmux_payload(monkeypatch)
    assert (
        "immediately call monitor report-batch exactly once with collected "
        "finished IDs and no intervening command"
    ) in payload
    assert "even when no new entry is known" in payload
    assert "applies one-open backpressure" in payload
    assert "every durable pending or newly queued escalation" in payload
    assert "retries the same message ID at most once this wake" in payload
    assert "new escalations pending and finished IDs ephemeral" in payload


def test_send_wake_trigger__director_full_body_ack_and_completion_ownership(
    monkeypatch,
):
    payload = _capture_tmux_payload(monkeypatch)
    assert "message show --full before acting and ACK it once" in payload
    assert "report-batch is the sole Director-delivery path" in payload
    assert "attach no task text or arbitrary instruction" in payload
    assert "take no ordinary action except the fixed member ping" in payload
    assert "finished is report-only" in payload
    assert "Director alone judges whether assigned work remains" in payload
    assert "cafleet message send" not in payload
    assert "cafleet message broadcast" not in payload
    assert "cafleet member prompt" not in payload


def test_send_wake_trigger__mixed_backends_and_sanitized_names(monkeypatch):
    payload = _capture_tmux_payload(
        monkeypatch,
        due_members=[
            _member(
                332,
                name="Director",
                coding_agent="opencode",
                is_director=True,
                wake_reasons=["interval"],
            ),
            _member(
                336,
                name="evil\r\nname\there`$(id)|whoami",
                coding_agent="claude",
                wake_reasons=["interval", "stall-check"],
            ),
            _member(
                337,
                name="bob",
                coding_agent="codex",
                wake_reasons=["status:done"],
            ),
        ],
    )

    assert payload.startswith("[monitor] wake: 3 members due")
    assert "director 332 (Director; coding_agent=opencode) [interval]" in payload
    assert (
        "member 336 (evil⏎name⏎hereˋ$﹙id)│whoami; coding_agent=claude) "
        "[interval,stall-check]"
    ) in payload
    assert "member 337 (bob; coding_agent=codex) [status:done]" in payload
    assert "Director 332 (coding_agent=opencode)" in payload
    assert "\n" not in payload
    assert "\r" not in payload
    assert "\t" not in payload
    assert "`" not in payload
    assert "$(" not in payload
    assert "|" not in payload


@pytest.mark.parametrize("backend", ["tmux", "herdr"])
@pytest.mark.parametrize(
    ("bad_location", "bad_value"),
    [
        ("due", None),
        ("due", "unknown-agent"),
        ("director", None),
        ("director", "unknown-agent"),
    ],
)
def test_send_wake_trigger__invalid_coding_agent_fails_before_keystroke(
    monkeypatch,
    backend,
    bad_location,
    bad_value,
):
    calls: list[list[str]] = []
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/backend")
    monkeypatch.setattr("time.sleep", lambda _seconds: None)
    due = _member()
    director = _director()
    if bad_location == "due":
        due["coding_agent"] = bad_value
    else:
        director["coding_agent"] = bad_value
    mux = TmuxMultiplexer() if backend == "tmux" else HerdrMultiplexer()
    module = multiplexer_tmux if backend == "tmux" else multiplexer_herdr
    monkeypatch.setattr(
        module,
        "_run",
        lambda args, **_kwargs: calls.append(list(args)) or "",
    )

    with pytest.raises(ValueError, match="coding.agent|coding_agent"):
        mux.send_wake_trigger(
            target_pane_id="%7" if backend == "tmux" else "wG:p1",
            due_members=[due],
            director=director,
        )

    assert calls == []


def test_send_wake_trigger__payload_byte_identical_across_backends(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/backend")
    monkeypatch.setattr("time.sleep", lambda _seconds: None)
    due_members = [
        _member(
            332,
            name="Director",
            coding_agent="opencode",
            is_director=True,
            wake_reasons=["interval"],
        ),
        _member(
            336,
            coding_agent="claude",
            wake_reasons=["interval", "stall-check", "unacked"],
        ),
        _member(
            337,
            name="bob",
            coding_agent="codex",
            wake_reasons=["status:done"],
        ),
    ]
    director = _director(coding_agent="opencode")

    tmux_calls: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kwargs: tmux_calls.append(list(args)) or "",
    )
    TmuxMultiplexer().send_wake_trigger(
        target_pane_id="%7",
        due_members=due_members,
        director=director,
    )

    herdr_calls: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_herdr,
        "_run",
        lambda args, **_kwargs: herdr_calls.append(list(args)) or "",
    )
    HerdrMultiplexer().send_wake_trigger(
        target_pane_id="wG:p1",
        due_members=due_members,
        director=director,
    )

    tmux_payload = tmux_calls[0][5]
    herdr_payload = herdr_calls[0][4]
    assert tmux_payload == herdr_payload
    assert tmux_payload.count("coding_agent=") == 5
