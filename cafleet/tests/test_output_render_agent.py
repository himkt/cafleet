"""Tests for ``output.render_agent``."""

import json

import pytest
from click.testing import CliRunner

from cafleet import config, output
from cafleet.cli import cli
from cafleet.multiplexer import MultiplexerContext as DirectorContext

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


def _sample_agent_with_placement() -> dict:
    return {
        "agent_id": "abcdef12-3456-7890-abcd-ef1234567890",
        "name": "Worker-A",
        "description": "test agent description",
        "status": "active",
        "registered_at": "2026-05-05T10:00:00.000000+00:00",
        "kind": "user",
        "placement": {
            "director_agent_id": "12121212-1212-1212-1212-121212121212",
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": "%17",
            "coding_agent": "claude",
            "created_at": "2026-05-05T10:00:00.000000+00:00",
        },
    }


def _sample_agent_without_placement() -> dict:
    return {
        "agent_id": "abcdef12-3456-7890-abcd-ef1234567890",
        "name": "Worker-A",
        "description": "test agent description",
        "status": "active",
        "registered_at": "2026-05-05T10:00:00.000000+00:00",
    }


def test_render_agent_slim__shape_id_name_status_coding_agent_no_full_card_keys():
    rendered = output.render_agent(_sample_agent_with_placement(), full=False)
    assert rendered["id"] == "abcdef12"
    assert rendered["name"] == "Worker-A"
    assert rendered["status"] == "active"
    # Full UUID must NOT leak through any key.
    assert "abcdef12-3456-7890-abcd-ef1234567890" not in json.dumps(rendered)
    # coding_agent is projected from placement.
    assert rendered["coding_agent"] == "claude"
    # Full-card keys absent.
    for forbidden in ("agent_id", "registered_at", "kind", "placement"):
        assert forbidden not in rendered

    # No placement source → coding_agent absent.
    rendered_no_placement = output.render_agent(
        _sample_agent_without_placement(), full=False
    )
    assert "coding_agent" not in rendered_no_placement


@pytest.mark.parametrize(
    ("scenario", "description", "expected"),
    [
        ("truncates_at_60_plus_ellipsis", "x" * 200, "x" * 60 + "…"),
        ("short_unchanged", "concise", "concise"),
        ("empty_unchanged", "", ""),
    ],
)
def test_render_agent_slim__description_truncation(scenario, description, expected):
    agent = _sample_agent_with_placement()
    agent["description"] = description
    rendered = output.render_agent(agent, full=False)
    assert rendered["description"] == expected


def test_render_agent_full__returns_unchanged_source():
    agent = _sample_agent_with_placement()
    rendered = output.render_agent(agent, full=True)
    for key in agent:
        assert key in rendered
    assert rendered["agent_id"] == agent["agent_id"]
    assert rendered["placement"] == agent["placement"]


@pytest.fixture
def bootstrapped_session(tmp_path, monkeypatch, _reset_engine_singletons):
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available",
        lambda self: None,
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.context_discovery",
        lambda self: _FAKE_DIRECTOR_CTX,
    )

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0
    create = runner.invoke(cli, ["session", "create", "--json"])
    assert create.exit_code == 0
    data = json.loads(create.output)
    return data["session_id"], data["director"]["agent_id"], runner


@pytest.mark.parametrize(
    ("verb", "mode", "expect_slim_keys", "expect_full_uuid"),
    [
        ("show", "default", True, False),
        ("show", "full", False, True),
        ("list", "default", True, False),
        ("list", "full", False, True),
    ],
)
def test_cli_agent_show_or_list__slim_and_full_shapes(
    bootstrapped_session, verb, mode, expect_slim_keys, expect_full_uuid
):
    sid, director_id, runner = bootstrapped_session
    args = ["--session-id", sid, "--json", "agent", verb, "--agent-id", director_id]
    if verb == "show":
        args.extend(["--id", director_id])
    if mode == "full":
        args.append("--full")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    rows = payload if isinstance(payload, list) else [payload]
    for row in rows:
        if expect_slim_keys:
            assert "id" in row
            assert len(row["id"]) == 8
            assert "agent_id" not in row
            assert "registered_at" not in row
            if verb == "show":
                assert "placement" not in row
                assert row["coding_agent"] == "claude"
        if expect_full_uuid:
            assert "agent_id" in row
            assert len(row["agent_id"]) > 8
            assert "registered_at" in row
            if verb == "show":
                assert "placement" in row
                assert row["placement"]["coding_agent"] == "claude"
