"""Tests for ``cafleet member delete`` CLI subcommand.

Pre-existing gap: ``cafleet member delete`` had no test coverage at all and
was missing the cross-Director authorization check that ``member capture`` and
``member send-input`` enforce. These tests pin:

  - Happy path: fetches agent, deregisters broker-side, sends ``/exit`` to
    the pane, rebalances layout, prints both lines of the text summary.
  - Missing agent → exit 1.
  - No placement row → exit 1 with the exact "use `cafleet deregister` instead"
    hint (member delete's placement-none wording differs from capture/send-input
    on purpose: the caller's intent is "delete this member", so the helpful
    redirect is ``cafleet deregister`` rather than ``cafleet member create``).
  - Cross-Director same-session deletion is REJECTED with exit 1 and the
    same "is not a member of your team" wording as ``member capture`` /
    ``member send-input``. This is the regression guard for the
    authorization gap fixed in this iteration.
  - Pending placement (``tmux_pane_id is None``) still deregisters but skips
    the ``/exit`` call and reports ``(pending — no pane)``.
  - ``tmux.send_exit`` failure is surfaced as a warning — the deregister is
    already committed, so the command still exits 0 (the user's registry state
    matches the intent; they can kill the pane manually).

All tests monkeypatch ``broker`` / ``tmux`` — no real tmux subprocess runs.
"""

import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli
from cafleet.tmux import DirectorContext, TmuxError


DIRECTOR_ID = "11111111-1111-1111-1111-111111111111"
MEMBER_ID = "22222222-2222-2222-2222-222222222222"
OTHER_DIRECTOR_ID = "33333333-3333-3333-3333-333333333333"
PANE_ID = "%7"
MEMBER_NAME = "Claude-B"

_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")

_UNSET: object = object()


def _placement(
    *,
    director_agent_id: str = DIRECTOR_ID,
    tmux_pane_id: str | None = PANE_ID,
    coding_agent: str = "claude",
) -> dict:
    return {
        "director_agent_id": director_agent_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": tmux_pane_id,
        "coding_agent": coding_agent,
        "created_at": "2026-04-16T08:00:00+00:00",
    }


def _agent(
    *,
    agent_id: str = MEMBER_ID,
    name: str = MEMBER_NAME,
    placement: dict | None | object = _UNSET,
) -> dict:
    resolved_placement = _placement() if placement is _UNSET else placement
    return {
        "agent_id": agent_id,
        "name": name,
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": resolved_placement,
    }


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def _stub_tmux_entrypoints(monkeypatch):
    """Every test runs with ``tmux`` side-effects stubbed to no-op."""
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "director_context", lambda: _DIRECTOR_CTX)
    monkeypatch.setattr(tmux, "send_exit", lambda **_: None)
    monkeypatch.setattr(tmux, "select_layout", lambda **_: None)


@pytest.fixture
def deregister_recorder(monkeypatch):
    """Record every ``broker.deregister_agent`` invocation. Returns True by default."""
    calls: list[str] = []

    def fake(member_id):
        calls.append(member_id)
        return True

    monkeypatch.setattr(broker, "deregister_agent", fake)
    return calls


@pytest.fixture
def send_exit_recorder(monkeypatch):
    """Record every ``tmux.send_exit`` invocation."""
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(tmux, "send_exit", fake)
    return calls


def _invoke(runner, session_id, *extra_args):
    return runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "delete",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            *extra_args,
        ],
    )


class TestHappyPath:
    def test_deregisters_and_sends_exit_to_pane(
        self, runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
    ):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
        result = _invoke(runner, session_id)
        assert result.exit_code == 0, (
            f"happy path must exit 0. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert deregister_recorder == [MEMBER_ID], (
            f"broker.deregister_agent must be called exactly once with member_id. "
            f"got: {deregister_recorder!r}"
        )
        assert send_exit_recorder == [
            {"target_pane_id": PANE_ID, "ignore_missing": True}
        ], (
            f"tmux.send_exit must be called with ignore_missing=True and the pane. "
            f"got: {send_exit_recorder!r}"
        )
        out = result.output
        assert "Member deleted." in out, (
            f"summary must include 'Member deleted.'. got: {out!r}"
        )
        assert MEMBER_ID in out, (
            f"summary must include the deleted agent_id. got: {out!r}"
        )
        assert f"{PANE_ID} (closed)" in out, (
            f"summary must report the pane as closed. got: {out!r}"
        )

    def test_json_output_returns_agent_id_and_pane_status(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "--json",
                "member",
                "delete",
                "--agent-id",
                DIRECTOR_ID,
                "--member-id",
                MEMBER_ID,
            ],
        )
        assert result.exit_code == 0, (
            f"--json happy path must exit 0. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        import json

        data = json.loads(result.output)
        assert data == {
            "agent_id": MEMBER_ID,
            "pane_status": f"{PANE_ID} (closed)",
        }, f"JSON output shape mismatch. got: {data!r}"


class TestAuthorizationBoundary:
    def test_missing_agent_exits_one(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
        result = _invoke(runner, session_id)
        assert result.exit_code == 1, (
            f"missing agent must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        out = result.output or ""
        assert MEMBER_ID in out, (
            f"error must reference the missing member_id. got: {out!r}"
        )
        # Pre-fix the CLI raised a ValueError inside the get_agent try/except
        # so the user saw "Error: failed to fetch member: Agent X not found" —
        # misleading because the fetch actually succeeded (it returned None).
        # The fix separates the "fetch threw" path (stays wrapped) from the
        # "fetch returned None" path (emits a direct "Error: Agent X not found").
        assert "failed to fetch member" not in out, (
            f"not-found path must NOT use 'failed to fetch member' wording "
            f"(the fetch succeeded and returned None). got: {out!r}"
        )
        assert f"Error: Agent {MEMBER_ID} not found" in out, (
            f"not-found path must emit the direct 'Error: Agent X not found' "
            f"message. got: {out!r}"
        )
        assert deregister_recorder == [], (
            f"broker.deregister_agent must NOT be called when the agent is "
            f"missing. got: {deregister_recorder!r}"
        )

    def test_fetch_db_error_surfaces_failed_to_fetch_wording(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        """When ``broker.get_agent`` itself raises (e.g. DB connection failure),
        the wrapper's ``failed to fetch member`` wording is appropriate —
        that's precisely the case where the fetch did fail. Symmetric guard
        for the fix above: the wrapper stays in place for real exceptions.
        """

        def boom(*_a, **_kw):
            raise RuntimeError("db connection lost")

        monkeypatch.setattr(broker, "get_agent", boom)
        result = _invoke(runner, session_id)
        assert result.exit_code == 1
        out = result.output or ""
        assert "failed to fetch member" in out, (
            f"real fetch failures must still use 'failed to fetch member' "
            f"wording. got: {out!r}"
        )
        assert "db connection lost" in out, (
            f"underlying exception message must be surfaced. got: {out!r}"
        )
        assert deregister_recorder == [], (
            f"broker.deregister_agent must not run when the fetch fails. "
            f"got: {deregister_recorder!r}"
        )

    def test_placement_none_exits_one_with_deregister_hint(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        monkeypatch.setattr(
            broker, "get_agent", lambda *_a, **_kw: _agent(placement=None)
        )
        result = _invoke(runner, session_id)
        assert result.exit_code == 1, (
            f"placement None must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out, (
            f"error must reference member_id. got: {out!r}"
        )
        assert "has no placement" in out, (
            f"error must say 'has no placement'. got: {out!r}"
        )
        assert "cafleet deregister" in out, (
            f"error must hint at 'cafleet deregister' (not 'member create', "
            f"because this is a delete intent). got: {out!r}"
        )
        assert deregister_recorder == [], (
            f"broker.deregister_agent must NOT be called when the placement "
            f"is absent. got: {deregister_recorder!r}"
        )

    def test_cross_director_same_session_is_rejected(
        self, runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
    ):
        """Regression guard: before this fix, Director A could delete
        Director X's member in the same session because ``member_delete``
        skipped the placement.director_agent_id check that ``member capture``
        and ``member send-input`` both enforce. The fix adds the same guard,
        and this test pins it.
        """
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(
                placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
            ),
        )
        result = _invoke(runner, session_id)
        assert result.exit_code == 1, (
            f"cross-Director delete must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out, (
            f"error must reference member_id. got: {out!r}"
        )
        assert "is not a member of your team" in out, (
            f"error must mirror 'member capture' / 'member send-input' wording "
            f"('is not a member of your team'). got: {out!r}"
        )
        assert OTHER_DIRECTOR_ID in out, (
            f"error must disclose the actual director_agent_id "
            f"{OTHER_DIRECTOR_ID!r}. got: {out!r}"
        )
        assert deregister_recorder == [], (
            f"broker.deregister_agent must NOT be called across Directors. "
            f"got: {deregister_recorder!r}"
        )
        assert send_exit_recorder == [], (
            f"tmux.send_exit must NOT be called across Directors. "
            f"got: {send_exit_recorder!r}"
        )


class TestPendingPlacement:
    def test_pending_pane_id_skips_send_exit(
        self, runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
    ):
        """A placement without a pane_id means split-window never completed.
        The broker row still exists, so we should still deregister — but there
        is no pane to ``/exit``, and the summary line flags that.
        """
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
        )
        result = _invoke(runner, session_id)
        assert result.exit_code == 0, (
            f"pending pane delete must still succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert deregister_recorder == [MEMBER_ID], (
            f"broker.deregister_agent must still be called on pending "
            f"placement. got: {deregister_recorder!r}"
        )
        assert send_exit_recorder == [], (
            f"tmux.send_exit must NOT be called when pane_id is None. "
            f"got: {send_exit_recorder!r}"
        )
        out = result.output
        assert "(pending" in out, f"summary must flag the pending state. got: {out!r}"
        assert "no pane" in out, f"summary must mention 'no pane'. got: {out!r}"


class TestTmuxErrorOnSendExit:
    def test_send_exit_failure_is_surfaced_as_warning(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        """If ``tmux send-keys`` fails after we already deregistered the agent,
        the deregister is NOT rolled back — that would leave the caller's
        intent violated. Instead, print a warning and exit 0. The user can
        ``tmux kill-pane`` manually per the hint.
        """
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

        def fake_send_exit(**_kw):
            raise TmuxError("send-keys failed: pane is dead")

        monkeypatch.setattr(tmux, "send_exit", fake_send_exit)
        result = _invoke(runner, session_id)
        assert result.exit_code == 0, (
            f"send_exit failure AFTER deregister must still exit 0. "
            f"exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert deregister_recorder == [MEMBER_ID], (
            f"broker.deregister_agent must have been called before the "
            f"send_exit failure. got: {deregister_recorder!r}"
        )
        out = result.output
        assert "Warning: send_exit failed" in out, (
            f"output must include the send_exit warning. got: {out!r}"
        )
        assert f"tmux kill-pane -t {PANE_ID}" in out, (
            f"warning must hint at manual kill-pane. got: {out!r}"
        )
        assert f"{PANE_ID} (send_exit failed)" in out, (
            f"summary line must flag the send_exit-failed state. got: {out!r}"
        )
