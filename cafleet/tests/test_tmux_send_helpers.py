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


@pytest.mark.parametrize("digit", [1, 2, 3])
def test_tmux_helpers__send_choice_key_records_exact_argv(run_recorder, digit):
    tmux.send_choice_key(target_pane_id="%7", digit=digit)
    assert len(run_recorder) == 1
    assert run_recorder[0]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        str(digit),
    ]


def test_tmux_helpers__send_choice_key_does_not_append_enter(run_recorder):
    tmux.send_choice_key(target_pane_id="%7", digit=1)
    assert "Enter" not in run_recorder[0]["args"]


@pytest.mark.parametrize("bad_digit", [0, 4, 5, -1, 10])
def test_tmux_helpers__send_choice_key_rejects_out_of_range(run_recorder, bad_digit):
    with pytest.raises(tmux.TmuxError, match="must be"):
        tmux.send_choice_key(target_pane_id="%7", digit=bad_digit)
    assert len(run_recorder) == 0


def test_tmux_helpers__send_choice_key_different_pane_id(run_recorder):
    tmux.send_choice_key(target_pane_id="%99", digit=2)
    assert run_recorder[0]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%99",
        "2",
    ]


def test_tmux_helpers__send_freetext_and_submit_three_calls_in_order(run_recorder):
    tmux.send_freetext_and_submit(target_pane_id="%7", text="hello")
    assert len(run_recorder) == 3
    assert run_recorder[0]["args"] == ["tmux", "send-keys", "-t", "%7", "4"]
    assert run_recorder[1]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "-l",
        "hello",
    ]
    assert run_recorder[2]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "Enter",
    ]


def test_tmux_helpers__send_freetext_and_submit_uses_literal_flag_for_text(
    run_recorder,
):
    tmux.send_freetext_and_submit(target_pane_id="%7", text="hello")
    second = run_recorder[1]["args"]
    assert "-l" in second
    l_index = second.index("-l")
    assert second[l_index + 1] == "hello"


def test_tmux_helpers__send_freetext_and_submit_empty_string_accepted(run_recorder):
    tmux.send_freetext_and_submit(target_pane_id="%7", text="")
    assert len(run_recorder) == 3
    assert run_recorder[1]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "-l",
        "",
    ]


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
def test_tmux_helpers__send_freetext_and_submit_rejects_newlines(
    run_recorder, bad_text
):
    with pytest.raises(tmux.TmuxError, match="(?i)newline"):
        tmux.send_freetext_and_submit(target_pane_id="%7", text=bad_text)
    assert len(run_recorder) == 0


def test_tmux_helpers__send_freetext_and_submit_shell_meta_passes_through_to_run(
    run_recorder,
):
    payload = "$(echo pwn) `bt` $VAR ; && | > < rm -rf /"
    tmux.send_freetext_and_submit(target_pane_id="%7", text=payload)
    assert run_recorder[1]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "-l",
        payload,
    ]


def test_tmux_helpers__send_freetext_and_submit_multibyte_passes_through(run_recorder):
    payload = "日本語 テスト ✓ 🚀"
    tmux.send_freetext_and_submit(target_pane_id="%7", text=payload)
    assert run_recorder[1]["args"][-1] == payload


def test_tmux_helpers__send_freetext_and_submit_key_name_lookalike_passes_through(
    run_recorder,
):
    payload = "Enter C-c Esc"
    tmux.send_freetext_and_submit(target_pane_id="%7", text=payload)
    assert run_recorder[1]["args"] == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "-l",
        payload,
    ]


def test_tmux_helpers__send_freetext_and_submit_different_pane_id(run_recorder):
    tmux.send_freetext_and_submit(target_pane_id="%42", text="x")
    for call in run_recorder:
        assert "%42" in call["args"]
