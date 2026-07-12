"""``TmuxMultiplexer.send_inline_preview`` keystroke helper."""

import pytest

from cafleet.multiplexer import tmux as multiplexer_tmux
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer

_tmux = TmuxMultiplexer()

ENVELOPE_PREFIX = "[cafleet msg "


def _capture_run(monkeypatch) -> list[list[str]]:
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    return captured


def _literal_payloads(captured: list[list[str]]) -> list[str]:
    payloads: list[str] = []
    for argv in captured:
        if "-l" in argv:
            idx = argv.index("-l")
            if idx + 1 < len(argv):
                payloads.append(argv[idx + 1])
    return payloads


def test_send_inline_preview__happy_path_envelope_body_and_submit(monkeypatch):
    captured = _capture_run(monkeypatch)
    sleep_calls: list[float] = []
    monkeypatch.setattr("time.sleep", lambda secs: sleep_calls.append(secs))

    body = "Did the API schema change?"
    ts = "2026-05-05T12:00:00.123456+00:00"
    result = _tmux.send_inline_preview(
        target_pane_id="%9",
        message_id=1234,
        sender_id=5678,
        ts=ts,
        text=body,
    )

    assert result is True
    joined = "\n".join(_literal_payloads(captured))
    assert ENVELOPE_PREFIX in joined
    assert "1234" in joined
    assert "5678" in joined
    assert ts in joined
    assert body in joined

    # Every tmux call must target %9.
    for argv in captured:
        assert "-t" in argv
        idx = argv.index("-t")
        assert argv[idx + 1] == "%9"

    # The inline preview leads with `Escape` (the
    # universal permission-prompt safeguard) and submits with the trailing Enter.
    assert captured[0] == ["tmux", "send-keys", "-t", "%9", "Escape"]
    assert captured[-1] == ["tmux", "send-keys", "-t", "%9", "Enter"]
    assert any(s > 0 for s in sleep_calls)

    # Anti-regression: the inline preview never sends a bare `4` keystroke as
    # one of its single-arg send-keys calls.
    for argv in captured:
        if argv[:4] == ["tmux", "send-keys", "-t", "%9"] and len(argv) == 5:
            assert argv[4] != "4"


def test_send_inline_preview__esc_first_full_sequence(monkeypatch):
    """The inline preview leads with `Escape` so a
    recipient pane parked on a pending permission-approval prompt dismisses it
    BEFORE any payload character is typed and the trailing Enter can never
    confirm it. Full shape: Escape → settle → `-l <payload>` → submit → Enter,
    identical in shape to `send_poll_trigger`/`member ping`."""
    captured = _capture_run(monkeypatch)
    sleeps: list[float] = []
    monkeypatch.setattr("time.sleep", lambda secs: sleeps.append(secs))

    result = _tmux.send_inline_preview(
        target_pane_id="%4",
        message_id=7,
        sender_id=8,
        ts="2026-06-15T00:00:00+00:00",
        text="body",
    )
    assert result is True

    # Exactly three send-keys calls: Escape, the literal payload, Enter.
    assert len(captured) == 3
    assert captured[0] == ["tmux", "send-keys", "-t", "%4", "Escape"]
    assert captured[1][:5] == ["tmux", "send-keys", "-t", "%4", "-l"]
    assert captured[2] == ["tmux", "send-keys", "-t", "%4", "Enter"]
    # Escape settles first, then the literal/Enter submit delay.
    assert sleeps == [
        multiplexer_tmux._ESC_SETTLE_DELAY,
        multiplexer_tmux._SUBMIT_DELAY,
    ]


def test_send_inline_preview__newline_soft_insert_single_submit(monkeypatch):
    """Anticipated soft-insert branch: the 2-line
    payload shape is preserved — the envelope/body separator stays a single
    embedded ``\\n`` carried by ONE ``-l`` keystroke, and the message is
    submitted by exactly ONE trailing ``Enter``. The embedded newline does NOT
    fragment delivery into a second submit; with the leading `Escape`
    guaranteeing the prompt is dismissed first, the 2-line form is safe."""
    captured = _capture_run(monkeypatch)
    monkeypatch.setattr("time.sleep", lambda _secs: None)

    _tmux.send_inline_preview(
        target_pane_id="%4",
        message_id=11,
        sender_id=22,
        ts="2026-06-15T00:00:00+00:00",
        text="single line body",
    )

    # Exactly one literal (`-l`) keystroke carries the whole 2-line payload …
    literal_payloads = _literal_payloads(captured)
    assert len(literal_payloads) == 1
    payload = literal_payloads[0]
    # … with exactly one embedded newline (the envelope/body separator) — the
    # 2-line shape, not a single-line collapse.
    assert payload.count("\n") == 1
    envelope, _, body_line = payload.partition("\n")
    assert envelope.startswith(ENVELOPE_PREFIX)
    assert body_line == "single line body"

    # Exactly one submit: a single trailing `Enter` keystroke, no second submit
    # produced by the embedded newline.
    enter_calls = [argv for argv in captured if argv[-1:] == ["Enter"]]
    assert len(enter_calls) == 1
    assert captured[-1] == ["tmux", "send-keys", "-t", "%4", "Enter"]


@pytest.mark.parametrize(
    ("scenario", "which_return", "mock_error", "expect_run_called"),
    [
        (
            "pane_not_found",
            "/usr/bin/tmux",
            TmuxError(
                "tmux command failed: tmux send-keys -t %99\n"
                "stderr: can't find pane: %99"
            ),
            True,
        ),
        ("tmux_binary_missing", None, None, False),
        (
            "server_error_never_raises",
            "/usr/bin/tmux",
            TmuxError("tmux command failed: server exited unexpectedly"),
            True,
        ),
    ],
)
def test_send_inline_preview__failure_modes_return_false(
    monkeypatch, scenario, which_return, mock_error, expect_run_called
):
    monkeypatch.setattr("shutil.which", lambda _: which_return)
    run_called = {"yes": False}

    def mock_run(args, **_kwargs):
        run_called["yes"] = True
        if mock_error is not None:
            raise mock_error
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    result = _tmux.send_inline_preview(
        target_pane_id="%7",
        message_id=1234,
        sender_id=5678,
        ts="2026-05-05T12:00:00.000000+00:00",
        text="body",
    )
    assert result is False
    if not expect_run_called:
        assert run_called["yes"] is False


def test_send_poll_trigger__keystroke_contract(monkeypatch):
    """`cafleet member ping` depends on ``send_poll_trigger``; this guards its
    observable keystroke contract against deletion or regression."""
    assert hasattr(TmuxMultiplexer, "send_poll_trigger")
    assert callable(TmuxMultiplexer.send_poll_trigger)

    captured = _capture_run(monkeypatch)
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    ok = _tmux.send_poll_trigger(
        target_pane_id="%5",
        fleet_id=42,
        member_id=7,
    )
    assert ok is True
    # send_poll_trigger is Esc-first: Escape → -l poll → Enter.
    assert len(captured) == 3
    assert captured[0] == ["tmux", "send-keys", "-t", "%5", "Escape"]
    keystroke = captured[1][-1]
    assert keystroke.startswith("cafleet message poll --fleet-id ")
    assert "--member-id" in keystroke
    assert captured[2] == ["tmux", "send-keys", "-t", "%5", "Enter"]
