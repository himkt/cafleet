"""CLI-level tests for the fleet-bootstrap surface."""

import json
import sqlite3

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxError
from tests._helpers import _init_registry
from tests.cli.conftest import _FAKE_DIRECTOR_CTX


@pytest.fixture
def db_file(tmp_path, monkeypatch):
    path = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{path}",
    )
    _init_registry()
    return path


@pytest.fixture
def mock_tmux_ok(_mock_tmux_for_fleet_create):
    """Alias for the conftest tmux stub."""


@pytest.fixture
def mock_tmux_unavailable(monkeypatch):
    def _raise(self):
        raise TmuxError("cafleet member commands must be run inside a tmux session")

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available", _raise
    )


def _fleet_rows(db_path) -> list[tuple]:
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT fleet_id, name, created_at, deleted_at, director_member_id "
            "FROM fleets"
        ).fetchall()
    finally:
        conn.close()


def test_fleet_create_text_output__compact_default_emits_fleet_id_and_full_ids(
    db_file, mock_tmux_ok
):
    """Default text output is 1 line: ``<fleet_id> director=<id>``."""
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot"])
    assert result.exit_code == 0, result.output

    out = result.output.strip()
    assert "\n" not in out, f"compact output must be 1 line; got:\n{result.output}"

    parts = out.split()
    assert len(parts) == 2
    sid = parts[0]
    assert sid.isdigit()
    assert parts[1].startswith("director=")

    rows = _fleet_rows(db_file)
    assert any(str(r[0]) == sid for r in rows)


def test_fleet_create_text_output__full_flag_emits_verbose_6_line_layout(
    db_file, mock_tmux_ok
):
    """``--full`` emits the verbose 6-line layout with field labels."""
    runner = CliRunner()
    result = runner.invoke(
        cli, ["fleet", "create", "--name", "bootstrap-check", "--full"]
    )
    assert result.exit_code == 0

    text = result.output
    assert "name:" in text
    assert "director_name:" in text
    assert "pane:" in text

    assert "Director" in text
    assert "bootstrap-check" in text

    assert (
        f"{_FAKE_DIRECTOR_CTX.session}:{_FAKE_DIRECTOR_CTX.window_id}:"
        f"{_FAKE_DIRECTOR_CTX.pane_id}" in text
    )


def test_fleet_create_json_output__top_level_keys(db_file, mock_tmux_ok):
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    assert result.exit_code == 0, result.output

    data = json.loads(result.output)
    assert set(data) == {"fleet_id", "name", "created_at", "director"}


def test_fleet_create_json_output__director_sub_dict(db_file, mock_tmux_ok):
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    director = data["director"]
    for key in ("member_id", "name", "description", "registered_at", "placement"):
        assert key in director
    assert director["name"] == "Director"
    assert director["description"] == "Root Director for this fleet"
    assert isinstance(director["member_id"], int)


def test_fleet_create_json_output__placement_sub_dict_matches_spec(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    placement = data["director"]["placement"]

    # Default --coding-agent is 'claude'.
    assert placement["coding_agent"] == "claude"
    assert placement["backend"] == "tmux"
    assert placement["mux_session"] == _FAKE_DIRECTOR_CTX.session
    assert placement["mux_window_id"] == _FAKE_DIRECTOR_CTX.window_id
    assert placement["mux_pane_id"] == _FAKE_DIRECTOR_CTX.pane_id
    assert "created_at" in placement


# --- fleet_create_coding_agent: ``--coding-agent`` on
# ``cafleet fleet create`` is operator-declared metadata that lands in
# ``placement.coding_agent`` for the root Director. The fleet-create path
# does NOT spawn anything, so the test does not need a fake codex binary.
# ---


def test_fleet_create_coding_agent__codex_records_codex_in_placement_text(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    result = runner.invoke(
        cli, ["fleet", "create", "--name", "boot", "--coding-agent", "codex", "--json"]
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["director"]["placement"]["coding_agent"] == "codex"


def test_fleet_create_coding_agent__claude_records_claude_in_placement(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    result = runner.invoke(
        cli, ["fleet", "create", "--name", "boot", "--coding-agent", "claude", "--json"]
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["director"]["placement"]["coding_agent"] == "claude"


def test_fleet_create_coding_agent__default_is_claude(db_file, mock_tmux_ok):
    """No ``--coding-agent`` flag defaults the placement to ``'claude'``."""
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["director"]["placement"]["coding_agent"] == "claude"


def test_fleet_create_coding_agent__unknown_value_rejected(db_file, mock_tmux_ok):
    """``click.Choice(['claude','codex'])`` rejects anything else (exit 2)."""
    runner = CliRunner()
    result = runner.invoke(
        cli, ["fleet", "create", "--name", "boot", "--coding-agent", "foo"]
    )
    assert result.exit_code == 2, result.output
    assert "--coding-agent" in (result.output or "")


def test_fleet_create_json_output__name_propagates_to_json(db_file, mock_tmux_ok):
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "json-name", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["name"] == "json-name"


def test_fleet_create_outside_tmux__fails_with_specific_error_and_exit_1(
    db_file, mock_tmux_unavailable
):
    runner = CliRunner()
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot"])
    assert result.exit_code == 1, result.output
    combined = (result.output or "") + (result.stderr or "")
    assert "cafleet fleet create must be run inside a tmux or herdr session" in combined


def test_fleet_create_outside_tmux__no_fleet_row_is_written_when_tmux_is_missing(
    db_file, mock_tmux_unavailable
):
    runner = CliRunner()
    before = _fleet_rows(db_file)
    result = runner.invoke(cli, ["fleet", "create", "--name", "boot"])
    assert result.exit_code == 1, result.output
    after = _fleet_rows(db_file)
    assert before == after


def test_fleet_list_hides_soft_deleted__deleted_fleet_is_hidden_from_text_list(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    r1 = runner.invoke(cli, ["fleet", "create", "--name", "keep", "--json"])
    r2 = runner.invoke(cli, ["fleet", "create", "--name", "drop", "--json"])
    assert r1.exit_code == 0
    assert r2.exit_code == 0
    keep_sid = json.loads(r1.output)["fleet_id"]
    drop_sid = json.loads(r2.output)["fleet_id"]

    del_result = runner.invoke(cli, ["fleet", "delete", "--fleet-id", str(drop_sid)])
    assert del_result.exit_code == 0, del_result.output

    list_result = runner.invoke(cli, ["fleet", "list"])
    assert list_result.exit_code == 0
    data_lines = list_result.output.splitlines()[1:]
    fleet_id_column = {line.split()[0] for line in data_lines if line.split()}
    assert str(keep_sid) in fleet_id_column
    assert str(drop_sid) not in fleet_id_column


def test_fleet_list_hides_soft_deleted__deleted_fleet_is_hidden_from_json_list(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    r1 = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    r2 = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    keep_sid = json.loads(r1.output)["fleet_id"]
    drop_sid = json.loads(r2.output)["fleet_id"]
    runner.invoke(cli, ["fleet", "delete", "--fleet-id", str(drop_sid)])

    list_result = runner.invoke(cli, ["fleet", "list", "--json"])
    assert list_result.exit_code == 0
    data = json.loads(list_result.output)
    ids = {s["fleet_id"] for s in data}
    assert keep_sid in ids
    assert drop_sid not in ids


def test_fleet_delete_unknown_and_idempotent__unknown_fleet_id_exits_1_with_not_found(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    fake = 999
    result = runner.invoke(cli, ["fleet", "delete", "--fleet-id", str(fake)])
    assert result.exit_code == 1, result.output
    combined = ((result.output or "") + (result.stderr or "")).lower()
    assert "not found" in combined
    assert str(fake) in (result.output or "") + (result.stderr or "")


def test_fleet_delete_unknown_and_idempotent__second_delete_is_idempotent_and_reports_zero(
    db_file, mock_tmux_ok
):
    runner = CliRunner()
    r = runner.invoke(cli, ["fleet", "create", "--name", "boot", "--json"])
    assert r.exit_code == 0
    sid = json.loads(r.output)["fleet_id"]

    first = runner.invoke(cli, ["fleet", "delete", "--fleet-id", str(sid)])
    second = runner.invoke(cli, ["fleet", "delete", "--fleet-id", str(sid)])

    assert first.exit_code == 0, first.output
    assert second.exit_code == 0, second.output
    assert "0 members" in second.output
