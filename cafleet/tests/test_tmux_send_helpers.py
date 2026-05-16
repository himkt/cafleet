"""Tests for ``cafleet.tmux.send_choice_key`` and ``cafleet.tmux.send_freetext_and_submit``."""

import pytest

from cafleet import tmux


@pytest.fixture
def run_recorder(monkeypatch):
    calls: list[dict] = []

    def fake_run(args, **kwargs):
        calls.append({"args": list(args), "kwargs": dict(kwargs)})
        return ""

    monkeypatch.setattr(tmux, "_run", fake_run)
    return calls


@pytest.mark.parametrize(
    ("scenario", "pane", "digit", "expect_raise_match"),
    [
        ("digit_1", "%7", 1, None),
        ("digit_2", "%7", 2, None),
        ("digit_3", "%7", 3, None),
        ("different_pane", "%99", 2, None),
        ("rejects_zero", "%7", 0, "must be"),
        ("rejects_four", "%7", 4, "must be"),
        ("rejects_five", "%7", 5, "must be"),
        ("rejects_negative", "%7", -1, "must be"),
        ("rejects_ten", "%7", 10, "must be"),
    ],
)
def test_send_choice_key__argv_and_validation(
    run_recorder, scenario, pane, digit, expect_raise_match
):
    if expect_raise_match is not None:
        with pytest.raises(tmux.TmuxError, match=expect_raise_match):
            tmux.send_choice_key(target_pane_id=pane, digit=digit)
        assert run_recorder == []
    else:
        tmux.send_choice_key(target_pane_id=pane, digit=digit)
        assert len(run_recorder) == 1
        assert run_recorder[0]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            pane,
            str(digit),
        ]
        # Choice keystroke must NOT append Enter.
        assert "Enter" not in run_recorder[0]["args"]


@pytest.mark.parametrize(
    ("scenario", "pane", "text"),
    [
        ("plain_ascii", "%7", "hello"),
        ("empty_string", "%7", ""),
        ("shell_meta_literal", "%7", "$(echo pwn) `bt` $VAR ; && | > < rm -rf /"),
        ("multibyte_literal", "%7", "日本語 テスト ✓ 🚀"),
        ("key_name_lookalike", "%7", "Enter C-c Esc"),
        ("different_pane", "%42", "x"),
    ],
)
def test_send_freetext_and_submit__argv_and_literal_passthrough(
    run_recorder, scenario, pane, text
):
    tmux.send_freetext_and_submit(target_pane_id=pane, text=text)
    assert len(run_recorder) == 3
    assert run_recorder[0]["args"] == ["tmux", "send-keys", "-t", pane, "4"]
    assert run_recorder[1]["args"] == ["tmux", "send-keys", "-t", pane, "-l", text]
    assert run_recorder[2]["args"] == ["tmux", "send-keys", "-t", pane, "Enter"]


@pytest.mark.parametrize(
    "bad_text",
    [
        "line1\nline2",
        "\n",
        "\r",
        "leading\ntext",
        "trailing\n",
        "mixed\r\nCRLF",
    ],
)
def test_send_freetext_and_submit__rejects_newlines(run_recorder, bad_text):
    with pytest.raises(tmux.TmuxError, match="(?i)newline"):
        tmux.send_freetext_and_submit(target_pane_id="%7", text=bad_text)
    assert run_recorder == []
