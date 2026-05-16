"""Unit and CLI tests for cafleet.base_dir (design 0000055)."""

import json
import re
from pathlib import Path

import pytest
from click.testing import CliRunner

from cafleet import base_dir as base_dir_module
from cafleet.base_dir import (
    ANCHOR_FILENAME,
    AnchorError,
    record,
    resolve,
)
from cafleet.cli import cli


@pytest.fixture
def runner():
    return CliRunner()


def _write_anchor(
    path: Path,
    *,
    base: str,
    source: str = "askuserquestion",
    version: int = 1,
    resolved_at: str = "2026-05-12T13:00:00.000000+00:00",
) -> None:
    path.write_text(
        json.dumps(
            {
                "version": version,
                "base": base,
                "source": source,
                "resolved_at": resolved_at,
            }
        )
    )


# ---------------------------------------------------------------------------
# Unit tests — resolve()
# ---------------------------------------------------------------------------


def test_resolve_cwd_outside_home_returns_cwd(tmp_path):
    """CWD-deterministic branch: CWD outside HOME returns CWD as BASE."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "proj"
    proj.mkdir()

    result = resolve(cwd=proj, home=home)

    assert result["status"] == "resolved"
    assert result["base"] == str(proj)
    assert result["source"] == "cwd-inference"
    assert result["anchor"] == str(proj / ANCHOR_FILENAME)


def test_resolve_cwd_outside_home_writes_anchor_with_source_cwd_inference(tmp_path):
    """The CWD-inference branch persists an anchor with source='cwd-inference'."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "proj"
    proj.mkdir()

    resolve(cwd=proj, home=home)

    anchor_path = proj / ANCHOR_FILENAME
    assert anchor_path.exists()
    data = json.loads(anchor_path.read_text())
    assert data["version"] == 1
    assert data["base"] == str(proj)
    assert data["source"] == "cwd-inference"
    assert re.match(
        r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}\+00:00$",
        data["resolved_at"],
    ), f"resolved_at not ISO 8601 microsecond+00:00: {data['resolved_at']!r}"


def test_resolve_cwd_outside_home_reads_existing_anchor(tmp_path):
    """If an anchor already exists at CWD and matches, resolve returns source='anchor'."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "proj"
    proj.mkdir()
    _write_anchor(proj / ANCHOR_FILENAME, base=str(proj))

    result = resolve(cwd=proj, home=home)

    assert result["status"] == "resolved"
    assert result["base"] == str(proj)
    assert result["source"] == "anchor"
    assert result["anchor"] == str(proj / ANCHOR_FILENAME)


def test_resolve_anchor_mismatch_raises(tmp_path):
    """If the anchor's 'base' field does not match its parent directory, resolve aborts."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "proj"
    proj.mkdir()
    _write_anchor(proj / ANCHOR_FILENAME, base="/elsewhere")

    with pytest.raises(AnchorError, match=r"records base="):
        resolve(cwd=proj, home=home)


def test_resolve_cwd_under_home_dot_claude_returns_needs_user_input(
    tmp_path, monkeypatch
):
    """CWD under $HOME/.claude triggers the AskUserQuestion branch."""
    home = tmp_path / "home"
    home.mkdir()
    claude_dir = home / ".claude"
    claude_dir.mkdir()
    tmp_candidate = tmp_path / "tmp-claude-code"  # absent → no anchor probe hit

    result = resolve(cwd=claude_dir, home=home, tmp_candidate=tmp_candidate)

    assert result["status"] == "needs-user-input"
    assert result["base"] is None
    assert result["source"] is None
    assert str(tmp_candidate) in result["candidates"]
    assert str(claude_dir) in result["candidates"]


def test_resolve_cwd_under_home_dot_claude_probes_tmp_anchor_first(tmp_path):
    """When CWD is under $HOME/.claude and the tmp anchor exists, it wins."""
    home = tmp_path / "home"
    home.mkdir()
    claude_dir = home / ".claude"
    claude_dir.mkdir()
    tmp_candidate = tmp_path / "tmp-claude-code"
    tmp_candidate.mkdir()
    _write_anchor(tmp_candidate / ANCHOR_FILENAME, base=str(tmp_candidate))

    result = resolve(cwd=claude_dir, home=home, tmp_candidate=tmp_candidate)

    assert result["status"] == "resolved"
    assert result["base"] == str(tmp_candidate)
    assert result["source"] == "anchor"
    assert result["anchor"] == str(tmp_candidate / ANCHOR_FILENAME)


def test_resolve_cwd_equals_home_probes_anchors_in_order(tmp_path):
    """When CWD == HOME, probe /tmp/claude-code first, then CWD; both-present → tmp wins."""
    home = tmp_path / "home"
    home.mkdir()
    tmp_candidate = tmp_path / "tmp-claude-code"
    tmp_candidate.mkdir()

    # Stage A: no anchors → needs-user-input
    result = resolve(cwd=home, home=home, tmp_candidate=tmp_candidate)
    assert result["status"] == "needs-user-input"

    # Stage B: only CWD anchor → resolved from CWD anchor
    _write_anchor(home / ANCHOR_FILENAME, base=str(home))
    result = resolve(cwd=home, home=home, tmp_candidate=tmp_candidate)
    assert result["status"] == "resolved"
    assert result["base"] == str(home)
    assert result["source"] == "anchor"

    # Stage C: both anchors → /tmp anchor wins (probed first)
    _write_anchor(tmp_candidate / ANCHOR_FILENAME, base=str(tmp_candidate))
    result = resolve(cwd=home, home=home, tmp_candidate=tmp_candidate)
    assert result["status"] == "resolved"
    assert result["base"] == str(tmp_candidate)
    assert result["source"] == "anchor"


# ---------------------------------------------------------------------------
# Unit tests — resolve() task-scope branch (design 0000060)
# ---------------------------------------------------------------------------


def _make_repo_root(tmp_path: Path) -> Path:
    """Create a fake git repo root under ``tmp_path`` and return it."""
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    (repo_root / ".git").mkdir()
    return repo_root


def test_resolve_relative_task_name_creates_folder_and_writes_anchor(tmp_path):
    """Bullet 1: A relative ``task_name`` resolves under the inferred repo root, auto-creates
    the folder, and writes a task-scope anchor."""
    repo_root = _make_repo_root(tmp_path)
    task_name = "design-docs/0000099-test-thing"

    result = resolve(task_name=task_name, cwd=repo_root)

    task_folder = repo_root / "design-docs" / "0000099-test-thing"
    assert task_folder.is_dir(), "task folder must be auto-created"
    assert result == {
        "status": "resolved",
        "base": str(task_folder),
        "source": "task-scope",
        "anchor": str(task_folder / ANCHOR_FILENAME),
        "task_name": task_name,
    }
    anchor_data = json.loads((task_folder / ANCHOR_FILENAME).read_text())
    assert anchor_data["version"] == 1
    assert anchor_data["base"] == str(task_folder)
    assert anchor_data["source"] == "task-scope"
    assert re.match(
        r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}\+00:00$",
        anchor_data["resolved_at"],
    ), f"resolved_at not ISO 8601 microsecond+00:00: {anchor_data['resolved_at']!r}"


def test_resolve_relative_task_name_reads_anchor_on_second_call(tmp_path):
    """Bullet 2: Re-invoking with the same ``task_name`` re-reads the existing anchor
    (``source='anchor'``)."""
    repo_root = _make_repo_root(tmp_path)
    task_name = "researches/topic-foo"

    first = resolve(task_name=task_name, cwd=repo_root)
    assert first["source"] == "task-scope"

    second = resolve(task_name=task_name, cwd=repo_root)
    assert second == {
        "status": "resolved",
        "base": first["base"],
        "source": "anchor",
        "anchor": first["anchor"],
        "task_name": task_name,
    }


def test_resolve_absolute_path_inside_researches_resolves_to_slug_folder(tmp_path):
    """Bullet 3: An absolute ``task_name`` whose path lives inside
    ``researches/<slug>/...`` resolves to that slug folder (not the inner file)."""
    repo_root = _make_repo_root(tmp_path)
    research_folder = repo_root / "researches" / "topic-foo"
    research_folder.mkdir(parents=True)
    inner = research_folder / "report.md"

    result = resolve(task_name=str(inner), cwd=repo_root)

    assert result == {
        "status": "resolved",
        "base": str(research_folder),
        "source": "task-scope",
        "anchor": str(research_folder / ANCHOR_FILENAME),
        "task_name": str(inner),
    }


def test_resolve_absolute_path_inside_design_docs_resolves_to_slug_folder(tmp_path):
    """Bullet 4: An absolute ``task_name`` whose path lives inside
    ``design-docs/<NNNNNNN>-<slug>/...`` resolves to that slug folder."""
    repo_root = _make_repo_root(tmp_path)
    dd_folder = repo_root / "design-docs" / "0000060-skill-task-scoped-base-dir"
    dd_folder.mkdir(parents=True)
    inner = dd_folder / "design-doc.md"

    result = resolve(task_name=str(inner), cwd=repo_root)

    assert result == {
        "status": "resolved",
        "base": str(dd_folder),
        "source": "task-scope",
        "anchor": str(dd_folder / ANCHOR_FILENAME),
        "task_name": str(inner),
    }


def test_resolve_absolute_path_outside_recognized_pattern_returns_unset(tmp_path):
    """Bullet 5: An absolute ``task_name`` outside any recognized task-folder pattern
    returns the ``<unset>`` shape (existing absolute-path-arg semantics preserved)."""
    repo_root = _make_repo_root(tmp_path)
    outside = tmp_path / "outside" / "random.md"

    result = resolve(task_name=str(outside), cwd=repo_root)

    assert result == {
        "status": "unset",
        "base": None,
        "source": "absolute-path-arg",
        "anchor": None,
        "task_name": str(outside),
    }


def test_resolve_absolute_path_with_bad_design_docs_slug_returns_unset(tmp_path):
    """Bullet 6: ``design-docs/garbage`` fails the ``^\\d{7}-`` slug regex in
    ``_match_known_task_pattern`` and produces no anchor."""
    repo_root = _make_repo_root(tmp_path)
    bad_folder = repo_root / "design-docs" / "garbage"
    bad_folder.mkdir(parents=True)
    inner = bad_folder / "doc.md"

    result = resolve(task_name=str(inner), cwd=repo_root)

    assert result == {
        "status": "unset",
        "base": None,
        "source": "absolute-path-arg",
        "anchor": None,
        "task_name": str(inner),
    }
    assert not (bad_folder / ANCHOR_FILENAME).exists()


# ---------------------------------------------------------------------------
# Unit tests — record()
# ---------------------------------------------------------------------------


def test_record_writes_well_formed_json(tmp_path):
    """record() writes <base>/.cafleet-base-dir.json with version=1 and ISO 8601 timestamp."""
    base = tmp_path / "proj"
    base.mkdir()

    anchor = record(str(base), source="askuserquestion")

    assert anchor == base / ANCHOR_FILENAME
    data = json.loads(anchor.read_text())
    assert data["version"] == 1
    assert data["base"] == str(base)
    assert data["source"] == "askuserquestion"
    assert re.match(
        r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}\+00:00$",
        data["resolved_at"],
    ), f"resolved_at not ISO 8601 microsecond+00:00: {data['resolved_at']!r}"


def test_record_is_idempotent_when_matching(tmp_path):
    """Re-recording with the same base on top of a matching anchor is a no-op."""
    base = tmp_path / "proj"
    base.mkdir()

    first = record(str(base), source="askuserquestion")
    first_payload = json.loads(first.read_text())

    # Second call with same base/source should succeed and not raise
    second = record(str(base), source="askuserquestion")
    second_payload = json.loads(second.read_text())

    assert first == second
    assert first_payload["base"] == second_payload["base"]
    assert first_payload["version"] == second_payload["version"]


def test_record_errors_on_mismatch(tmp_path):
    """Recording onto an existing anchor whose 'base' field disagrees raises AnchorError."""
    base = tmp_path / "proj"
    base.mkdir()
    _write_anchor(base / ANCHOR_FILENAME, base="/elsewhere")

    with pytest.raises(AnchorError, match=r"records base="):
        record(str(base), source="askuserquestion")


def test_record_rejects_non_absolute_base(tmp_path):
    """record() rejects relative paths — BASE must always be absolute."""
    with pytest.raises((ValueError, AnchorError), match=r"[Aa]bsolute"):
        record("relative/path", source="askuserquestion")


# ---------------------------------------------------------------------------
# CLI tests
# ---------------------------------------------------------------------------


def test_cli_resolve_emits_documented_json_shape(tmp_path, runner, monkeypatch):
    """`cafleet base-dir resolve --json` emits the documented JSON for each status branch."""
    home = tmp_path / "home"
    home.mkdir()
    fake_tmp_candidate = tmp_path / "tmp-claude-code"

    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setattr(base_dir_module, "_TMP_CANDIDATE", fake_tmp_candidate)

    # Branch 1: cwd-inference (first call, no anchor)
    proj = tmp_path / "proj"
    proj.mkdir()
    monkeypatch.chdir(proj)
    result = runner.invoke(cli, ["base-dir", "resolve", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data == {
        "status": "resolved",
        "base": str(proj),
        "source": "cwd-inference",
        "anchor": str(proj / ANCHOR_FILENAME),
    }

    # Branch 2: anchor (second call, anchor now exists)
    result = runner.invoke(cli, ["base-dir", "resolve", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data == {
        "status": "resolved",
        "base": str(proj),
        "source": "anchor",
        "anchor": str(proj / ANCHOR_FILENAME),
    }

    # Branch 3: needs-user-input (CWD == HOME, no anchors)
    monkeypatch.chdir(home)
    result = runner.invoke(cli, ["base-dir", "resolve", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["status"] == "needs-user-input"
    assert data["base"] is None
    assert data["source"] is None
    assert isinstance(data["candidates"], list)
    assert str(fake_tmp_candidate) in data["candidates"]


def test_cli_record_writes_anchor_or_errors_on_mismatch(tmp_path, runner):
    """`cafleet base-dir record` writes the anchor; idempotent on match; errors on mismatch."""
    proj = tmp_path / "proj"
    proj.mkdir()

    # First call: anchor lands with the expected JSON shape
    result = runner.invoke(
        cli,
        [
            "base-dir",
            "record",
            "--base",
            str(proj),
            "--source",
            "askuserquestion",
        ],
    )
    assert result.exit_code == 0, result.output
    anchor = proj / ANCHOR_FILENAME
    assert anchor.exists()
    data = json.loads(anchor.read_text())
    assert data["version"] == 1
    assert data["base"] == str(proj)
    assert data["source"] == "askuserquestion"

    # Idempotent: re-invoking with the same args is a successful no-op
    result = runner.invoke(
        cli,
        [
            "base-dir",
            "record",
            "--base",
            str(proj),
            "--source",
            "askuserquestion",
        ],
    )
    assert result.exit_code == 0, result.output

    # Mismatched base: pre-existing anchor records a different `base` field
    other = tmp_path / "other"
    other.mkdir()
    _write_anchor(
        other / ANCHOR_FILENAME, base=str(proj)
    )  # mismatch: anchor body != location

    result = runner.invoke(
        cli,
        [
            "base-dir",
            "record",
            "--base",
            str(other),
            "--source",
            "askuserquestion",
        ],
    )
    assert result.exit_code != 0
    combined = (result.output or "") + (result.stderr or "")
    assert "records base=" in combined


def test_resolve_rejects_unknown_anchor_version(tmp_path, runner, monkeypatch):
    """`cafleet base-dir resolve` exits non-zero with the standardized error on bad version."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "proj"
    proj.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setattr(base_dir_module, "_TMP_CANDIDATE", tmp_path / "tmp-absent")
    monkeypatch.chdir(proj)

    # version=2 → forward-incompatible, rejected
    _write_anchor(proj / ANCHOR_FILENAME, base=str(proj), version=2)
    result = runner.invoke(cli, ["base-dir", "resolve", "--json"])
    assert result.exit_code != 0
    combined = (result.output or "") + (result.stderr or "")
    assert "version" in combined
    assert "=2" in combined or " 2" in combined
    assert "supports version 1" in combined or "version 1" in combined

    # version=0 → also rejected (non-positive)
    _write_anchor(proj / ANCHOR_FILENAME, base=str(proj), version=0)
    result = runner.invoke(cli, ["base-dir", "resolve", "--json"])
    assert result.exit_code != 0


def test_cli_resolve_task_name_outside_git_repo_exits_1_no_json(
    tmp_path, runner, monkeypatch
):
    """Bullet 7 (no-repo-root failure): a positional ``task_name`` invoked when CWD has no
    ``.git`` ancestor exits 1 with the standardized stderr message and emits no JSON payload,
    even with ``--json``. The Python-level resolver raises ``RuntimeError`` for the same
    scenario."""
    home = tmp_path / "home"
    home.mkdir()
    nowhere = tmp_path / "nowhere"
    nowhere.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.chdir(nowhere)

    result = runner.invoke(
        cli, ["base-dir", "resolve", "--json", "design-docs/0000099-x"]
    )

    assert result.exit_code != 0
    combined = (result.output or "") + (result.stderr or "")
    assert "no .git ancestor" in combined
    # The error is plain text, not a JSON payload — parsing the combined output as JSON fails.
    with pytest.raises(json.JSONDecodeError):
        json.loads(combined)

    # The Python-level resolver raises RuntimeError from the same scenario.
    with pytest.raises(RuntimeError, match=r"no \.git ancestor"):
        resolve(task_name="design-docs/0000099-x", cwd=nowhere)
