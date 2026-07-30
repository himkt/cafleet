"""Exact cross-backend contract for the pure-trigger monitoring-member wake.

The wake payload is a pure trigger: the due list, the Director descriptor, and
one pointer sentence naming the monitor role protocol. The full on-wake
protocol lives in the cafleet skill's monitor role file, never in the payload.
"""

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
    "Director: 332 (coding_agent=opencode). "
    "Follow your monitor role protocol."
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


def test_send_wake_trigger__payload_exact_text_multi_member(monkeypatch):
    payload = _capture_tmux_payload(
        monkeypatch,
        due_members=[
            _member(
                332,
                name="Director",
                coding_agent="codex",
                is_director=True,
                wake_reasons=["interval"],
            ),
            _member(
                336,
                name="alice",
                coding_agent="claude",
                wake_reasons=["interval", "stall-check"],
            ),
        ],
        director=_director(332, coding_agent="codex"),
    )
    assert payload == (
        "[monitor] wake: 2 members due — "
        "director 332 (Director; coding_agent=codex) [interval], "
        "member 336 (alice; coding_agent=claude) [interval,stall-check]. "
        "Director: 332 (coding_agent=codex). "
        "Follow your monitor role protocol."
    )


def test_send_wake_trigger__pure_trigger_carries_no_protocol_clauses(monkeypatch):
    """The payload names who is due and who the Director is — nothing else.

    The on-wake protocol (capture, classify, two-wake confirmation, ping,
    per-event Director messages) is carried solely by the monitor role file;
    none of its vocabulary appears in the wake payload.
    """
    payload = _capture_tmux_payload(monkeypatch)
    lowered = payload.lower()
    for forbidden in (
        "capture",
        "classify",
        "observe",
        "ping",
        "token",
        "aggregate",
        "report-batch",
        "escalat",
        "stall_candidate",
        "awaiting_user",
        "--lines",
        "message send",
    ):
        assert forbidden not in lowered
    assert "\n" not in payload
    assert len(payload) < 400


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
    assert "Director: 332 (coding_agent=opencode)." in payload
    assert payload.endswith("Follow your monitor role protocol.")
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
    # three due entries plus the single Director descriptor
    assert tmux_payload.count("coding_agent=") == 4
