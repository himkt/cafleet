"""Tests for the ``cafleet setup`` CLI command.

The skills half is exercised entirely offline: ``importlib.metadata.version``,
the ``GET /releases/tags/<version>`` lookup and the asset download are all
monkeypatched, and ``AGENT_SKILLS_DIRS`` is redirected to ``tmp_path`` homes.
Every ``cafleet setup`` run also drives the database half, so an autouse
fixture redirects the registry at a temp SQLite file.

Contract notes for the implementation under test (``cafleet.cli.setup``):

* The installed version is read via a module-qualified
  ``importlib.metadata.version("cafleet")`` call.
* Both the release lookup and the asset download go through
  ``urllib.request.urlopen``.
* ``shutil.copytree`` performs the per-skill install.
* ``AGENT_SKILLS_DIRS`` is a module-level dict of ``{agent: Path}``.
"""

import importlib.metadata
import io
import json
import sqlite3
import urllib.error
import urllib.request
import zipfile

import pytest
from click.testing import CliRunner

from cafleet import config

SKILL_DIR_NAMES = ("cafleet", "cafleet-design-doc", "cafleet-research")
CLI_VERSION = "0.12.2"
ASSET_NAME = f"cafleet-skills-v{CLI_VERSION}.zip"
DOWNLOAD_URL = f"https://example.invalid/download/{ASSET_NAME}"
_RELEASE_API_FRAGMENT = "/releases/tags/"


def _make_skills_zip(
    *, skill_dirs=SKILL_DIR_NAMES, extra_dirs=(), extra_files=(), raw_members=None
) -> bytes:
    """Build an in-memory skills archive.

    ``raw_members`` bypasses the canonical layout and writes the named members
    verbatim (used for zip-slip cases). Otherwise the archive unpacks to
    ``skills/<name>/...`` for each entry in ``skill_dirs`` plus any ``extra_*``.
    """
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        if raw_members is not None:
            for member in raw_members:
                zf.writestr(member, "x\n")
        else:
            for name in skill_dirs:
                zf.writestr(f"skills/{name}/SKILL.md", f"# {name}\n")
                zf.writestr(f"skills/{name}/reference/page.md", "ref\n")
            for name in extra_dirs:
                zf.writestr(f"skills/{name}/SKILL.md", "x\n")
            for name in extra_files:
                zf.writestr(f"skills/{name}", "x\n")
    return buf.getvalue()


def _mock_release(
    monkeypatch,
    *,
    version=CLI_VERSION,
    zip_bytes=None,
    assets=None,
    api_error=None,
    download_error=None,
):
    """Patch the version lookup and ``urlopen`` for the skills-half network calls."""
    real_version = importlib.metadata.version

    def fake_version(package):
        return version if package == "cafleet" else real_version(package)

    monkeypatch.setattr(importlib.metadata, "version", fake_version)

    if assets is None:
        assets = [{"name": ASSET_NAME, "browser_download_url": DOWNLOAD_URL}]
    release_body = json.dumps({"tag_name": version, "assets": assets}).encode()

    def fake_urlopen(req, timeout=None):
        url = req.full_url if hasattr(req, "full_url") else req
        if _RELEASE_API_FRAGMENT in url:
            if api_error is not None:
                raise api_error
            return io.BytesIO(release_body)
        if url == DOWNLOAD_URL:
            if download_error is not None:
                raise download_error
            return io.BytesIO(zip_bytes if zip_bytes is not None else b"")
        raise AssertionError(f"unexpected URL: {url}")

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)


@pytest.fixture(autouse=True)
def registry_db(tmp_path, monkeypatch):
    """Redirect the DB half at a temp SQLite so no test touches the real registry."""
    db_path = tmp_path / "registry" / "cafleet.db"
    monkeypatch.setattr(
        config.settings, "database_url", f"sqlite+aiosqlite:///{db_path}"
    )
    return db_path


@pytest.fixture
def homes(tmp_path, monkeypatch):
    """Point every agent's skills dir at an isolated ``tmp_path`` home.

    Returns the ``{agent: skills_dir}`` mapping; the home (the skills dir's
    parent) is created on demand by ``_make_home``.
    """
    from cafleet.cli import setup as setup_module

    mapping = {
        "claude": tmp_path / "h_claude" / ".claude" / "skills",
        "codex": tmp_path / "h_codex" / ".codex" / "skills",
        "opencode": tmp_path / "h_opencode" / ".config" / "opencode" / "skills",
    }
    monkeypatch.setattr(setup_module, "AGENT_SKILLS_DIRS", dict(mapping))
    return mapping


def _make_home(skills_dir):
    skills_dir.parent.mkdir(parents=True, exist_ok=True)


def _installed_skill_dirs(skills_dir):
    if not skills_dir.exists():
        return set()
    return {p.name for p in skills_dir.iterdir() if p.is_dir()}


def _table_names(db_path):
    conn = sqlite3.connect(str(db_path))
    try:
        rows = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
        return {row[0] for row in rows}
    finally:
        conn.close()


def _run_setup(args=()):
    from cafleet.cli import cli

    return CliRunner().invoke(cli, ["setup", *args])


# --------------------------------------------------------------------------- #
# Target resolution: auto-detect and --agent scoping                          #
# --------------------------------------------------------------------------- #


def test_autodetect_installs_only_present_homes(homes, registry_db, monkeypatch):
    """Auto-detect installs only where the agent home exists."""
    _make_home(homes["claude"])
    _make_home(homes["opencode"])
    # codex home intentionally absent
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup()

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["opencode"]) == set(SKILL_DIR_NAMES)
    assert not homes["codex"].exists()
    assert {"fleets", "agents", "tasks", "alembic_version"} <= _table_names(registry_db)


def test_agent_flag_scopes_targets_and_dedupes(homes, registry_db, monkeypatch):
    """--agent limits the skills targets; repeated values are deduped silently."""
    for skills_dir in homes.values():
        _make_home(skills_dir)
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup(["--agent", "claude", "--agent", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["codex"]) == set()
    assert _installed_skill_dirs(homes["opencode"]) == set()
    # silent dedupe: claude is installed/reported exactly once
    assert result.output.count("claude:") == 1


def test_agent_flag_db_half_still_runs(homes, registry_db, monkeypatch):
    """The database half runs even when --agent scopes the skills half."""
    _make_home(homes["codex"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup(["--agent", "codex"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["codex"]) == set(SKILL_DIR_NAMES)
    assert {"alembic_version", "fleets"} <= _table_names(registry_db)


def test_agent_flag_creates_missing_home(homes, registry_db, monkeypatch):
    """An explicitly named agent whose home is absent gets its tree created."""
    # no homes pre-created
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)


def test_zero_homes_detected(homes, registry_db, monkeypatch):
    """Auto-detect with no homes fails the skills half listing the searched paths."""
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup()

    assert result.exit_code == 1, result.output
    out = result.output.lower()
    assert "no coding-agent homes detected" in out
    assert ".claude" in result.output
    assert ".codex" in result.output
    assert "opencode" in result.output
    # the database half still ran despite the skills-half failure
    assert "alembic_version" in _table_names(registry_db)


# --------------------------------------------------------------------------- #
# Replace semantics / idempotency                                             #
# --------------------------------------------------------------------------- #


def test_replace_idempotency_second_run_clean_tree(homes, registry_db, monkeypatch):
    """Each run replaces the skill dirs; re-running yields the same clean tree."""
    _make_home(homes["claude"])
    stale = homes["claude"] / "cafleet" / "STALE.md"
    stale.parent.mkdir(parents=True)
    stale.write_text("old")
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    first = _run_setup(["--agent", "claude"])
    assert first.exit_code == 0, first.output
    assert not stale.exists()  # replaced, not merged
    tree_after_first = sorted(
        p.relative_to(homes["claude"]).as_posix() for p in homes["claude"].rglob("*")
    )

    second = _run_setup(["--agent", "claude"])
    assert second.exit_code == 0, second.output
    tree_after_second = sorted(
        p.relative_to(homes["claude"]).as_posix() for p in homes["claude"].rglob("*")
    )
    assert tree_after_second == tree_after_first


# --------------------------------------------------------------------------- #
# Archive integrity                                                           #
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("evil_member", ["../evil.txt", "/abs/evil.txt"])
def test_zip_slip_member_rejected_nothing_extracted(
    homes, registry_db, monkeypatch, evil_member
):
    """A ``..``/absolute member is rejected before extraction; targets untouched."""
    _make_home(homes["claude"])
    sentinel = homes["claude"] / "cafleet" / "SENTINEL.md"
    sentinel.parent.mkdir(parents=True)
    sentinel.write_text("keep")
    members = [f"skills/{name}/SKILL.md" for name in SKILL_DIR_NAMES] + [evil_member]
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip(raw_members=members))

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert sentinel.exists()  # nothing extracted, nothing removed
    assert sentinel.read_text() == "keep"


@pytest.mark.parametrize(
    "zip_bytes",
    [
        _make_skills_zip(extra_dirs=("rogue",)),
        _make_skills_zip(extra_files=("README.md",)),
    ],
    ids=["extra-dir", "stray-file"],
)
def test_extra_entry_under_skills_is_malformed(
    homes, registry_db, monkeypatch, zip_bytes
):
    """Any entry under ``skills/`` beyond the three skill dirs is malformed."""
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=zip_bytes)

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_missing_skill_dir_is_malformed(homes, registry_db, monkeypatch):
    """A ``skills/`` holding only two of the three skill dirs is malformed."""
    _make_home(homes["claude"])
    _mock_release(
        monkeypatch,
        zip_bytes=_make_skills_zip(skill_dirs=("cafleet", "cafleet-design-doc")),
    )

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_bad_zip_is_malformed(homes, registry_db, monkeypatch):
    """A non-zip / truncated download surfaces as the malformed-asset error."""
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=b"this is not a zip archive")

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


# --------------------------------------------------------------------------- #
# Release / network resolution                                               #
# --------------------------------------------------------------------------- #


def test_missing_asset_message(homes, registry_db, monkeypatch):
    """An asset absent from the release surfaces the specific not-found message."""
    _make_home(homes["claude"])
    _mock_release(
        monkeypatch,
        assets=[{"name": "something-else.zip", "browser_download_url": DOWNLOAD_URL}],
    )

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert ASSET_NAME in result.output
    assert "not found" in result.output.lower()


def test_no_release_for_version(homes, registry_db, monkeypatch):
    """A 404 from ``/releases/tags/<version>`` is the no-release-for-version error."""
    _make_home(homes["claude"])
    err = urllib.error.HTTPError(
        url="https://api.github.com/repos/himkt/cafleet/releases/tags/0.12.2",
        code=404,
        msg="Not Found",
        hdrs=None,
        fp=None,
    )
    _mock_release(monkeypatch, api_error=err)

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert CLI_VERSION in result.output
    assert "no release found" in result.output.lower()


@pytest.mark.parametrize(
    "api_error",
    [
        urllib.error.URLError("dns failure"),
        urllib.error.HTTPError(
            url="https://api", code=403, msg="rate limited", hdrs=None, fp=None
        ),
        urllib.error.HTTPError(
            url="https://api", code=500, msg="server error", hdrs=None, fp=None
        ),
        TimeoutError("timed out"),
    ],
    ids=["urlerror", "http-403", "http-500", "timeout"],
)
def test_network_error_folded(homes, registry_db, monkeypatch, api_error):
    """URLError, timeout, 403 rate-limit and non-404 5xx fold into one message."""
    _make_home(homes["claude"])
    _mock_release(monkeypatch, api_error=api_error)

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "could not reach the github api" in result.output.lower()


@pytest.mark.parametrize(
    "download_error",
    [
        urllib.error.URLError("connection reset"),
        urllib.error.HTTPError(
            url=DOWNLOAD_URL, code=403, msg="expired", hdrs=None, fp=None
        ),
        urllib.error.HTTPError(
            url=DOWNLOAD_URL, code=500, msg="server error", hdrs=None, fp=None
        ),
    ],
    ids=["urlerror", "http-403", "http-500"],
)
def test_download_network_error_folded(homes, registry_db, monkeypatch, download_error):
    """A network failure on the asset download folds into the same message.

    The release lookup succeeds; the failure happens while streaming the asset,
    so this exercises the second ``urlopen`` call (step 4), distinct from the
    release-lookup path covered by ``test_network_error_folded``.
    """
    _make_home(homes["claude"])
    _mock_release(
        monkeypatch, zip_bytes=_make_skills_zip(), download_error=download_error
    )

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "could not reach the github api" in result.output.lower()


# --------------------------------------------------------------------------- #
# Install-time filesystem errors                                             #
# --------------------------------------------------------------------------- #


def test_unwritable_target_permission_error(homes, registry_db, monkeypatch):
    """A ``PermissionError`` during install surfaces as a ClickException."""
    from cafleet.cli import setup as setup_module

    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    def deny_copytree(src, dst, *args, **kwargs):
        raise PermissionError(13, "Permission denied", str(dst))

    monkeypatch.setattr(setup_module.shutil, "copytree", deny_copytree)

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    out = result.output.lower()
    assert "permission" in out or "denied" in out


# --------------------------------------------------------------------------- #
# Independence of the two halves (Step 4 task 2)                              #
# --------------------------------------------------------------------------- #


def test_independence_skills_fail_db_succeeds(homes, registry_db, monkeypatch):
    """Skills half fails (malformed asset); the DB half still runs to head."""
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=b"not a zip")

    result = _run_setup()

    assert result.exit_code == 1, result.output
    out = result.output.lower()
    # skills-half failure reported
    assert "malformed" in out
    assert "skills" in out
    # db-half success: schema created at head and its status line printed
    assert {"alembic_version", "fleets"} <= _table_names(registry_db)
    assert "applied" in out or "head" in out


def test_independence_db_fail_skills_succeed(homes, registry_db, monkeypatch):
    """DB half fails (orphan-tables DB); the skills half still installs."""
    _make_home(homes["claude"])
    registry_db.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(registry_db))
    try:
        conn.execute("CREATE TABLE legacy_squat (id INTEGER PRIMARY KEY)")
        conn.commit()
    finally:
        conn.close()
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    out = result.output
    # skills-half success: installed and its report line printed
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert "installed" in out.lower()
    # db-half failure reported
    assert "alembic stamp head" in out
    assert "db" in out.lower()
