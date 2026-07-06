"""Tests for ``cafleet.multiplexer.herdr.HerdrMultiplexer`` (all subprocesses mocked).

The herdr backend shells out through two dispatchers: ``_run`` (returns raw
stdout) and ``_run_json`` (parses the ``{"result": {...}}`` envelope and returns
the ``result`` object). ``_run_json`` calls the module-level ``_run``, so every
test monkeypatches ``herdr._run`` alone — capturing the exact argv and feeding a
per-call return value (a JSON envelope for the read paths, ``""`` for the
keystroke/write paths, or an exception to model a failure). This mirrors
``test_tmux.py``'s single-``_run`` mock and pins the herdr argv for each
``Multiplexer`` method plus the ``AgentStateAware`` capability.
"""

import json

import pytest

from cafleet.multiplexer import herdr as multiplexer_herdr
from cafleet.multiplexer.base import MultiplexerContext
from cafleet.multiplexer.herdr import HerdrError, HerdrMultiplexer

# Stateless class — one shared instance for all tests is safe.
_herdr = HerdrMultiplexer()


def _envelope(result) -> str:
    """A herdr success envelope carrying ``result`` (a dict, or None for a
    no-neighbor ``pane``)."""
    return json.dumps({"id": 1, "result": result, "type": "response"})


@pytest.fixture
def herdr_run(monkeypatch):
    """Patch ``herdr._run`` to capture argv and feed per-call return values.

    Returns ``(captured, set_returns)``: ``captured`` is the list of argv lists
    in call order; ``set_returns(*vals)`` primes the per-call return sequence.
    A primed value that is an exception is raised (modelling a subprocess
    failure); once the sequence is exhausted every further call returns ``""``
    (the keystroke/write paths ignore stdout).
    """
    captured: list[list[str]] = []
    state: dict = {"returns": iter(())}

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        try:
            val = next(state["returns"])
        except StopIteration:
            return ""
        if isinstance(val, BaseException):
            raise val
        return val

    monkeypatch.setattr(multiplexer_herdr, "_run", mock_run)

    def set_returns(*vals) -> None:
        state["returns"] = iter(vals)

    return captured, set_returns


# --- ensure_available ------------------------------------------------------


@pytest.mark.parametrize(
    ("which_return", "herdr_env_set", "expected_match"),
    [
        (None, True, "herdr binary not found on PATH"),
        ("/usr/bin/herdr", False, "must be run inside a herdr session"),
    ],
)
def test_ensure_available__detects_binary_and_session_env(
    monkeypatch, which_return, herdr_env_set, expected_match
):
    monkeypatch.setattr("shutil.which", lambda _: which_return)
    if herdr_env_set:
        monkeypatch.setenv("HERDR_ENV", "1")
    else:
        monkeypatch.delenv("HERDR_ENV", raising=False)
    with pytest.raises(HerdrError, match=expected_match):
        _herdr.ensure_available()


def test_ensure_available__succeeds_when_binary_and_env_present(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setenv("HERDR_ENV", "1")
    assert _herdr.ensure_available() is None


# --- context_discovery -----------------------------------------------------


def test_context_discovery__parses_pane_current_json(herdr_run):
    captured, set_returns = herdr_run
    set_returns(
        _envelope(
            {"pane": {"workspace_id": "wG", "tab_id": "wG:t1", "pane_id": "wG:p1"}}
        )
    )
    ctx = _herdr.context_discovery()
    assert captured == [["herdr", "pane", "current"]]
    # session ← workspace_id, window_id ← tab_id, pane_id ← pane_id.
    assert ctx.session == "wG"
    assert ctx.window_id == "wG:t1"
    assert ctx.pane_id == "wG:p1"


def test_context_discovery__missing_field_raises(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"pane": {"workspace_id": "wG", "tab_id": "wG:t1"}}))
    with pytest.raises(HerdrError, match="missing"):
        _herdr.context_discovery()


# --- split_window (layout parity) ------------------------------------------
#
# split_window emulates tmux main-vertical without a single herdr reflow command:
# it reads `herdr pane list`, derives the Director's right column as every pane in
# the Director's tab except the Director's own, and either starts the column
# (first member → split the Director --direction right) or appends to it
# (subsequent → split max(column) --direction down, then equalizes the column via
# _equalize_focused_tab_column). The max()-based stack order is validation-pending
# against a real herdr binary; the argv shapes below pin the current implementation.

_REFERENCE = MultiplexerContext(session="wG", window_id="wG:t1", pane_id="wG:p1")


def test_split_window__first_member_splits_director_right(herdr_run):
    """No pane in the Director's tab besides the Director itself → empty column →
    the first member starts the right column with ``pane split --direction
    right``, then runs the command in the new pane. Panes in other tabs and the
    Director's own pane are excluded from the column."""
    captured, set_returns = herdr_run
    set_returns(
        _envelope(
            {
                "panes": [
                    {"pane_id": "wG:p1", "tab_id": "wG:t1"},  # the Director — excluded
                    {"pane_id": "wX:p1", "tab_id": "wX:t9"},  # another tab — excluded
                ]
            }
        ),
        _envelope({"pane": {"pane_id": "wG:p2"}}),  # split right → new pane
    )
    new_pane = _herdr.split_window(
        reference=_REFERENCE,
        env={"CAFLEET_DATABASE_URL": "sqlite:////tmp/cafleet.db"},
        command=["claude", "Hello world"],
    )
    assert new_pane == "wG:p2"
    assert captured == [
        ["herdr", "pane", "list"],
        [
            "herdr",
            "pane",
            "split",
            "wG:p1",
            "--direction",
            "right",
            "--no-focus",
            "--env",
            "CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet.db",
        ],
        # pane run submits the shlex-joined argv as one shell line.
        ["herdr", "pane", "run", "wG:p2", "claude 'Hello world'"],
    ]


def test_split_window__first_member_no_env_omits_env_flags(herdr_run):
    captured, set_returns = herdr_run
    set_returns(
        _envelope({"panes": [{"pane_id": "wG:p1", "tab_id": "wG:t1"}]}),
        _envelope({"pane": {"pane_id": "wG:p2"}}),
    )
    _herdr.split_window(reference=_REFERENCE, env={}, command=["claude"])
    split_argv = captured[1]
    assert split_argv == [
        "herdr",
        "pane",
        "split",
        "wG:p1",
        "--direction",
        "right",
        "--no-focus",
    ]
    assert "--env" not in split_argv


def _balanced_layout(tab_id: str, col_x: int) -> dict:
    """A two-member right column already at equal heights (down split ratio 0.5),
    so ``_equalize_focused_tab_column`` computes a zero delta and emits no resize.
    The Director pane sits at ``x=0`` (the leftmost column, excluded)."""
    return {
        "layout": {
            "tab_id": tab_id,
            "panes": [
                {
                    "pane_id": "wG:p1",
                    "rect": {"x": 0, "y": 1, "width": 50, "height": 80},
                },
                {
                    "pane_id": "wG:p3",
                    "rect": {"x": col_x, "y": 1, "width": 50, "height": 40},
                },
                {
                    "pane_id": "wG:p4",
                    "rect": {"x": col_x, "y": 41, "width": 50, "height": 40},
                },
            ],
            "splits": [
                {"direction": "right", "rect": {"x": 0, "y": 1}, "ratio": 0.5},
                {"direction": "down", "rect": {"x": col_x, "y": 1}, "ratio": 0.5},
            ],
        }
    }


def test_split_window__subsequent_member_splits_max_then_equalizes(herdr_run):
    """A non-empty column → the member splits ``max(column)`` downward, then
    ``_equalize_focused_tab_column`` reads the focused tab (``pane current`` +
    ``pane layout``) before the command runs. The column is listed [p3, p2] to
    pin that the split targets ``max`` (p3), not the last-listed pane (p2); the
    layout is already balanced (0.5), so no resize is emitted."""
    captured, set_returns = herdr_run
    set_returns(
        _envelope(
            {
                "panes": [
                    {"pane_id": "wG:p1", "tab_id": "wG:t1"},  # Director — excluded
                    {"pane_id": "wG:p3", "tab_id": "wG:t1"},  # column, listed first
                    {"pane_id": "wG:p2", "tab_id": "wG:t1"},  # column, listed second
                    {"pane_id": "wX:p9", "tab_id": "wX:t9"},  # other tab — excluded
                ]
            }
        ),
        _envelope({"pane": {"pane_id": "wG:p4"}}),  # split(max=p3, down) → new pane
        _envelope({"pane": {"tab_id": "wG:t1"}}),  # pane current → focused tab
        _envelope(_balanced_layout("wG:t1", col_x=100)["layout"]),  # pane layout
    )
    new_pane = _herdr.split_window(
        reference=_REFERENCE, env={}, command=["claude", "second"]
    )
    assert new_pane == "wG:p4"
    assert captured == [
        ["herdr", "pane", "list"],
        # max(["wG:p3", "wG:p2"]) == "wG:p3" — the deterministic column anchor.
        ["herdr", "pane", "split", "wG:p3", "--direction", "down", "--no-focus"],
        ["herdr", "pane", "current"],
        ["herdr", "pane", "layout"],
        ["herdr", "pane", "run", "wG:p4", "claude second"],
    ]


def test_split_window__pane_list_missing_field_raises(herdr_run):
    _captured, set_returns = herdr_run
    # a pane entry missing ``pane_id`` → HerdrError (not a silent skip).
    set_returns(_envelope({"panes": [{"tab_id": "wG:t1"}]}))
    with pytest.raises(HerdrError, match="missing"):
        _herdr.split_window(reference=_REFERENCE, env={}, command=["claude"])


# --- _equalize_focused_tab_column (deterministic ratio math) ---------------
#
# The right column is a right-leaning chain of `down` splits; equal heights ⇔
# split_k has ratio 1/(N-k). `pane resize --amount` is a signed delta on the
# split ratio, so each split is driven to target by one arithmetic resize. The
# layout fixture below is the real `herdr pane layout` shape captured from a live
# 3-member column (heights 43/22/21, both splits at ratio 0.5).


def _column_layout(splits_ratios: list[float]) -> str:
    """A `pane layout` envelope for a right column of ``len(splits_ratios)+1``
    members (x=195) beside a Director pane (x=26). Only the split ratios and the
    per-pane x/y (used for ordering + column detection) drive the math."""
    n = len(splits_ratios) + 1
    panes = [
        {"pane_id": "wT:p3", "rect": {"x": 26, "y": 1, "width": 169, "height": 86}}
    ]
    y = 1
    for k in range(n):
        panes.append(
            {
                "pane_id": f"wT:m{k}",
                "rect": {"x": 195, "y": y, "width": 169, "height": 20},
            }
        )
        y += 20
    splits = [{"direction": "right", "rect": {"x": 26, "y": 1}, "ratio": 0.5}]
    y = 1
    for ratio in splits_ratios:
        splits.append({"direction": "down", "rect": {"x": 195, "y": y}, "ratio": ratio})
        y += 20
    return _envelope({"layout": {"tab_id": "wT:t3", "panes": panes, "splits": splits}})


def test_equalize__three_member_column_drives_top_split_to_one_third(herdr_run):
    """Real captured state: a 3-member column with both splits at 0.5 (heights
    43/22/21). Equal thirds ⇒ top split → 1/3, bottom pair split stays 1/2. Only
    the top split moves: resize member1 up by 1/3-1/2 = 0.1667 (deterministic)."""
    captured, set_returns = herdr_run
    set_returns(
        _envelope({"pane": {"tab_id": "wT:t3"}}),  # pane current → focused tab
        _column_layout([0.5, 0.5]),  # pane layout: two down splits at 0.5
    )
    _herdr._equalize_focused_tab_column()
    assert captured == [
        ["herdr", "pane", "current"],
        ["herdr", "pane", "layout"],
        # top split 0.5 → 1/3: negative delta grows the pane below (m1) upward.
        [
            "herdr",
            "pane",
            "resize",
            "--pane",
            "wT:m1",
            "--direction",
            "up",
            "--amount",
            "0.1667",
        ],
    ]


def test_equalize__already_balanced_column_emits_no_resize(herdr_run):
    """A 3-member column already at 1/3 and 1/2 is balanced — every delta rounds
    below the 1e-3 threshold, so no resize is emitted."""
    captured, set_returns = herdr_run
    set_returns(
        _envelope({"pane": {"tab_id": "wT:t3"}}),
        _column_layout([0.3333, 0.5]),
    )
    _herdr._equalize_focused_tab_column()
    assert captured == [["herdr", "pane", "current"], ["herdr", "pane", "layout"]]


def test_equalize__focus_moved_between_reads_skips(herdr_run):
    """The layout's tab_id differs from the focused tab (focus moved between the
    two reads) → no resize, to avoid rebalancing the wrong tab."""
    captured, set_returns = herdr_run
    set_returns(
        _envelope({"pane": {"tab_id": "wT:t3"}}),
        _envelope({"layout": {"tab_id": "wT:t9", "panes": [], "splits": []}}),
    )
    _herdr._equalize_focused_tab_column()
    assert captured == [["herdr", "pane", "current"], ["herdr", "pane", "layout"]]


def test_equalize__best_effort_swallows_herdr_error(herdr_run):
    """A resize failure (or any HerdrError) never propagates — equalization is
    cosmetic and must not fail a spawn."""
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed: server unreachable"))
    assert _herdr._equalize_focused_tab_column() is None


# --- list_pane_ids ---------------------------------------------------------


def test_list_pane_ids__collects_pane_id_set(herdr_run):
    captured, set_returns = herdr_run
    set_returns(_envelope({"panes": [{"pane_id": "wG:p1"}, {"pane_id": "wG:p2"}]}))
    assert _herdr.list_pane_ids() == {"wG:p1", "wG:p2"}
    assert captured == [["herdr", "pane", "list"]]


# --- pane_exists / wait_for_pane_gone --------------------------------------


def test_pane_exists__present_returns_true(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")  # pane get succeeds
    assert _herdr.pane_exists(target_pane_id="wG:p1") is True
    assert captured == [["herdr", "pane", "get", "wG:p1"]]


def test_pane_exists__pane_not_found_returns_false(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    assert _herdr.pane_exists(target_pane_id="wG:p1") is False


def test_pane_exists__other_error_propagates(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed: server unreachable"))
    with pytest.raises(HerdrError, match="server unreachable"):
        _herdr.pane_exists(target_pane_id="wG:p1")


def test_wait_for_pane_gone__returns_true_when_pane_disappears(monkeypatch, herdr_run):
    _captured, set_returns = herdr_run
    monkeypatch.setattr("time.sleep", lambda _s: None)
    # present, present, then pane_not_found → gone.
    set_returns(
        "",
        "",
        HerdrError("herdr command failed", code="pane_not_found"),
    )
    assert (
        _herdr.wait_for_pane_gone(target_pane_id="wG:p1", timeout=5.0, interval=0.1)
        is True
    )


# --- kill_pane (not_found-tolerant teardown) -------------------------------


def test_kill_pane__argv_and_success(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    assert _herdr.kill_pane(target_pane_id="wG:p1") is None
    assert captured == [["herdr", "pane", "close", "wG:p1"]]


def test_kill_pane__ignore_missing_swallows_pane_not_found(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    # ignore_missing=True swallows the pane-not-found teardown race.
    assert _herdr.kill_pane(target_pane_id="wG:p1", ignore_missing=True) is None


def test_kill_pane__ignore_missing_does_not_swallow_other_errors(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed: server unreachable"))
    with pytest.raises(HerdrError, match="server unreachable"):
        _herdr.kill_pane(target_pane_id="wG:p1", ignore_missing=True)


def test_kill_pane__default_raises_on_pane_not_found(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    with pytest.raises(HerdrError):
        _herdr.kill_pane(target_pane_id="wG:p1")


# --- send_exit -------------------------------------------------------------


def test_send_exit__argv_and_ignore_missing(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    _herdr.send_exit(target_pane_id="wG:p1")
    assert captured == [["herdr", "pane", "run", "wG:p1", "/exit"]]


def test_send_exit__ignore_missing_swallows_pane_not_found(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    assert _herdr.send_exit(target_pane_id="wG:p1", ignore_missing=True) is None


# --- send_poll_trigger (esc-safeguarded) -----------------------------------


def test_send_poll_trigger__esc_then_poll_command(monkeypatch, herdr_run):
    captured, set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    result = _herdr.send_poll_trigger(target_pane_id="wG:p1", fleet_id=24, agent_id=88)
    assert result is True
    # Leading esc safeguard, then the poll command via pane run (atomic submit).
    assert captured == [
        ["herdr", "pane", "send-keys", "wG:p1", "esc"],
        [
            "herdr",
            "pane",
            "run",
            "wG:p1",
            "cafleet message poll --fleet-id 24 --agent-id 88",
        ],
    ]


def test_send_poll_trigger__herdr_missing_returns_false(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: None)
    assert (
        _herdr.send_poll_trigger(target_pane_id="wG:p1", fleet_id=24, agent_id=88)
        is False
    )
    assert captured == []


def test_send_poll_trigger__herdr_error_returns_false(monkeypatch, herdr_run):
    _captured, set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    set_returns(HerdrError("herdr command failed: server unreachable"))
    assert (
        _herdr.send_poll_trigger(target_pane_id="wG:p1", fleet_id=24, agent_id=88)
        is False
    )


# --- send_wake_trigger (no esc) --------------------------------------------


def test_send_wake_trigger__no_esc_single_pane_run(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    result = _herdr.send_wake_trigger(
        target_pane_id="wG:p1",
        due_agents=[{"agent_id": 332, "name": "Director", "is_director": True}],
        director_agent_id=332,
    )
    assert result is True
    # No leading esc — the monitoring member's own pane is never on a prompt.
    assert len(captured) == 1
    argv = captured[0]
    assert argv[:4] == ["herdr", "pane", "run", "wG:p1"]
    assert "esc" not in argv
    payload = argv[4]
    assert payload.startswith("[monitor] wake: 1 agent due")
    assert "director 332 (Director)" in payload
    assert "(332)" in payload


def test_send_wake_trigger__sanitizes_name_metacharacters(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    _herdr.send_wake_trigger(
        target_pane_id="wG:p1",
        due_agents=[
            {"agent_id": 332, "name": "Director", "is_director": True},
            {"agent_id": 336, "name": "evil\r\nname\there`$(id)", "is_director": False},
        ],
        director_agent_id=332,
    )
    payload = captured[0][4]
    assert payload.startswith("[monitor] wake: 2 agents due")
    # Single-line + no shell metacharacters survive the sanitizer.
    assert "\n" not in payload
    assert "\r" not in payload
    assert "\t" not in payload
    assert "`" not in payload
    assert "$(" not in payload
    assert "⏎" in payload


# --- send_inline_preview (esc, send-text, enter) ---------------------------


def test_send_inline_preview__esc_send_text_enter(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    result = _herdr.send_inline_preview(
        target_pane_id="wG:p1",
        task_id=7,
        sender_id=42,
        ts="2026-07-06T00:00:00+00:00",
        text="hello there",
    )
    assert result is True
    assert captured == [
        ["herdr", "pane", "send-keys", "wG:p1", "esc"],
        [
            "herdr",
            "pane",
            "send-text",
            "wG:p1",
            "[cafleet msg 7 from 42 2026-07-06T00:00:00+00:00]\nhello there",
        ],
        ["herdr", "pane", "send-keys", "wG:p1", "enter"],
    ]


def test_send_inline_preview__sanitizes_body_newlines(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    _herdr.send_inline_preview(
        target_pane_id="wG:p1",
        task_id=7,
        sender_id=42,
        ts="2026-07-06T00:00:00+00:00",
        text="line1\nline2\r\nline3",
    )
    # The single envelope/body separator newline is kept; body newlines fold to ⏎.
    payload = captured[1][4]
    header, body = payload.split("\n", 1)
    assert header == "[cafleet msg 7 from 42 2026-07-06T00:00:00+00:00]"
    assert "\n" not in body
    assert "\r" not in body
    assert body == "line1⏎line2⏎line3"


# --- send_bash_command -----------------------------------------------------


def test_send_bash_command__argv_and_strip(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    _herdr.send_bash_command(target_pane_id="wG:p1", command="  git status  ")
    assert captured == [["herdr", "pane", "run", "wG:p1", "! git status"]]


@pytest.mark.parametrize(
    ("command", "expected_match"),
    [
        ("", "may not be empty"),
        ("   ", "may not be empty"),
        ("line1\nline2", "may not contain newlines"),
        ("carriage\rreturn", "may not contain newlines"),
    ],
)
def test_send_bash_command__validation(herdr_run, command, expected_match):
    captured, _set_returns = herdr_run
    with pytest.raises(HerdrError, match=expected_match):
        _herdr.send_bash_command(target_pane_id="wG:p1", command=command)
    assert captured == []


# --- capture_pane ----------------------------------------------------------


def test_capture_pane__returns_raw_stdout_verbatim(herdr_run):
    """``herdr pane read`` prints the raw terminal buffer (not a JSON envelope),
    so ``capture_pane`` returns ``_run``'s stdout verbatim."""
    captured, set_returns = herdr_run
    set_returns("line 1\nline 2\n")
    assert _herdr.capture_pane(target_pane_id="wG:p1", lines=20) == "line 1\nline 2\n"
    assert captured == [
        [
            "herdr",
            "pane",
            "read",
            "wG:p1",
            "--source",
            "recent-unwrapped",
            "--lines",
            "20",
        ]
    ]


@pytest.mark.parametrize("lines", [0, -1])
def test_capture_pane__rejects_non_positive_lines(herdr_run, lines):
    _captured, _set_returns = herdr_run
    with pytest.raises(HerdrError, match="lines must be positive"):
        _herdr.capture_pane(target_pane_id="wG:p1", lines=lines)


# --- AgentStateAware: agent_status -----------------------------------------


def test_agent_status__reads_native_state(herdr_run):
    captured, set_returns = herdr_run
    set_returns(_envelope({"pane": {"pane_id": "wG:p1", "agent_status": "blocked"}}))
    assert _herdr.agent_status(target_pane_id="wG:p1") == "blocked"
    assert captured == [["herdr", "pane", "get", "wG:p1"]]


def test_agent_status__empty_or_absent_status_is_none(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"pane": {"pane_id": "wG:p1", "agent_status": ""}}))
    assert _herdr.agent_status(target_pane_id="wG:p1") is None


def test_agent_status__missing_pane_raises(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"not_pane": {}}))
    with pytest.raises(HerdrError, match="missing"):
        _herdr.agent_status(target_pane_id="wG:p1")


def test_agent_status__pane_not_found_returns_none(herdr_run):
    """A pane closing between the tick's ``list_pane_ids`` and this read is a
    teardown race, not an error: ``pane_not_found`` → None (no agent)."""
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    assert _herdr.agent_status(target_pane_id="wG:p1") is None


def test_agent_status__other_error_propagates(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed: server unreachable"))
    with pytest.raises(HerdrError, match="server unreachable"):
        _herdr.agent_status(target_pane_id="wG:p1")


# --- AgentStateAware: wait_agent_status ------------------------------------


def test_wait_agent_status__argv_and_true_on_success(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    result = _herdr.wait_agent_status(
        target_pane_id="wG:p1", status="done", timeout_ms=5000
    )
    assert result is True
    assert captured == [
        [
            "herdr",
            "wait",
            "agent-status",
            "wG:p1",
            "--status",
            "done",
            "--timeout",
            "5000",
        ]
    ]


def test_wait_agent_status__false_on_error(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command timed out"))
    assert (
        _herdr.wait_agent_status(target_pane_id="wG:p1", status="done", timeout_ms=1000)
        is False
    )
