"""Tests for ``output.render_agent`` (design doc 0000049, Surface 18, Step 16).

The ``render_agent`` projection mirrors ``render_task``: by default it
returns only the fields the Director's monitoring loop and an agent's own
identity check actually need; ``full=True`` returns the un-projected dict.
The slim shape drops ~250 tokens / agent / list call from
``cafleet agent list``'s wire cost.

Slim shape (default):
    ``id`` — 8-char prefix of ``agent_id`` (matches ``render_task``)
    ``name`` — full
    ``description`` — truncated to 60 codepoints + ``…`` suffix
    ``status`` — full
    ``coding_agent`` — projected from ``placement.coding_agent`` when
        the source dict has a placement; absent when it does not (so
        ``cafleet agent list``, which today returns no placement, stays
        backwards-compatible).
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker, config, output
from cafleet.cli import cli
from cafleet.db import engine as engine_mod
from cafleet.tmux import DirectorContext


_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


# ============================================================================
#                              Direct unit tests
# ============================================================================


def _sample_agent_with_placement() -> dict:
    """Mirrors what ``broker.get_agent`` returns (with placement)."""
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
    """Mirrors what ``broker.list_agents`` returns (no placement)."""
    return {
        "agent_id": "abcdef12-3456-7890-abcd-ef1234567890",
        "name": "Worker-A",
        "description": "test agent description",
        "status": "active",
        "registered_at": "2026-05-05T10:00:00.000000+00:00",
    }


def test_render_agent_slim__projects_id_to_8_char_prefix():
    """The slim shape uses ``id`` (8-char prefix) — the same naming /
    prefix-rendering convention ``render_task`` already uses."""
    rendered = output.render_agent(_sample_agent_with_placement(), full=False)
    assert rendered["id"] == "abcdef12"
    # The full UUID MUST NOT leak through under any other key — that would
    # defeat the byte-cost reduction the projection exists to deliver.
    serialized = json.dumps(rendered)
    assert "abcdef12-3456-7890-abcd-ef1234567890" not in serialized


def test_render_agent_slim__keeps_name_status():
    rendered = output.render_agent(_sample_agent_with_placement(), full=False)
    assert rendered["name"] == "Worker-A"
    assert rendered["status"] == "active"


def test_render_agent_slim__truncates_description_to_60_codepoints():
    """``description`` is truncated to the same 60-codepoint cap that
    ``format_agent(full=True)`` uses (per Surface 5 + Surface 18). A 200-
    char description is cut to 60 chars + the single-codepoint ``…``."""
    agent = _sample_agent_with_placement()
    agent["description"] = "x" * 200

    rendered = output.render_agent(agent, full=False)

    # 60 ``x`` characters + the 1-codepoint ``…`` suffix.
    assert rendered["description"] == "x" * 60 + "…"


def test_render_agent_slim__short_description_passes_through_untruncated():
    agent = _sample_agent_with_placement()
    agent["description"] = "concise"

    rendered = output.render_agent(agent, full=False)

    assert rendered["description"] == "concise"


def test_render_agent_slim__projects_coding_agent_from_placement():
    """``coding_agent`` lives at ``placement.coding_agent`` in the broker
    dict; the projection promotes it to a top-level field so monitoring
    consumers can branch without diving into a nested object."""
    agent = _sample_agent_with_placement()
    agent["placement"]["coding_agent"] = "codex"

    rendered = output.render_agent(agent, full=False)

    assert rendered["coding_agent"] == "codex"


def test_render_agent_slim__no_placement_omits_coding_agent():
    """When the source dict has no ``placement`` key (the
    ``broker.list_agents`` shape), the projection MUST NOT inject a
    fabricated ``coding_agent``. Omission is the only honest projection."""
    rendered = output.render_agent(_sample_agent_without_placement(), full=False)

    assert "coding_agent" not in rendered


def test_render_agent_slim__omits_legacy_keys():
    """Keys that the slim shape drops ``MUST`` be absent — verifies the
    projection does not silently smuggle the full ``agent_id``,
    ``registered_at``, ``kind``, or the nested ``placement`` blob through."""
    rendered = output.render_agent(_sample_agent_with_placement(), full=False)

    assert "agent_id" not in rendered
    assert "registered_at" not in rendered
    assert "kind" not in rendered
    assert "placement" not in rendered


def test_render_agent_full__returns_dict_unchanged():
    """``full=True`` is the escape hatch for operators who need the legacy
    shape (and are paying the cost knowingly). The dict returned MUST
    carry every key from the source, including the nested placement."""
    agent = _sample_agent_with_placement()

    rendered = output.render_agent(agent, full=True)

    for key in agent:
        assert key in rendered
    assert rendered["agent_id"] == agent["agent_id"]
    assert rendered["placement"] == agent["placement"]


def test_render_agent_slim__no_description_handled_gracefully():
    """Sanity check on the truncation path: a missing or empty description
    should not crash the projection. ``broker.list_agents`` always supplies
    one today, but the projection should not depend on that invariant."""
    agent = _sample_agent_without_placement()
    agent["description"] = ""

    rendered = output.render_agent(agent, full=False)

    # Empty string passes through untruncated.
    assert rendered["description"] == ""


# ============================================================================
#                          Integration tests via CLI
# ============================================================================


@pytest.fixture
def _reset_engine():
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None
    yield
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None


@pytest.fixture
def bootstrapped_session(tmp_path, monkeypatch, _reset_engine):
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
    return data["session_id"], data["director"]["agent_id"], runner


def test_cli_agent_show_default_json__returns_slim_shape(bootstrapped_session):
    """``cafleet agent show --json`` MUST emit the slim render_agent shape
    by default. The full ``agent_id`` UUID does not appear (it's prefixed
    to ``id``); ``placement`` and ``registered_at`` are omitted."""
    sid, director_id, runner = bootstrapped_session

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "agent",
            "show",
            "--agent-id",
            director_id,
            "--id",
            director_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)

    assert payload["id"] == director_id[:8]
    assert "agent_id" not in payload
    assert "placement" not in payload
    assert "registered_at" not in payload
    # Director registered with coding_agent="claude" via session create.
    assert payload["coding_agent"] == "claude"


def test_cli_agent_show_full_json__returns_full_shape(bootstrapped_session):
    """``--full`` returns the un-projected broker dict — full ``agent_id``,
    nested ``placement``, the lot."""
    sid, director_id, runner = bootstrapped_session

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "agent",
            "show",
            "--agent-id",
            director_id,
            "--id",
            director_id,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)

    assert payload["agent_id"] == director_id
    assert "placement" in payload
    assert payload["placement"]["coding_agent"] == "claude"


def test_cli_agent_list_default_json__returns_slim_rows(bootstrapped_session):
    """``cafleet agent list --json`` projects every row through render_agent.
    No row carries a full ``agent_id`` UUID; ``id`` (8-char prefix) is the
    only identity surface in the default shape."""
    sid, director_id, runner = bootstrapped_session

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "agent",
            "list",
            "--agent-id",
            director_id,
        ],
    )
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)

    for row in rows:
        assert "id" in row
        assert len(row["id"]) == 8
        assert "agent_id" not in row
        assert "registered_at" not in row


def test_cli_agent_list_full_json__returns_full_rows(bootstrapped_session):
    """``--full`` on the list endpoint restores the full broker shape."""
    sid, director_id, runner = bootstrapped_session

    result = runner.invoke(
        cli,
        [
            "--session-id",
            sid,
            "--json",
            "agent",
            "list",
            "--agent-id",
            director_id,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    rows = json.loads(result.output)

    for row in rows:
        assert "agent_id" in row
        assert len(row["agent_id"]) > 8  # full UUID, not the slim prefix
        assert "registered_at" in row
