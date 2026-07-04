"""Tests for the ``cafleet setup`` CLI group (bare, ``db``, ``skill``).

The skills half is exercised entirely offline: ``importlib.metadata.version``,
the ``GET /releases/tags/<version>`` lookup and the asset download are all
monkeypatched, and ``AGENT_SKILLS_DIRS`` is redirected to ``tmp_path`` homes.
An autouse fixture redirects the registry at a temp SQLite file.

Contract notes for the implementation under test (``cafleet.cli.setup``):

* The installed version is read via a module-qualified
  ``importlib.metadata.version("cafleet")`` call.
* Both the release lookup and the asset download go through
  ``urllib.request.urlopen``.
* ``shutil.copytree`` performs the per-skill install.
* ``AGENT_SKILLS_DIRS`` is a module-level dict of ``{agent: Path}``.
* The db half calls ``run_db_init`` through the module attribute
  ``cafleet.cli.setup.run_db_init``.
"""

import importlib.metadata
import io
import json
import shutil
import sqlite3
import urllib.error
import urllib.request
import zipfile

import click
import pytest
from click.testing import CliRunner

from cafleet import config

SKILL_DIR_NAMES = ("cafleet", "cafleet-design-doc", "cafleet-research")
CLI_VERSION = "0.12.2"
ASSET_NAME = f"cafleet-skills-v{CLI_VERSION}.zip"
DOWNLOAD_URL = f"https://example.invalid/download/{ASSET_NAME}"
_RELEASE_API_FRAGMENT = "/releases/tags/"

PREFLIGHT_ERROR = (
    "the database schema is missing or outdated; "
    "run 'cafleet setup' or 'cafleet setup db' first"
)


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
    release_body=None,
):
    """Patch the version lookup and ``urlopen`` for the skills-half network calls.

    ``release_body`` overrides the API response payload with raw bytes (used to
    inject unparseable / non-assets JSON); otherwise it is built from ``assets``.
    """
    real_version = importlib.metadata.version

    def fake_version(package):
        return version if package == "cafleet" else real_version(package)

    monkeypatch.setattr(importlib.metadata, "version", fake_version)

    if release_body is None:
        if assets is None:
            assets = [{"name": ASSET_NAME, "browser_download_url": DOWNLOAD_URL}]
        release_body = json.dumps({"tag_name": version, "assets": assets}).encode()

    def fake_urlopen(req, timeout=None):
        url = req.full_url if hasattr(req, "full_url") else req
        if _RELEASE_API_FRAGMENT in url:
            if api_error is not None:
                raise api_error
            return io.BytesIO(release_body)
        if url.endswith(".zip"):
            if download_error is not None:
                raise download_error
            return io.BytesIO(zip_bytes if zip_bytes is not None else b"")
        raise AssertionError(f"unexpected URL: {url}")

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)


def _forbid_network(monkeypatch):
    """Fail loudly if the command under test touches the network at all."""

    def _no_network(*args, **kwargs):
        raise AssertionError("unexpected network access")

    monkeypatch.setattr(urllib.request, "urlopen", _no_network)


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


def _skill_install_rows(db_path):
    """Return ``[(coding_agent, cafleet_version), ...]`` ordered by agent."""
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT coding_agent, cafleet_version FROM skill_installs"
            " ORDER BY coding_agent"
        ).fetchall()
    finally:
        conn.close()


def _init_schema():
    """Migrate the registry to head the way ``cafleet setup db`` does."""
    from cafleet.db.init import run_db_init

    run_db_init()


def _run(args):
    from cafleet.cli import cli

    return CliRunner().invoke(cli, args)


def _run_setup(args=()):
    return _run(["setup", *args])


def _run_setup_db():
    return _run(["setup", "db"])


def _run_setup_skill(args=()):
    return _run(["setup", "skill", *args])


# --------------------------------------------------------------------------- #
# Bare ``cafleet setup``: db half first, then skills half                      #
# --------------------------------------------------------------------------- #


def test_bare_setup_runs_both_halves_and_records_rows(homes, registry_db, monkeypatch):
    """Bare setup creates the schema, installs every detected home, records rows."""
    _make_home(homes["claude"])
    _make_home(homes["opencode"])
    # codex home intentionally absent
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup()

    assert result.exit_code == 0, result.output
    assert {"fleets", "agents", "skill_installs"} <= _table_names(registry_db)
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["opencode"]) == set(SKILL_DIR_NAMES)
    assert not homes["codex"].exists()
    assert _skill_install_rows(registry_db) == [
        ("claude", CLI_VERSION),
        ("opencode", CLI_VERSION),
    ]


def test_bare_setup_rejects_agent_option(homes, registry_db, monkeypatch):
    """``--agent`` moved to ``setup skill``; bare setup rejects it with exit 2."""
    _make_home(homes["claude"])
    _forbid_network(monkeypatch)

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 2, result.output
    assert "no such option" in result.output.lower()


def test_bare_setup_db_failure_cascades_to_skills_preflight(
    homes, registry_db, monkeypatch
):
    """A failed db half makes the skills half fail its schema pre-flight.

    Both halves are reported failed, db first (matching the run order), and
    the command exits 1 with the joined summary.
    """
    from cafleet.cli import setup as setup_module

    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    def broken_run_db_init():
        raise click.ClickException("disk full")

    monkeypatch.setattr(setup_module, "run_db_init", broken_run_db_init)

    result = _run_setup()

    assert result.exit_code == 1, result.output
    assert "db half failed: disk full" in result.output
    assert "skills half failed:" in result.output
    assert PREFLIGHT_ERROR in result.output
    assert "db and skills half failed" in result.output
    assert result.output.index("db half failed") < result.output.index(
        "skills half failed"
    )


def test_bare_setup_skills_fail_db_succeeds(homes, registry_db, monkeypatch):
    """Skills half fails (malformed asset); the db half already succeeded."""
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=b"not a zip")

    result = _run_setup()

    assert result.exit_code == 1, result.output
    assert "skills half failed:" in result.output
    assert "malformed" in result.output.lower()
    assert "db half failed" not in result.output
    assert "Error: skills half failed" in result.output
    assert {"fleets", "skill_installs"} <= _table_names(registry_db)
    assert _skill_install_rows(registry_db) == []


def test_bare_setup_zero_homes_detected(homes, registry_db, monkeypatch):
    """Auto-detect with no homes fails the skills half listing the searched paths."""
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup()

    assert result.exit_code == 1, result.output
    out = result.output.lower()
    assert "no coding-agent homes detected" in out
    assert ".claude" in result.output
    assert ".codex" in result.output
    assert "opencode" in result.output
    # the db half ran (and succeeded) despite the skills-half failure
    assert "skill_installs" in _table_names(registry_db)
    assert "Error: skills half failed" in result.output


# --------------------------------------------------------------------------- #
# ``cafleet setup db``: schema only                                            #
# --------------------------------------------------------------------------- #


def test_setup_db_creates_schema_only(homes, registry_db, monkeypatch):
    """``setup db`` migrates the schema, writes no rows, and never goes online."""
    _make_home(homes["claude"])
    _forbid_network(monkeypatch)

    result = _run_setup_db()

    assert result.exit_code == 0, result.output
    assert f"Created {registry_db} and applied migrations to head" in result.output
    assert {"fleets", "skill_installs", "alembic_version"} <= _table_names(registry_db)
    assert _skill_install_rows(registry_db) == []
    assert _installed_skill_dirs(homes["claude"]) == set()


def test_setup_db_idempotent_and_preserves_rows(homes, registry_db, monkeypatch):
    """A second ``setup db`` run succeeds and never touches recorded rows."""
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    assert _run_setup_db().exit_code == 0
    assert _run_setup_skill(["--agent", "claude"]).exit_code == 0
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]

    result = _run_setup_db()

    assert result.exit_code == 0, result.output
    assert "already at head" in result.output.lower()
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]


# --------------------------------------------------------------------------- #
# ``cafleet setup skill``: pre-flight, recording, upsert                       #
# --------------------------------------------------------------------------- #


def test_setup_skill_preflight_error_when_schema_missing(
    homes, registry_db, monkeypatch
):
    """Without the ``skill_installs`` table the pre-flight fails before any
    network access or install."""
    _make_home(homes["claude"])
    _forbid_network(monkeypatch)

    result = _run_setup_skill()

    assert result.exit_code == 1, result.output
    assert PREFLIGHT_ERROR in result.output
    assert _installed_skill_dirs(homes["claude"]) == set()


def test_setup_skill_agent_records_one_row(homes, registry_db, monkeypatch):
    """``setup skill --agent claude`` installs one home and records one row."""
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]
    # the per-home success line is unchanged from the pre-group surface
    expected_line = (
        f"claude: installed {', '.join(SKILL_DIR_NAMES)} "
        f"(v{CLI_VERSION}) -> {homes['claude']}"
    )
    assert expected_line in result.output


def test_setup_skill_reinstall_upserts_row(homes, registry_db, monkeypatch):
    """Re-installing replaces the home's row instead of adding a second one."""
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    assert _run_setup_skill(["--agent", "claude"]).exit_code == 0
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]

    _mock_release(
        monkeypatch,
        version="0.12.3",
        zip_bytes=_make_skills_zip(),
        assets=[
            {
                "name": "cafleet-skills-v0.12.3.zip",
                "browser_download_url": "https://example.invalid/download/cafleet-skills-v0.12.3.zip",
            }
        ],
    )

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 0, result.output
    assert _skill_install_rows(registry_db) == [("claude", "0.12.3")]


def test_setup_skill_autodetect_installs_only_present_homes(
    homes, registry_db, monkeypatch
):
    """Without ``--agent`` the targets are the detected homes, one row each."""
    _init_schema()
    _make_home(homes["claude"])
    _make_home(homes["opencode"])
    # codex home intentionally absent
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup_skill()

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["opencode"]) == set(SKILL_DIR_NAMES)
    assert not homes["codex"].exists()
    assert _skill_install_rows(registry_db) == [
        ("claude", CLI_VERSION),
        ("opencode", CLI_VERSION),
    ]


def test_setup_skill_agent_flag_scopes_and_dedupes(homes, registry_db, monkeypatch):
    """``--agent`` limits the targets; repeated values are deduped silently."""
    _init_schema()
    for skills_dir in homes.values():
        _make_home(skills_dir)
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup_skill(["--agent", "claude", "--agent", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["codex"]) == set()
    assert _installed_skill_dirs(homes["opencode"]) == set()
    assert result.output.count("claude:") == 1
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]


def test_setup_skill_agent_flag_creates_missing_home(homes, registry_db, monkeypatch):
    """An explicitly named agent whose home is absent gets its tree created."""
    _init_schema()
    # no homes pre-created
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set(SKILL_DIR_NAMES)
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]


def test_setup_skill_install_failure_keeps_prior_rows(homes, registry_db, monkeypatch):
    """An install failure aborts the loop; rows for completed homes remain."""
    from cafleet.cli import setup as setup_module

    _init_schema()
    _make_home(homes["claude"])
    _make_home(homes["codex"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    real_copytree = shutil.copytree

    def failing_copytree(src, dst, *args, **kwargs):
        if "h_codex" in str(dst):
            raise OSError("disk full")
        return real_copytree(src, dst, *args, **kwargs)

    monkeypatch.setattr(setup_module.shutil, "copytree", failing_copytree)

    result = _run_setup_skill(["--agent", "claude", "--agent", "codex"])

    assert result.exit_code == 1, result.output
    assert "failed to install skills into" in result.output
    assert _skill_install_rows(registry_db) == [("claude", CLI_VERSION)]


# --------------------------------------------------------------------------- #
# Archive integrity (via ``setup skill``)                                      #
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("evil_member", ["../evil.txt", "/abs/evil.txt"])
def test_zip_slip_member_rejected_nothing_extracted(
    homes, registry_db, monkeypatch, evil_member
):
    """A ``..``/absolute member is rejected before extraction; targets untouched."""
    _init_schema()
    _make_home(homes["claude"])
    sentinel = homes["claude"] / "cafleet" / "SENTINEL.md"
    sentinel.parent.mkdir(parents=True)
    sentinel.write_text("keep")
    members = [f"skills/{name}/SKILL.md" for name in SKILL_DIR_NAMES] + [evil_member]
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip(raw_members=members))

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert sentinel.exists()  # nothing extracted, nothing removed
    assert sentinel.read_text() == "keep"
    assert _skill_install_rows(registry_db) == []


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
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=zip_bytes)

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_missing_skill_dir_is_malformed(homes, registry_db, monkeypatch):
    """A ``skills/`` holding only two of the three skill dirs is malformed."""
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(
        monkeypatch,
        zip_bytes=_make_skills_zip(skill_dirs=("cafleet", "cafleet-design-doc")),
    )

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_bad_zip_is_malformed(homes, registry_db, monkeypatch):
    """A non-zip / truncated download surfaces as the malformed-asset error."""
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=b"this is not a zip archive")

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_extractall_oserror_is_malformed(homes, registry_db, monkeypatch):
    """An ``OSError`` raised mid-extraction is reported as the malformed-asset error.

    The archive passes the zip-slip and bad-zip pre-checks; the failure happens
    inside ``ZipFile.extractall`` (e.g. a filesystem error during extraction).
    """
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    def boom(self, *args, **kwargs):
        raise OSError("extraction failed")

    monkeypatch.setattr(zipfile.ZipFile, "extractall", boom)

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


# --------------------------------------------------------------------------- #
# Release / network resolution (via ``setup skill``)                           #
# --------------------------------------------------------------------------- #


def test_missing_asset_message(homes, registry_db, monkeypatch):
    """An asset absent from the release surfaces the specific not-found message."""
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(
        monkeypatch,
        assets=[{"name": "something-else.txt", "browser_download_url": DOWNLOAD_URL}],
    )

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert ASSET_NAME in result.output
    assert "not found" in result.output.lower()


@pytest.mark.parametrize(
    "release_body",
    [b"this is not json", b'{"tag_name": "0.12.2"}'],
    ids=["non-json", "no-assets-key"],
)
def test_unparseable_api_response(homes, registry_db, monkeypatch, release_body):
    """A 200 body that is not JSON, or lacks the ``assets`` array, is rejected."""
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, release_body=release_body)

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "could not parse the github api response" in result.output.lower()


def test_no_release_for_version(homes, registry_db, monkeypatch):
    """A 404 from ``/releases/tags/<version>`` is the no-release-for-version error."""
    _init_schema()
    _make_home(homes["claude"])
    err = urllib.error.HTTPError(
        url="https://api.github.com/repos/himkt/cafleet/releases/tags/0.12.2",
        code=404,
        msg="Not Found",
        hdrs=None,
        fp=None,
    )
    _mock_release(monkeypatch, api_error=err)

    result = _run_setup_skill(["--agent", "claude"])

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
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, api_error=api_error)

    result = _run_setup_skill(["--agent", "claude"])

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
    so this exercises the second ``urlopen`` call, distinct from the
    release-lookup path covered by ``test_network_error_folded``.
    """
    _init_schema()
    _make_home(homes["claude"])
    _mock_release(
        monkeypatch, zip_bytes=_make_skills_zip(), download_error=download_error
    )

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    assert "could not reach the github api" in result.output.lower()


# --------------------------------------------------------------------------- #
# Install-time filesystem errors (via ``setup skill``)                         #
# --------------------------------------------------------------------------- #


def test_unwritable_target_permission_error(homes, registry_db, monkeypatch):
    """A ``PermissionError`` during install surfaces as a ClickException."""
    from cafleet.cli import setup as setup_module

    _init_schema()
    _make_home(homes["claude"])
    _mock_release(monkeypatch, zip_bytes=_make_skills_zip())

    def deny_copytree(src, dst, *args, **kwargs):
        raise PermissionError(13, "Permission denied", str(dst))

    monkeypatch.setattr(setup_module.shutil, "copytree", deny_copytree)

    result = _run_setup_skill(["--agent", "claude"])

    assert result.exit_code == 1, result.output
    out = result.output.lower()
    assert "permission" in out or "denied" in out
    assert _skill_install_rows(registry_db) == []
