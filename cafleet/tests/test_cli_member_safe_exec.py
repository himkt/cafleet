"""CLI tests for ``cafleet member safe-exec`` (design doc 0000037)."""

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
    """Three isolated settings files patched into ``permissions.discover_settings_paths``."""
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
            "safe-exec",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
        ]
    )
    args.extend(extra_args)
    return runner.invoke(cli, args)


# ---------------------------------------------------------------------------
# Allow path
# ---------------------------------------------------------------------------


class TestAllowPath:
    def test_dispatches_via_send_bash_command(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], allow=["Bash(git status:*)"])

        result = _invoke(runner, session_id, "--bash", "git status --short")

        assert result.exit_code == 0, result.output
        assert len(bash_recorder) == 1
        call = bash_recorder[0]
        assert call["target_pane_id"] == PANE_ID
        assert call["command"] == "git status --short"

    def test_text_output_matches_send_input_wording(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], allow=["Bash(git status:*)"])

        result = _invoke(runner, session_id, "--bash", "git status --short")

        assert result.exit_code == 0, result.output
        expected = (
            f"Sent bash command 'git status --short' to member "
            f"{MEMBER_NAME} ({PANE_ID})."
        )
        assert expected in (result.output or "")

    def test_allow_pattern_in_user_layer_takes_effect(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        # Only the user-layer file has the allow rule
        _write_settings(settings_files[2], allow=["Bash(git status:*)"])

        result = _invoke(runner, session_id, "--bash", "git status")

        assert result.exit_code == 0, result.output
        assert len(bash_recorder) == 1


# ---------------------------------------------------------------------------
# Deny path
# ---------------------------------------------------------------------------


class TestDenyPath:
    def test_deny_match_exits_two_and_does_not_dispatch(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], deny=["Bash(rm -rf *)"])

        result = _invoke(runner, session_id, "--bash", "rm -rf /tmp/x")

        assert result.exit_code == 2, result.output
        assert len(bash_recorder) == 0

    def test_deny_message_names_pattern_file_and_command(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], deny=["Bash(rm -rf *)"])

        result = _invoke(runner, session_id, "--bash", "rm -rf /tmp/x")

        out = result.output or ""
        assert "Bash(rm -rf *)" in out
        assert str(settings_files[0]) in out
        assert "rm -rf /tmp/x" in out

    def test_deny_wins_over_allow_when_both_match(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(
            settings_files[0],
            allow=["Bash(git push:*)"],
            deny=["Bash(git push:*)"],
        )

        result = _invoke(runner, session_id, "--bash", "git push origin main")

        assert result.exit_code == 2, result.output
        assert len(bash_recorder) == 0


# ---------------------------------------------------------------------------
# Ask path
# ---------------------------------------------------------------------------


class TestAskPath:
    def test_no_pattern_matches_exits_three(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], allow=["Bash(npm test:*)"])

        result = _invoke(runner, session_id, "--bash", "git status")

        assert result.exit_code == 3, result.output
        assert len(bash_recorder) == 0

    def test_ask_message_lists_three_searched_files(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        # All three files left unwritten — ask path
        result = _invoke(runner, session_id, "--bash", "git status")

        assert result.exit_code == 3, result.output
        out = result.output or ""
        for path in settings_files:
            assert str(path) in out

    def test_ask_message_suggests_first_token_pattern(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        result = _invoke(runner, session_id, "--bash", "git status --short")

        assert result.exit_code == 3, result.output
        out = result.output or ""
        # Suggested pattern uses the first whitespace-delimited token
        assert "Bash(git:*)" in out


# ---------------------------------------------------------------------------
# Pre-flight validation
# ---------------------------------------------------------------------------


class TestPreflightValidation:
    def test_empty_string_rejected(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        result = _invoke(runner, session_id, "--bash", "")

        assert result.exit_code == 2, result.output
        assert "--bash command may not be empty." in (result.output or "")
        assert len(bash_recorder) == 0

    @pytest.mark.parametrize(
        "bad_text",
        [
            "foo\nbar",
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
        settings_files,
        bad_text,
    ):
        result = _invoke(runner, session_id, "--bash", bad_text)

        assert result.exit_code == 2, result.output
        assert "--bash command may not contain newlines." in (result.output or "")
        assert len(bash_recorder) == 0

    def test_missing_bash_flag_exits_two(
        self, runner, session_id, happy_path_agent, settings_files
    ):
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "safe-exec",
                "--agent-id",
                DIRECTOR_ID,
                "--member-id",
                MEMBER_ID,
            ],
        )

        assert result.exit_code == 2, result.output
        assert "--bash" in (result.output or "")


# ---------------------------------------------------------------------------
# Authorization boundary
# ---------------------------------------------------------------------------


class TestAuthorizationBoundary:
    def test_cross_director_exits_one_with_existing_wording(
        self, runner, session_id, monkeypatch, bash_recorder, settings_files
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(
                placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
            ),
        )

        result = _invoke(runner, session_id, "--bash", "git status")

        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out
        assert "is not a member of your team" in out
        assert OTHER_DIRECTOR_ID in out
        assert len(bash_recorder) == 0

    def test_pending_pane_exits_one_with_existing_wording(
        self, runner, session_id, monkeypatch, bash_recorder, settings_files
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
        )

        result = _invoke(runner, session_id, "--bash", "git status")

        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert f"member {MEMBER_ID}" in out
        assert "has no pane yet" in out
        assert "pending placement" in out
        assert len(bash_recorder) == 0


# ---------------------------------------------------------------------------
# JSON output (--json)
# ---------------------------------------------------------------------------


_JSON_KEYS = {
    "outcome",
    "matched_pattern",
    "matched_file",
    "offending_substring",
    "searched_files",
}


def _extract_json(result) -> dict:
    """Find and parse the JSON payload from CliRunner ``result.output``."""
    text = (result.output or "").strip()
    # When CliRunner's default ``mix_stderr`` is on, stderr lines may
    # appear before the JSON — pick the JSON object substring.
    start = text.find("{")
    end = text.rfind("}")
    assert start != -1, f"no opening brace in output: {text!r}"
    assert end != -1, f"no closing brace in output: {text!r}"
    return json.loads(text[start : end + 1])


class TestJsonOutput:
    def test_json_allow_has_exactly_five_keys(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], allow=["Bash(git status:*)"])

        result = _invoke(
            runner, session_id, "--bash", "git status --short", json_mode=True
        )

        assert result.exit_code == 0, result.output
        data = _extract_json(result)
        assert set(data.keys()) == _JSON_KEYS
        assert data["outcome"] == "allow"
        assert data["matched_pattern"] == "Bash(git status:*)"
        assert data["matched_file"] == str(settings_files[0])
        assert data["offending_substring"] == "git status --short"
        assert data["searched_files"] == [str(p) for p in settings_files]

    def test_json_deny_has_exactly_five_keys(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        _write_settings(settings_files[0], deny=["Bash(rm -rf *)"])

        result = _invoke(runner, session_id, "--bash", "rm -rf /tmp/x", json_mode=True)

        assert result.exit_code == 2, result.output
        data = _extract_json(result)
        assert set(data.keys()) == _JSON_KEYS
        assert data["outcome"] == "deny"
        assert data["matched_pattern"] == "Bash(rm -rf *)"
        assert data["matched_file"] == str(settings_files[0])
        assert data["offending_substring"] == "rm -rf /tmp/x"
        assert data["searched_files"] == [str(p) for p in settings_files]

    def test_json_ask_has_exactly_five_keys_with_nulls(
        self, runner, session_id, happy_path_agent, bash_recorder, settings_files
    ):
        # No settings files written → ask
        result = _invoke(runner, session_id, "--bash", "git status", json_mode=True)

        assert result.exit_code == 3, result.output
        data = _extract_json(result)
        assert set(data.keys()) == _JSON_KEYS
        assert data["outcome"] == "ask"
        assert data["matched_pattern"] is None
        assert data["matched_file"] is None
        assert data["offending_substring"] is None
        assert data["searched_files"] == [str(p) for p in settings_files]
