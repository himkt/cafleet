"""CLI tests for ``cafleet member exec`` (design doc 0000037, Step 5).

``member exec`` is the bare-dispatch sibling of ``member safe-exec``: it
validates the positional ``CMD``, enforces the cross-Director boundary,
and dispatches via ``tmux.send_bash_command`` WITHOUT consulting
``cafleet.permissions`` at all. Operator confirmation is delegated to the
outer Bash hook layer (Claude Code's ``permissions.ask`` rule on the
Director-side invocation).
"""

import json
import uuid
from pathlib import Path

import pytest
from click.testing import CliRunner

from cafleet import broker, permissions, tmux
from cafleet.cli import cli

DIRECTOR_ID = "11111111-1111-1111-1111-111111111111"
MEMBER_ID = "22222222-2222-2222-2222-222222222222"
OTHER_DIRECTOR_ID = "33333333-3333-3333-3333-333333333333"
PANE_ID = "%7"
MEMBER_NAME = "Claude-B"

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
    resolved = _placement() if placement is _UNSET else placement
    return {
        "agent_id": agent_id,
        "name": name,
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": resolved,
    }


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture(autouse=True)
def _stub_tmux_available(monkeypatch):
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def happy_path_agent(monkeypatch):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())


@pytest.fixture
def bash_recorder(monkeypatch):
    """Record every call into ``tmux.send_bash_command``."""
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(tmux, "send_bash_command", fake, raising=False)
    return calls


@pytest.fixture
def settings_files(tmp_path, monkeypatch):
    """Three isolated settings files patched into ``permissions.discover_settings_paths``.

    ``member exec`` MUST NOT consult these — the fixture exists to expose
    a deny pattern that ``safe-exec`` would honor, so the dispatch-bypass
    behavior of ``exec`` can be asserted distinctly.
    """
    project_local = tmp_path / "proj" / ".claude" / "settings.local.json"
    project_shared = tmp_path / "proj" / ".claude" / "settings.json"
    user_settings = tmp_path / "user" / ".claude" / "settings.json"
    paths = [project_local, project_shared, user_settings]
    monkeypatch.setattr(
        permissions, "discover_settings_paths", lambda: paths, raising=False
    )
    return paths


def _write_settings(path: Path, *, allow=None, deny=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps({"permissions": {"allow": allow or [], "deny": deny or []}})
    )


def _invoke(runner, session_id, *extra_args, json_mode: bool = False):
    args = ["--session-id", session_id]
    if json_mode:
        args.append("--json")
    args.extend(
        [
            "member",
            "exec",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
        ]
    )
    args.extend(extra_args)
    return runner.invoke(cli, args)


# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------


class TestExecDispatch:
    def test_dispatches_via_send_bash_command(
        self, runner, session_id, happy_path_agent, bash_recorder
    ):
        result = _invoke(runner, session_id, "git status --short")

        assert result.exit_code == 0, result.output
        assert len(bash_recorder) == 1
        call = bash_recorder[0]
        assert call["target_pane_id"] == PANE_ID
        assert call["command"] == "git status --short"

    def test_text_output_matches_send_input_wording(
        self, runner, session_id, happy_path_agent, bash_recorder
    ):
        result = _invoke(runner, session_id, "git status --short")

        assert result.exit_code == 0, result.output
        expected = (
            f"Sent bash command 'git status --short' to member "
            f"{MEMBER_NAME} ({PANE_ID})."
        )
        assert expected in (result.output or "")

    def test_dispatched_even_when_settings_deny_would_match(
        self,
        runner,
        session_id,
        happy_path_agent,
        bash_recorder,
        settings_files,
    ):
        """Bare-dispatch contract: ``exec`` does not consult ``settings.json``.

        A deny pattern that ``safe-exec`` would honor (and exit 2) MUST NOT
        block ``exec``; the keystroke is dispatched and exit 0 is returned.
        """
        _write_settings(settings_files[0], deny=["Bash(rm -rf *)"])

        result = _invoke(runner, session_id, "rm -rf /tmp/foo")

        assert result.exit_code == 0, result.output
        assert len(bash_recorder) == 1
        assert bash_recorder[0]["command"] == "rm -rf /tmp/foo"

    def test_does_not_invoke_permissions_decide(
        self,
        runner,
        session_id,
        happy_path_agent,
        bash_recorder,
        monkeypatch,
    ):
        """``member exec`` must not call ``permissions.decide`` at all."""
        called = {"count": 0}

        def fail(*_a, **_kw):
            called["count"] += 1
            raise AssertionError("exec must not call permissions.decide")

        monkeypatch.setattr(permissions, "decide", fail, raising=False)

        result = _invoke(runner, session_id, "git status")

        assert result.exit_code == 0, result.output
        assert called["count"] == 0
        assert len(bash_recorder) == 1


# ---------------------------------------------------------------------------
# Pre-flight validation
# ---------------------------------------------------------------------------


class TestExecPreflightValidation:
    def test_empty_string_rejected(
        self, runner, session_id, happy_path_agent, bash_recorder
    ):
        result = _invoke(runner, session_id, "")

        assert result.exit_code == 2, result.output
        assert "command may not be empty." in (result.output or "")
        assert len(bash_recorder) == 0

    @pytest.mark.parametrize("ws", ["   ", "\t", " \t ", "\t\t"])
    def test_whitespace_only_rejected(
        self, runner, session_id, happy_path_agent, bash_recorder, ws
    ):
        """Post-strip empty CMD — guards against whitespace-only dispatch."""
        result = _invoke(runner, session_id, ws)

        assert result.exit_code == 2, result.output
        assert "command may not be empty." in (result.output or "")
        assert len(bash_recorder) == 0

    @pytest.mark.parametrize(
        "bad_text",
        [
            "line1\nline2",
            "trailing\n",
            "\nleading",
            "carriage\rreturn",
            "mixed\r\nCRLF",
        ],
    )
    def test_newline_or_carriage_return_rejected(
        self,
        runner,
        session_id,
        happy_path_agent,
        bash_recorder,
        bad_text,
    ):
        result = _invoke(runner, session_id, bad_text)

        assert result.exit_code == 2, result.output
        assert "command may not contain newlines." in (result.output or "")
        assert len(bash_recorder) == 0

    def test_missing_command_argument_exits_two(
        self, runner, session_id, happy_path_agent, bash_recorder
    ):
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "exec",
                "--agent-id",
                DIRECTOR_ID,
                "--member-id",
                MEMBER_ID,
            ],
        )

        assert result.exit_code == 2, result.output
        out = result.output or ""
        assert "Missing argument" in out or "COMMAND" in out
        assert len(bash_recorder) == 0


# ---------------------------------------------------------------------------
# Authorization boundary
# ---------------------------------------------------------------------------


class TestExecAuthorizationBoundary:
    def test_cross_director_exits_one_with_existing_wording(
        self, runner, session_id, monkeypatch, bash_recorder
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(
                placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
            ),
        )

        result = _invoke(runner, session_id, "git status")

        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out
        assert "is not a member of your team" in out
        assert OTHER_DIRECTOR_ID in out
        assert len(bash_recorder) == 0

    def test_pending_pane_exits_one_with_existing_wording(
        self, runner, session_id, monkeypatch, bash_recorder
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
        )

        result = _invoke(runner, session_id, "git status")

        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert f"member {MEMBER_ID}" in out
        assert "has no pane yet" in out
        assert "pending placement" in out
        assert len(bash_recorder) == 0


# ---------------------------------------------------------------------------
# JSON output (--json)
# ---------------------------------------------------------------------------


class TestExecJsonOutput:
    def test_json_output_has_exactly_four_keys(
        self, runner, session_id, happy_path_agent, bash_recorder
    ):
        result = _invoke(runner, session_id, "git status --short", json_mode=True)

        assert result.exit_code == 0, result.output
        data = json.loads(result.output)
        assert set(data.keys()) == {
            "member_agent_id",
            "pane_id",
            "action",
            "value",
        }
        assert data["member_agent_id"] == MEMBER_ID
        assert data["pane_id"] == PANE_ID
        assert data["action"] == "bash"
        assert data["value"] == "git status --short"

    def test_json_output_preserves_command_verbatim(
        self, runner, session_id, happy_path_agent, bash_recorder
    ):
        """Shell meta and multi-byte chars survive verbatim in JSON ``value``."""
        payload = "echo $HOME && grep -F '日本語' file"
        result = _invoke(runner, session_id, payload, json_mode=True)

        assert result.exit_code == 0, result.output
        data = json.loads(result.output)
        assert data["value"] == payload


# ---------------------------------------------------------------------------
# tmux unavailable
# ---------------------------------------------------------------------------


class TestExecMissingTmux:
    def test_ensure_tmux_unavailable_exits_one(
        self, runner, session_id, happy_path_agent, bash_recorder, monkeypatch
    ):
        def boom():
            raise tmux.TmuxError(
                "cafleet member commands must be run inside a tmux session"
            )

        monkeypatch.setattr(tmux, "ensure_tmux_available", boom)

        result = _invoke(runner, session_id, "git status")

        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert "cafleet member commands must be run inside a tmux session" in out
        assert len(bash_recorder) == 0


# ---------------------------------------------------------------------------
# tmux send failure
# ---------------------------------------------------------------------------


class TestExecSendFailure:
    def test_send_bash_command_failure_exits_one(
        self, runner, session_id, happy_path_agent, monkeypatch
    ):
        def boom(**_kwargs):
            raise tmux.TmuxError("pane vanished")

        monkeypatch.setattr(tmux, "send_bash_command", boom, raising=False)

        result = _invoke(runner, session_id, "git status")

        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert "send failed:" in out
