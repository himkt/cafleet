"""Unit tests for ``tmux.send_choice_key`` and ``tmux.send_freetext_and_submit``.

Design doc 0000027 Step 3 — two new helpers alongside the existing
``send_exit`` / ``send_poll_trigger`` / ``capture_pane`` functions in
``cafleet/src/cafleet/tmux.py``. Both are thin wrappers around ``_run``
so every test here monkeypatches ``tmux._run`` and asserts on the exact
``["tmux", "send-keys", ...]`` argv — no real tmux subprocess is ever
invoked.

Contracts pinned:

  - ``send_choice_key(digit=N)`` with ``N in {1,2,3}`` issues exactly one
    ``tmux send-keys -t <pane> <N>`` call (no Enter)
  - ``send_choice_key(digit=N)`` with ``N not in {1,2,3}`` raises
    ``TmuxError`` BEFORE any ``_run`` call
  - ``send_freetext_and_submit(text=T)`` with newline-free ``T`` issues
    three ``_run`` calls in strict order: ``<pane> 4``, ``<pane> -l <T>``,
    ``<pane> Enter`` — three calls because tmux's ``-l`` flag is
    per-invocation and cannot mix literal text with the Enter key name
  - ``send_freetext_and_submit`` with ``"\n"`` or ``"\r"`` in the text
    raises ``TmuxError`` BEFORE any ``_run`` call
  - Empty-string text is accepted and the second call carries ``-l ""``
  - Shell-meta / multi-byte / key-name-lookalike text passes through to
    the helper unchanged (the literal-flag contract is on the tmux side;
    the helper's job is to record the exact argv)
"""

import pytest

from cafleet import tmux


# ---------------------------------------------------------------------------
# Shared fixture
# ---------------------------------------------------------------------------


@pytest.fixture
def run_recorder(monkeypatch):
    """Record every argv list passed to ``tmux._run``.

    ``_run``'s real signature is ``_run(args, *, timeout=None)``. The fake
    accepts ``**kwargs`` so callers that pass ``timeout=`` still work, and
    records both ``args`` and ``kwargs`` for inspection.
    """
    calls: list[dict] = []

    def fake_run(args, **kwargs):
        calls.append({"args": list(args), "kwargs": dict(kwargs)})
        return ""

    monkeypatch.setattr(tmux, "_run", fake_run)
    return calls


class TestTmuxHelpers:
    """Design doc 0000027 Specification § "``tmux.py`` additions"."""

    # ---- send_choice_key -------------------------------------------------

    @pytest.mark.parametrize("digit", [1, 2, 3])
    def test_send_choice_key_records_exact_argv(self, run_recorder, digit):
        tmux.send_choice_key(target_pane_id="%7", digit=digit)
        assert len(run_recorder) == 1, (
            f"send_choice_key must issue exactly one _run call. "
            f"got {len(run_recorder)}: {run_recorder!r}"
        )
        assert run_recorder[0]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            str(digit),
        ], (
            f"send_choice_key argv must be "
            f"['tmux','send-keys','-t','%7','{digit}']. "
            f"got: {run_recorder[0]['args']!r}"
        )

    def test_send_choice_key_does_not_append_enter(self, run_recorder):
        """Digit keys select an option directly; no Enter key follows."""
        tmux.send_choice_key(target_pane_id="%7", digit=1)
        assert "Enter" not in run_recorder[0]["args"], (
            f"send_choice_key argv must NOT contain 'Enter'. "
            f"got: {run_recorder[0]['args']!r}"
        )

    @pytest.mark.parametrize("bad_digit", [0, 4, 5, -1, 10])
    def test_send_choice_key_rejects_out_of_range(self, run_recorder, bad_digit):
        with pytest.raises(tmux.TmuxError) as exc_info:
            tmux.send_choice_key(target_pane_id="%7", digit=bad_digit)
        assert "1, 2, or 3" in str(exc_info.value) or "must be" in str(
            exc_info.value
        ), f"error must explain the digit-range rule. got: {exc_info.value!r}"
        assert len(run_recorder) == 0, (
            f"no _run call must be issued when digit is out of range. "
            f"got: {run_recorder!r}"
        )

    def test_send_choice_key_different_pane_id(self, run_recorder):
        """Pane id is forwarded verbatim — no assumptions about format."""
        tmux.send_choice_key(target_pane_id="%99", digit=2)
        assert run_recorder[0]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%99",
            "2",
        ], f"got: {run_recorder[0]['args']!r}"

    # ---- send_freetext_and_submit ---------------------------------------

    def test_send_freetext_and_submit_three_calls_in_order(self, run_recorder):
        tmux.send_freetext_and_submit(target_pane_id="%7", text="hello")
        assert len(run_recorder) == 3, (
            f"send_freetext_and_submit must issue exactly three _run calls. "
            f"got {len(run_recorder)}: {run_recorder!r}"
        )
        assert run_recorder[0]["args"] == ["tmux", "send-keys", "-t", "%7", "4"], (
            f"1st call must select option 4. got: {run_recorder[0]['args']!r}"
        )
        assert run_recorder[1]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            "-l",
            "hello",
        ], f"2nd call must use -l for literal text. got: {run_recorder[1]['args']!r}"
        assert run_recorder[2]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            "Enter",
        ], f"3rd call must submit with Enter. got: {run_recorder[2]['args']!r}"

    def test_send_freetext_and_submit_uses_literal_flag_for_text(self, run_recorder):
        """The ``-l`` flag MUST immediately precede the user's text on the
        second call — that is what forces tmux to interpret every byte as
        a literal character instead of a key name.
        """
        tmux.send_freetext_and_submit(target_pane_id="%7", text="hello")
        second = run_recorder[1]["args"]
        assert "-l" in second, f"2nd call must carry -l. got: {second!r}"
        l_index = second.index("-l")
        assert second[l_index + 1] == "hello", (
            f"the arg immediately after -l must be the literal text. got: {second!r}"
        )

    def test_send_freetext_and_submit_empty_string_accepted(self, run_recorder):
        tmux.send_freetext_and_submit(target_pane_id="%7", text="")
        assert len(run_recorder) == 3, (
            f"empty text must still produce 3 _run calls. got: {run_recorder!r}"
        )
        assert run_recorder[1]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            "-l",
            "",
        ], f"empty text must be passed as -l ''. got: {run_recorder[1]['args']!r}"

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
    def test_send_freetext_and_submit_rejects_newlines(self, run_recorder, bad_text):
        with pytest.raises(tmux.TmuxError) as exc_info:
            tmux.send_freetext_and_submit(target_pane_id="%7", text=bad_text)
        assert "newline" in str(exc_info.value).lower(), (
            f"error must mention newlines. got: {exc_info.value!r}"
        )
        assert len(run_recorder) == 0, (
            f"no _run call must be issued when text contains newlines. "
            f"got: {run_recorder!r}"
        )

    def test_send_freetext_and_submit_shell_meta_passes_through_to_run(
        self, run_recorder
    ):
        """Shell meta never gets interpreted because ``subprocess.run`` is
        invoked with ``shell=False`` — the helper's job is to forward the
        exact string to the second ``_run`` call.
        """
        payload = "$(echo pwn) `bt` $VAR ; && | > < rm -rf /"
        tmux.send_freetext_and_submit(target_pane_id="%7", text=payload)
        assert run_recorder[1]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            "-l",
            payload,
        ], f"shell meta must be forwarded verbatim. got: {run_recorder[1]['args']!r}"

    def test_send_freetext_and_submit_multibyte_passes_through(self, run_recorder):
        payload = "日本語 テスト ✓ 🚀"
        tmux.send_freetext_and_submit(target_pane_id="%7", text=payload)
        assert run_recorder[1]["args"][-1] == payload, (
            f"multi-byte text must be forwarded verbatim. "
            f"got: {run_recorder[1]['args']!r}"
        )

    def test_send_freetext_and_submit_key_name_lookalike_passes_through(
        self, run_recorder
    ):
        """Text that resembles a tmux key name (``Enter``, ``C-c``, ``Esc``)
        must end up on the ``-l`` branch — the per-invocation literal flag
        is precisely what protects against misinterpretation.
        """
        payload = "Enter C-c Esc"
        tmux.send_freetext_and_submit(target_pane_id="%7", text=payload)
        assert run_recorder[1]["args"] == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            "-l",
            payload,
        ], (
            f"key-name-lookalike text must ride on the -l literal branch. "
            f"got: {run_recorder[1]['args']!r}"
        )

    def test_send_freetext_and_submit_different_pane_id(self, run_recorder):
        tmux.send_freetext_and_submit(target_pane_id="%42", text="x")
        for i, call in enumerate(run_recorder):
            assert "%42" in call["args"], (
                f"call {i} must target %42. got: {call['args']!r}"
            )
