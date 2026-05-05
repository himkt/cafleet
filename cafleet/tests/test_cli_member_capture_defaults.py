"""Tests for ``cafleet member capture`` defaults (design doc 0000049, Surface 9).

Step 12 changes:

* ``--lines`` default drops from 80 to 30 (the per-Director-tick cost
  dominates the steady-state token bill).
* New ``--ansi`` / ``--no-ansi`` boolean flag pair, default ``--no-ansi`` —
  strips CSI escape sequences from the captured buffer (regex
  ``\\x1b\\[[0-?]*[ -/]*[@-~]``) and de-fragments TUI redraws by collapsing
  carriage-return overstrikes within a line.
* ``--tail`` is registered as an alias for ``--lines`` (Director's
  recall-time muscle-memory from ``tail -f``).
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import cli
from cafleet.db import engine as engine_mod
from cafleet.tmux import DirectorContext


_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def _reset_engine():
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None
    yield
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None


@pytest.fixture
def bootstrapped_member(tmp_path, monkeypatch, _reset_engine):
    """Fresh DB + session + 1 fake member registered with a known pane_id.

    Returns ``(sid, director_id, member_id, pane_id, runner)``. The member
    is registered via ``broker.register_agent`` (no real ``member create``)
    so the tests can stay focused on capture-flag behaviour.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _FAKE_DIRECTOR_CTX)

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    create = runner.invoke(cli, ["session", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    sid = data["session_id"]
    director_id = data["director"]["agent_id"]

    pane_id = "%17"
    agent = broker.register_agent(
        session_id=sid,
        name="capture-target",
        description="member to capture from",
        placement={
            "director_agent_id": director_id,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": "claude",
        },
    )
    return sid, director_id, agent["agent_id"], pane_id, runner


def _record_run(monkeypatch, *, returns: str = "") -> list[list[str]]:
    """Replace ``tmux._run`` with a recorder, return the list of arg-lists.

    The conftest's autouse fixture stubs ``tmux._run`` to a no-op; this
    helper overrides it with a per-test recorder so the assertions can read
    back the exact ``capture-pane`` argv issued and inject a controlled
    stdout for post-processing tests.
    """
    from cafleet import tmux

    calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        calls.append(list(args))
        return returns

    monkeypatch.setattr(tmux, "_run", mock_run)
    return calls


# --- default --lines is 30 ---


def test_member_capture_default_lines__no_flag_uses_30(
    bootstrapped_member, monkeypatch
):
    """No ``--lines`` flag → tmux receives ``-S -30``. The legacy default
    was ``-S -80``; this is the single most impactful change in Step 12."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    calls = _record_run(monkeypatch)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output

    capture_call = next(call for call in calls if "capture-pane" in call)
    assert capture_call[-2:] == ["-S", "-30"]


def test_member_capture_explicit_lines__overrides_default(
    bootstrapped_member, monkeypatch
):
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    calls = _record_run(monkeypatch)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
            "--lines",
            "150",
        ],
    )
    assert result.exit_code == 0, result.output

    capture_call = next(call for call in calls if "capture-pane" in call)
    assert capture_call[-2:] == ["-S", "-150"]


# --- --tail alias ---


def test_member_capture_tail_alias__forwards_to_lines(
    bootstrapped_member, monkeypatch
):
    """``--tail N`` is sugar for ``--lines N``. Both surfaces are wired
    through the same Click option so tmux receives the same ``-S -N``."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    calls = _record_run(monkeypatch)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
            "--tail",
            "55",
        ],
    )
    assert result.exit_code == 0, result.output

    capture_call = next(call for call in calls if "capture-pane" in call)
    assert capture_call[-2:] == ["-S", "-55"]


# --- --no-ansi (default): ANSI strip + CR de-fragmentation ---


def test_member_capture_default__strips_ansi_escape_sequences(
    bootstrapped_member, monkeypatch
):
    """Default ``--no-ansi`` strips CSI escape sequences from the captured
    buffer so the Director's context window doesn't have to absorb the
    raw byte cost of TUI colour codes."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "\x1b[31mhello\x1b[0m world\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert "\x1b[" not in result.output
    assert "hello world" in result.output


def test_member_capture_default__strips_complex_ansi_sequences(
    bootstrapped_member, monkeypatch
):
    """The strip MUST cover the full CSI grammar from the design doc:
    ``\\x1b\\[`` → 0..n parameter bytes (``[0-?]``) → 0..n intermediate
    bytes (``[ -/]``) → 1 final byte (``[@-~]``). Cursor-positioning,
    SGR, and erase-in-line are all in scope."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "\x1b[2J\x1b[H\x1b[1;33mwarn\x1b[0m: \x1b[Kdone\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert "\x1b" not in result.output
    assert "warn: done" in result.output


def test_member_capture_default__defragments_carriage_return_redraws(
    bootstrapped_member, monkeypatch
):
    """Default ``--no-ansi`` ALSO de-fragments TUI redraws — a line whose
    contents have been overwritten via ``\\r`` reduces to the final visible
    text, not the concatenation of every overstrike. ``loading…\\rdone\\n``
    becomes ``done\\n``."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "loading...\rdone\nnext line\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert "loading" not in result.output
    assert "done" in result.output
    assert "next line" in result.output


def test_member_capture_default__defragments_multiple_redraws_per_line(
    bootstrapped_member, monkeypatch
):
    """Multiple ``\\r`` overstrikes within a single line collapse to the
    last segment — every TUI progress-bar update before the final state
    is discarded."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "10%\r50%\r90%\rfinal\nafter\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert "10%" not in result.output
    assert "50%" not in result.output
    assert "90%" not in result.output
    assert "final" in result.output
    assert "after" in result.output


# --- --ansi: opt back into the raw buffer ---


def test_member_capture_ansi_flag__preserves_raw_escape_sequences(
    bootstrapped_member, monkeypatch
):
    """``--ansi`` is the explicit opt-in for the raw tmux buffer. With the
    flag set, the CSI escape sequences make it into the rendered output
    untouched — useful when the operator is debugging an actual TUI render."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "\x1b[31mhello\x1b[0m world\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
            "--ansi",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "\x1b[31m" in result.output
    assert "\x1b[0m" in result.output


def test_member_capture_ansi_flag__preserves_carriage_returns(
    bootstrapped_member, monkeypatch
):
    """``--ansi`` opts out of the full post-processing pipeline, not just
    ANSI-strip — carriage returns survive too. The flag's contract is
    ``raw vs. cleaned`` rather than ``ANSI-only``."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "loading...\rdone\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
            "--ansi",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "loading" in result.output
    assert "\r" in result.output


# --- JSON-mode parity ---


def test_member_capture_json_default__content_is_post_processed(
    bootstrapped_member, monkeypatch
):
    """The JSON envelope's ``content`` field MUST go through the same
    ANSI-strip + CR-defrag pipeline as the text mode. Otherwise downstream
    JSON consumers receive raw bytes that the text mode has already cleaned
    up — surprise asymmetry."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    raw = "\x1b[32mhello\x1b[0m\rworld\n"
    _record_run(monkeypatch, returns=raw)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert "\x1b" not in payload["content"]
    assert "hello" not in payload["content"]
    assert "world" in payload["content"]


def test_member_capture_json_default__lines_field_reflects_new_default(
    bootstrapped_member, monkeypatch
):
    """The JSON envelope's ``lines`` field is the value the operator can
    re-issue verbatim. With the new default in place, an unflagged
    invocation reports ``"lines": 30`` (was 80)."""
    sid, director_id, member_id, _pane_id, runner = bootstrapped_member
    _record_run(monkeypatch)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "member",
            "capture",
            "--agent-id",
            director_id,
            "--member-id",
            member_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["lines"] == 30
