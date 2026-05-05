"""Tests for ``tmux.capture_pane`` default-line behaviour (design doc 0000049,
Surface 9, Step 12).

The library default for ``capture_pane(... lines=30)`` drops from 80 to 30
lines, the dominant Director-tick token sink. Existing call sites that pass
``lines`` explicitly are unchanged.
"""

import pytest

from cafleet import tmux


def test_capture_pane__default_lines_is_30(monkeypatch):
    """No ``lines`` kwarg supplied — the library MUST request the last 30
    lines from tmux, not the legacy 80. This is the change that drops
    per-tick capture cost."""
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.capture_pane(target_pane_id="%7")

    assert captured_args == [
        "tmux",
        "capture-pane",
        "-p",
        "-t",
        "%7",
        "-S",
        "-30",
    ]


def test_capture_pane__explicit_lines_still_overrides(monkeypatch):
    """An explicit positive ``lines`` kwarg still overrides the new default."""
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.capture_pane(target_pane_id="%7", lines=120)

    assert captured_args[-1] == "-120"


def test_capture_pane__rejects_non_positive_lines_at_new_default():
    """The ``lines > 0`` invariant is independent of the default value
    change. Sanity-checking it stays in place after the default flips."""
    with pytest.raises(tmux.TmuxError, match="lines must be positive, got 0"):
        tmux.capture_pane(target_pane_id="%7", lines=0)
