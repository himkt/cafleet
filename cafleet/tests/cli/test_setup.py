"""Tests for the ``cafleet setup`` CLI command (single command, ``--skip``).

The assets half is exercised entirely offline: ``importlib.metadata.version``,
the ``GET /releases/tags/<version>`` lookup and the asset download are all
monkeypatched, and ``AGENT_SKILLS_DIRS`` / ``AGENT_PRESET_TARGETS`` are
redirected to ``tmp_path`` homes. An autouse fixture redirects the registry at
a temp SQLite file.

Contract notes for the implementation under test (``cafleet.cli.setup``):

* The installed version is read via a module-qualified
  ``importlib.metadata.version("cafleet")`` call.
* Both the release lookup and the asset download go through
  ``urllib.request.urlopen``.
* ``shutil.copytree`` performs the per-skill install.
* ``AGENT_SKILLS_DIRS`` is a module-level dict of ``{agent: Path}``.
* ``AGENT_PRESET_TARGETS`` is a module-level dict of ``{agent: Path}`` holding
  the preset install target for exactly ``codex`` and ``opencode`` (claude has
  no preset); each target is ``expanduser()``-ed at install time.
* The preset archive sources are the fixed members
  ``presets/codex/cafleet.rules`` and ``presets/opencode/cafleet.md``.
* The db half calls ``run_db_init`` through the module attribute
  ``cafleet.cli.setup.run_db_init``.
"""

import hashlib
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

AGENTS = ("claude", "codex", "opencode")
SKILL_DIR_NAMES = (
    "cafleet",
    "cafleet-design-doc",
    "cafleet-research",
    "cafleet-model-catalog-refresh",
)
CLI_VERSION = "0.12.2"
ASSET_NAME = f"cafleet-assets-v{CLI_VERSION}.zip"
DOWNLOAD_URL = f"https://example.invalid/download/{ASSET_NAME}"
_RELEASE_API_FRAGMENT = "/releases/tags/"

PREFLIGHT_ERROR = (
    "the database schema is missing or outdated; run 'cafleet setup' first"
)

SCHEMA_ONLY_ARGS = (
    "--skip",
    "claude",
    "--skip",
    "codex",
    "--skip",
    "opencode",
)
ASSETS_SKIPPED_LINE = "assets half skipped (all agents skipped)"

OPENCODE_PRESET_CONTENT = "# cafleet opencode agent preset\n"
CODEX_RULES_CONTENT = 'prefix_rule(pattern = ["cafleet"], decision = "allow")\n'

PRESET_ARCHIVE_MEMBERS = {
    "opencode": ("presets/opencode/cafleet.md", OPENCODE_PRESET_CONTENT),
    "codex": ("presets/codex/cafleet.rules", CODEX_RULES_CONTENT),
}

CATALOG_MEMBER = "skills/cafleet/reference/model-catalog.md"
MANIFEST_MEMBER = "skills/cafleet/asset-manifest.json"
CATALOG_CONTENT = "# CAFleet Model Catalog\n"
MANIFEST_CONTENT = (
    json.dumps(
        {
            "cafleet_version": CLI_VERSION,
            "catalog_sha256": hashlib.sha256(
                CATALOG_CONTENT.encode("utf-8")
            ).hexdigest(),
        },
        sort_keys=True,
        indent=2,
    )
    + "\n"
)


def _make_assets_zip(
    *,
    skill_dirs=SKILL_DIR_NAMES,
    extra_dirs=(),
    extra_files=(),
    raw_members=None,
    preset_agents=("opencode", "codex"),
    cafleet_manifest=True,
    cafleet_catalog=True,
) -> bytes:
    """Build an in-memory release archive (``skills/`` + ``presets/``).

    ``raw_members`` bypasses the canonical layout and writes the named members
    verbatim (used for zip-slip cases). Otherwise the archive unpacks to
    ``skills/<name>/...`` for each entry in ``skill_dirs`` plus any ``extra_*``,
    the cafleet catalog + asset manifest (droppable via ``cafleet_catalog`` /
    ``cafleet_manifest``), and one preset file per agent in ``preset_agents``
    (drop an agent to build an archive missing that preset).
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
            if "cafleet" in skill_dirs:
                if cafleet_catalog:
                    zf.writestr(CATALOG_MEMBER, CATALOG_CONTENT)
                if cafleet_manifest:
                    zf.writestr(MANIFEST_MEMBER, MANIFEST_CONTENT)
            for name in extra_dirs:
                zf.writestr(f"skills/{name}/SKILL.md", "x\n")
            for name in extra_files:
                zf.writestr(f"skills/{name}", "x\n")
            for agent in preset_agents:
                path, content = PRESET_ARCHIVE_MEMBERS[agent]
                zf.writestr(path, content)
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
    """Patch the version lookup and ``urlopen`` for the assets-half network calls.

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


def _preset_target_mapping(tmp_path):
    return {
        "codex": tmp_path / "h_codex" / ".codex" / "rules" / "cafleet.rules",
        "opencode": tmp_path / "h_opencode" / ".opencode" / "agents" / "cafleet.md",
    }


@pytest.fixture
def homes(tmp_path, monkeypatch):
    """Point every agent's skills dir and preset target at ``tmp_path`` homes.

    Returns the ``{agent: skills_dir}`` mapping. Nothing is pre-created: the
    install path creates each target tree as needed. The preset targets are
    redirected alongside so no install ever touches the real ``$HOME``.
    """
    from cafleet.cli import setup as setup_module

    mapping = {
        "claude": tmp_path / "h_claude" / ".claude" / "skills",
        "codex": tmp_path / "h_codex" / ".codex" / "skills",
        "opencode": tmp_path / "h_opencode" / ".config" / "opencode" / "skills",
    }
    monkeypatch.setattr(setup_module, "AGENT_SKILLS_DIRS", dict(mapping))
    monkeypatch.setattr(
        setup_module, "AGENT_PRESET_TARGETS", _preset_target_mapping(tmp_path)
    )
    return mapping


@pytest.fixture
def preset_targets(tmp_path, homes):
    """The redirected ``{agent: preset target}`` mapping (codex and opencode)."""
    return _preset_target_mapping(tmp_path)


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


def _asset_install_rows(db_path):
    """Return ``[(coding_agent, cafleet_version), ...]`` ordered by agent."""
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT coding_agent, cafleet_version FROM asset_installs"
            " ORDER BY coding_agent"
        ).fetchall()
    finally:
        conn.close()


def _run(args):
    from cafleet.cli import cli

    return CliRunner().invoke(cli, args)


def _run_setup(args=()):
    return _run(["setup", *args])


def _only(agent):
    """``--skip`` args narrowing the assets half to exactly ``agent``."""
    return [arg for other in AGENTS if other != agent for arg in ("--skip", other)]


def _skills_line(agent, skills_dir, version=CLI_VERSION):
    return (
        f"{agent}: installed {', '.join(SKILL_DIR_NAMES)} (v{version}) -> {skills_dir}"
    )


def _preset_line(agent, target, version=CLI_VERSION):
    return f"{agent}: installed preset (v{version}) -> {target}"


# --------------------------------------------------------------------------- #
# Bare ``cafleet setup``: db half first, then assets half for all three agents #
# --------------------------------------------------------------------------- #


def test_bare_setup_installs_all_three_agents(
    homes, preset_targets, registry_db, monkeypatch
):
    """Bare setup migrates the schema, then installs skills + presets for
    claude, codex, and opencode (in that order), creating every home from
    scratch, and records one row per agent."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup()

    assert result.exit_code == 0, result.output
    assert {"fleets", "members", "asset_installs"} <= _table_names(registry_db)
    for agent in AGENTS:
        assert _installed_skill_dirs(homes[agent]) == set(SKILL_DIR_NAMES)
    assert preset_targets["codex"].read_text(encoding="utf-8") == CODEX_RULES_CONTENT
    assert (
        preset_targets["opencode"].read_text(encoding="utf-8")
        == OPENCODE_PRESET_CONTENT
    )
    # claude has no preset at all
    assert "claude: installed preset" not in result.output
    assert result.output.count("installed preset") == 2
    assert _asset_install_rows(registry_db) == [
        ("claude", CLI_VERSION),
        ("codex", CLI_VERSION),
        ("opencode", CLI_VERSION),
    ]
    # fixed per-target order: claude, then codex, then opencode
    positions = [result.output.index(_skills_line(a, homes[a])) for a in AGENTS]
    assert positions == sorted(positions)


def test_bare_setup_per_agent_skills_line_precedes_preset_line(
    homes, preset_targets, registry_db, monkeypatch
):
    """Per agent the skills line precedes the preset line."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup()

    assert result.exit_code == 0, result.output
    for agent in ("codex", "opencode"):
        skills_line = _skills_line(agent, homes[agent])
        preset_line = _preset_line(agent, preset_targets[agent])
        assert skills_line in result.output
        assert preset_line in result.output
        assert result.output.index(skills_line) < result.output.index(preset_line)


def test_setup_help_contract(homes, registry_db, monkeypatch):
    """The command help and the ``--skip`` help match the documented strings."""
    _forbid_network(monkeypatch)

    result = _run_setup(["--help"])

    assert result.exit_code == 0, result.output
    assert (
        "Migrate the database schema and install the coding-agent assets"
        " (skills and presets)." in " ".join(result.output.split())
    )
    assert "Skip the named agent's assets install (repeatable)." in " ".join(
        result.output.split()
    )


def test_bare_setup_rejects_agent_option(homes, registry_db, monkeypatch):
    """``setup`` takes no ``--agent``; the option rejects with exit 2."""
    _forbid_network(monkeypatch)

    result = _run_setup(["--agent", "claude"])

    assert result.exit_code == 2, result.output
    assert "no such option" in result.output.lower()


def test_bare_setup_db_failure_cascades_to_assets_preflight(
    homes, registry_db, monkeypatch
):
    """A failed db half makes the assets half fail its schema pre-flight.

    Both halves are reported failed, db first (matching the run order), and
    the command exits 1 with the joined summary.
    """
    from cafleet.cli import setup as setup_module

    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    def broken_run_db_init():
        raise click.ClickException("disk full")

    monkeypatch.setattr(setup_module, "run_db_init", broken_run_db_init)

    result = _run_setup()

    assert result.exit_code == 1, result.output
    assert "db half failed: disk full" in result.output
    assert "assets half failed:" in result.output
    assert PREFLIGHT_ERROR in result.output
    assert "db and assets half failed" in result.output
    assert result.output.index("db half failed") < result.output.index(
        "assets half failed"
    )


def test_bare_setup_assets_fail_db_succeeds(homes, registry_db, monkeypatch):
    """Assets half fails (malformed asset); the db half already succeeded."""
    _mock_release(monkeypatch, zip_bytes=b"not a zip")

    result = _run_setup()

    assert result.exit_code == 1, result.output
    assert "assets half failed:" in result.output
    assert "malformed" in result.output.lower()
    assert "db half failed" not in result.output
    assert "Error: assets half failed" in result.output
    assert {"fleets", "asset_installs"} <= _table_names(registry_db)
    assert _asset_install_rows(registry_db) == []


def test_bare_setup_preset_failure_aborts_remaining_targets(
    homes, preset_targets, registry_db, monkeypatch
):
    """A preset failure aborts the assets loop exactly like a skills failure.

    claude (first target) completes and keeps its row; codex fails on its
    preset after its skills installed; opencode is never processed.
    """
    # ``~/.codex/rules`` exists as a regular file: the parent-dir creation
    # for the preset target fails with an OSError.
    rules_parent = preset_targets["codex"].parent
    rules_parent.parent.mkdir(parents=True, exist_ok=True)
    rules_parent.write_text("not a directory\n", encoding="utf-8")
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup()

    assert result.exit_code == 1, result.output
    assert f"failed to install preset into {preset_targets['codex']}" in result.output
    assert "assets half failed:" in result.output
    # skills for codex installed before the preset step aborted the loop
    assert _installed_skill_dirs(homes["codex"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["opencode"]) == set()
    assert _asset_install_rows(registry_db) == [("claude", CLI_VERSION)]


# --------------------------------------------------------------------------- #
# ``--skip``: single, repeated, duplicated, all three, invalid                 #
# --------------------------------------------------------------------------- #


def test_skip_single_agent(homes, preset_targets, registry_db, monkeypatch):
    """``--skip claude`` installs codex and opencode only."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(["--skip", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set()
    assert _installed_skill_dirs(homes["codex"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["opencode"]) == set(SKILL_DIR_NAMES)
    assert "claude:" not in result.output
    assert _asset_install_rows(registry_db) == [
        ("codex", CLI_VERSION),
        ("opencode", CLI_VERSION),
    ]


def test_skip_repeated_agents(homes, preset_targets, registry_db, monkeypatch):
    """Repeated ``--skip`` narrows the targets to the one remaining agent."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(["--skip", "claude", "--skip", "codex"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set()
    assert _installed_skill_dirs(homes["codex"]) == set()
    assert _installed_skill_dirs(homes["opencode"]) == set(SKILL_DIR_NAMES)
    assert not preset_targets["codex"].exists()
    assert (
        preset_targets["opencode"].read_text(encoding="utf-8")
        == OPENCODE_PRESET_CONTENT
    )
    assert _asset_install_rows(registry_db) == [("opencode", CLI_VERSION)]


def test_skip_duplicates_are_deduplicated(
    homes, preset_targets, registry_db, monkeypatch
):
    """``--skip claude --skip claude`` equals a single ``--skip claude``."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(["--skip", "claude", "--skip", "claude"])

    assert result.exit_code == 0, result.output
    assert _installed_skill_dirs(homes["claude"]) == set()
    assert _installed_skill_dirs(homes["codex"]) == set(SKILL_DIR_NAMES)
    assert _installed_skill_dirs(homes["opencode"]) == set(SKILL_DIR_NAMES)
    assert _asset_install_rows(registry_db) == [
        ("codex", CLI_VERSION),
        ("opencode", CLI_VERSION),
    ]


def test_skip_all_three_is_schema_only(homes, registry_db, monkeypatch):
    """Skipping all three agents runs the db half only: no network access, no
    installs, no rows — the documented schema-only path."""
    _forbid_network(monkeypatch)

    result = _run_setup(SCHEMA_ONLY_ARGS)

    assert result.exit_code == 0, result.output
    assert f"Created {registry_db} and applied migrations to head" in result.output
    assert ASSETS_SKIPPED_LINE in result.output
    assert {"fleets", "asset_installs", "alembic_version"} <= _table_names(registry_db)
    assert _asset_install_rows(registry_db) == []
    for agent in AGENTS:
        assert _installed_skill_dirs(homes[agent]) == set()


def test_skip_all_three_db_failure_reports_db_half_only(
    homes, registry_db, monkeypatch
):
    """With every agent skipped, a db failure is the only reported failure —
    the skipped assets half is not-run and cannot contribute."""
    from cafleet.cli import setup as setup_module

    _forbid_network(monkeypatch)

    def broken_run_db_init():
        raise click.ClickException("disk full")

    monkeypatch.setattr(setup_module, "run_db_init", broken_run_db_init)

    result = _run_setup(SCHEMA_ONLY_ARGS)

    assert result.exit_code == 1, result.output
    assert "db half failed: disk full" in result.output
    assert "assets half failed" not in result.output
    assert "Error: db half failed" in result.output


def test_skip_invalid_choice(homes, registry_db, monkeypatch):
    """An unknown ``--skip`` value fails with Click's invalid-choice error."""
    _forbid_network(monkeypatch)

    result = _run_setup(["--skip", "vim"])

    assert result.exit_code == 2, result.output
    assert "invalid value" in result.output.lower()
    assert "vim" in result.output


# --------------------------------------------------------------------------- #
# Removed subcommands: ``db``, per-agent, ``skill``                            #
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("word", ["db", "claude", "codex", "opencode", "skill"])
def test_removed_subcommand_fails_with_extra_argument(
    homes, registry_db, monkeypatch, word
):
    """Former subcommand invocations fail with Click's standard extra-argument
    error (exit 2) and touch neither the DB nor the network."""
    _forbid_network(monkeypatch)

    result = _run_setup([word])

    assert result.exit_code == 2, result.output
    assert "got unexpected extra argument" in result.output.lower()
    assert word in result.output
    assert not registry_db.exists()


# --------------------------------------------------------------------------- #
# Idempotency and row upserts                                                  #
# --------------------------------------------------------------------------- #


def test_schema_only_idempotent_and_preserves_rows(homes, registry_db, monkeypatch):
    """A second schema-only run reports ``Already at head`` and never touches
    recorded rows."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    assert _run_setup(_only("claude")).exit_code == 0
    assert _asset_install_rows(registry_db) == [("claude", CLI_VERSION)]

    result = _run_setup(SCHEMA_ONLY_ARGS)

    assert result.exit_code == 0, result.output
    assert "already at head" in result.output.lower()
    assert _asset_install_rows(registry_db) == [("claude", CLI_VERSION)]


def test_reinstall_upserts_row(homes, registry_db, monkeypatch):
    """Re-installing replaces the agent's row instead of adding a second one."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    assert _run_setup(_only("claude")).exit_code == 0
    assert _asset_install_rows(registry_db) == [("claude", CLI_VERSION)]

    _mock_release(
        monkeypatch,
        version="0.12.3",
        zip_bytes=_make_assets_zip(),
        assets=[
            {
                "name": "cafleet-assets-v0.12.3.zip",
                "browser_download_url": "https://example.invalid/download/cafleet-assets-v0.12.3.zip",
            }
        ],
    )

    result = _run_setup(_only("claude"))

    assert result.exit_code == 0, result.output
    assert _asset_install_rows(registry_db) == [("claude", "0.12.3")]


def test_skills_install_failure_keeps_prior_rows(homes, registry_db, monkeypatch):
    """A skills failure aborts the assets loop; rows for completed targets remain."""
    from cafleet.cli import setup as setup_module

    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    real_copytree = shutil.copytree

    def failing_copytree(src, dst, *args, **kwargs):
        if "h_codex" in str(dst):
            raise OSError("disk full")
        return real_copytree(src, dst, *args, **kwargs)

    monkeypatch.setattr(setup_module.shutil, "copytree", failing_copytree)

    result = _run_setup()

    assert result.exit_code == 1, result.output
    assert "failed to install skills into" in result.output
    assert _asset_install_rows(registry_db) == [("claude", CLI_VERSION)]


# --------------------------------------------------------------------------- #
# Preset install semantics: overwrite matrix + failure                         #
# --------------------------------------------------------------------------- #


def test_preset_overwrites_existing_file(
    homes, preset_targets, registry_db, monkeypatch
):
    """An existing regular file at the target is replaced by the bundled copy."""
    target = preset_targets["opencode"]
    target.parent.mkdir(parents=True)
    target.write_text("operator custom\n", encoding="utf-8")
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(_only("opencode"))

    assert result.exit_code == 0, result.output
    assert not target.is_symlink()
    assert target.read_text(encoding="utf-8") == OPENCODE_PRESET_CONTENT


def test_preset_overwrites_existing_directory(
    homes, preset_targets, registry_db, monkeypatch
):
    """An existing directory at the target is removed and replaced by the file."""
    target = preset_targets["opencode"]
    target.mkdir(parents=True)
    (target / "nested.md").write_text("x\n", encoding="utf-8")
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(_only("opencode"))

    assert result.exit_code == 0, result.output
    assert target.is_file()
    assert target.read_text(encoding="utf-8") == OPENCODE_PRESET_CONTENT


def test_preset_overwrites_symlink_to_file(
    tmp_path, homes, preset_targets, registry_db, monkeypatch
):
    """A symlink target is unlinked and replaced by a regular file; the
    linked-to file itself is untouched."""
    real = tmp_path / "elsewhere" / "real.md"
    real.parent.mkdir(parents=True)
    real.write_text("keep\n", encoding="utf-8")
    target = preset_targets["opencode"]
    target.parent.mkdir(parents=True)
    target.symlink_to(real)
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(_only("opencode"))

    assert result.exit_code == 0, result.output
    assert not target.is_symlink()
    assert target.is_file()
    assert target.read_text(encoding="utf-8") == OPENCODE_PRESET_CONTENT
    assert real.read_text(encoding="utf-8") == "keep\n"


def test_preset_overwrites_symlink_to_directory(
    tmp_path, homes, preset_targets, registry_db, monkeypatch
):
    """A symlink-to-directory target is unlinked (the symlink check precedes
    the dir check); the pointed-to directory and its contents stay intact."""
    real_dir = tmp_path / "elsewhere" / "agents_dir"
    real_dir.mkdir(parents=True)
    (real_dir / "keep.md").write_text("keep\n", encoding="utf-8")
    target = preset_targets["opencode"]
    target.parent.mkdir(parents=True)
    target.symlink_to(real_dir, target_is_directory=True)
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(_only("opencode"))

    assert result.exit_code == 0, result.output
    assert not target.is_symlink()
    assert target.is_file()
    assert target.read_text(encoding="utf-8") == OPENCODE_PRESET_CONTENT
    assert real_dir.is_dir()
    assert (real_dir / "keep.md").read_text(encoding="utf-8") == "keep\n"


def test_preset_install_failure_aborts_and_skips_row(
    homes, preset_targets, registry_db, monkeypatch
):
    """A filesystem error on the preset install exits 1 with the preset error
    string; the agent's row is not recorded even though its skills installed."""
    # ``~/.codex/rules`` exists as a regular file: the parent-dir creation fails.
    rules_parent = preset_targets["codex"].parent
    rules_parent.parent.mkdir(parents=True, exist_ok=True)
    rules_parent.write_text("not a directory\n", encoding="utf-8")
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(_only("codex"))

    assert result.exit_code == 1, result.output
    assert f"failed to install preset into {preset_targets['codex']}" in result.output
    assert _installed_skill_dirs(homes["codex"]) == set(SKILL_DIR_NAMES)
    assert _asset_install_rows(registry_db) == []


# --------------------------------------------------------------------------- #
# Archive integrity (claude-only invocation)                                   #
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("evil_member", ["../evil.txt", "/abs/evil.txt"])
def test_zip_slip_member_rejected_nothing_extracted(
    homes, registry_db, monkeypatch, evil_member
):
    """A ``..``/absolute member is rejected before extraction; targets untouched."""
    sentinel = homes["claude"] / "cafleet" / "SENTINEL.md"
    sentinel.parent.mkdir(parents=True)
    sentinel.write_text("keep")
    members = [f"skills/{name}/SKILL.md" for name in SKILL_DIR_NAMES]
    members += [path for path, _ in PRESET_ARCHIVE_MEMBERS.values()]
    members += [evil_member]
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip(raw_members=members))

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert sentinel.exists()  # nothing extracted, nothing removed
    assert sentinel.read_text() == "keep"
    assert _asset_install_rows(registry_db) == []


@pytest.mark.parametrize(
    "zip_bytes",
    [
        _make_assets_zip(extra_dirs=("rogue",)),
        _make_assets_zip(extra_files=("README.md",)),
    ],
    ids=["extra-dir", "stray-file"],
)
def test_extra_entry_under_skills_is_malformed(
    homes, registry_db, monkeypatch, zip_bytes
):
    """Any entry under ``skills/`` beyond the four skill dirs is malformed."""
    _mock_release(monkeypatch, zip_bytes=zip_bytes)

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_missing_skill_dir_is_malformed(homes, registry_db, monkeypatch):
    """A ``skills/`` holding only a subset of the four skill dirs is malformed."""
    _mock_release(
        monkeypatch,
        zip_bytes=_make_assets_zip(skill_dirs=("cafleet", "cafleet-design-doc")),
    )

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_missing_refresh_skill_dir_is_malformed(homes, registry_db, monkeypatch):
    """An archive with the legacy three skill dirs (no refresh skill) is malformed."""
    _mock_release(
        monkeypatch,
        zip_bytes=_make_assets_zip(
            skill_dirs=("cafleet", "cafleet-design-doc", "cafleet-research")
        ),
    )

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()
    assert _asset_install_rows(registry_db) == []


@pytest.mark.parametrize(
    "zip_kwargs",
    [{"cafleet_manifest": False}, {"cafleet_catalog": False}],
    ids=["missing-asset-manifest", "missing-model-catalog"],
)
def test_archive_missing_catalog_member_is_malformed(
    homes, registry_db, monkeypatch, zip_kwargs
):
    """The cafleet asset manifest and model catalog ship in every archive; an
    archive lacking either is malformed and records no install."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip(**zip_kwargs))

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()
    assert _asset_install_rows(registry_db) == []


def test_installed_replicas_carry_manifest_and_catalog(
    homes, preset_targets, registry_db, monkeypatch
):
    """Every per-backend cafleet replica receives the asset manifest and the
    model catalog byte-for-byte — the atomic-distribution guarantee."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup()

    assert result.exit_code == 0, result.output
    for agent in AGENTS:
        replica = homes[agent] / "cafleet"
        assert (replica / "asset-manifest.json").read_text(
            encoding="utf-8"
        ) == MANIFEST_CONTENT
        assert (replica / "reference" / "model-catalog.md").read_text(
            encoding="utf-8"
        ) == CATALOG_CONTENT


def test_reinstall_overwrites_stale_replica_manifest_and_catalog(
    homes, registry_db, monkeypatch
):
    """A pre-existing replica's manifest and catalog are replaced by the
    archive copies on reinstall, so stale fingerprints never survive setup."""
    stale_replica = homes["claude"] / "cafleet"
    (stale_replica / "reference").mkdir(parents=True)
    (stale_replica / "asset-manifest.json").write_text(
        '{"cafleet_version": "0.0.1"}\n', encoding="utf-8"
    )
    (stale_replica / "reference" / "model-catalog.md").write_text(
        "stale catalog\n", encoding="utf-8"
    )
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    result = _run_setup(_only("claude"))

    assert result.exit_code == 0, result.output
    assert (stale_replica / "asset-manifest.json").read_text(
        encoding="utf-8"
    ) == MANIFEST_CONTENT
    assert (stale_replica / "reference" / "model-catalog.md").read_text(
        encoding="utf-8"
    ) == CATALOG_CONTENT


@pytest.mark.parametrize(
    "preset_agents",
    [("codex",), ("opencode",)],
    ids=["missing-opencode-preset", "missing-codex-rules"],
)
def test_archive_missing_preset_is_malformed(
    homes, registry_db, monkeypatch, preset_agents
):
    """Layout validation requires both preset archive sources as regular files,
    whatever the install target is."""
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip(preset_agents=preset_agents))

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()
    assert _asset_install_rows(registry_db) == []


def test_bad_zip_is_malformed(homes, registry_db, monkeypatch):
    """A non-zip / truncated download surfaces as the malformed-asset error."""
    _mock_release(monkeypatch, zip_bytes=b"this is not a zip archive")

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


def test_extractall_oserror_is_malformed(homes, registry_db, monkeypatch):
    """An ``OSError`` raised mid-extraction is reported as the malformed-asset error.

    The archive passes the zip-slip and bad-zip pre-checks; the failure happens
    inside ``ZipFile.extractall`` (e.g. a filesystem error during extraction).
    """
    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    def boom(self, *args, **kwargs):
        raise OSError("extraction failed")

    monkeypatch.setattr(zipfile.ZipFile, "extractall", boom)

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "malformed" in result.output.lower()


# --------------------------------------------------------------------------- #
# Release / network resolution (claude-only invocation)                        #
# --------------------------------------------------------------------------- #


def test_missing_asset_message(homes, registry_db, monkeypatch):
    """A release lacking the expected asset name surfaces the specific
    not-found message — the lookup matches exactly one name, no fallback."""
    _mock_release(
        monkeypatch,
        assets=[
            {
                "name": "other-asset.zip",
                "browser_download_url": DOWNLOAD_URL,
            }
        ],
    )

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert f"asset {ASSET_NAME} not found in release {CLI_VERSION}" in result.output
    assert "Error: assets half failed" in result.output


@pytest.mark.parametrize(
    "release_body",
    [b"this is not json", b'{"tag_name": "0.12.2"}'],
    ids=["non-json", "no-assets-key"],
)
def test_unparseable_api_response(homes, registry_db, monkeypatch, release_body):
    """A 200 body that is not JSON, or lacks the ``assets`` array, is rejected."""
    _mock_release(monkeypatch, release_body=release_body)

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "could not parse the github api response" in result.output.lower()


def test_no_release_for_version(homes, registry_db, monkeypatch):
    """A 404 from ``/releases/tags/<version>`` is the no-release-for-version
    error, and it remains a loud exit-1 failure."""
    err = urllib.error.HTTPError(
        url="https://api.github.com/repos/himkt/cafleet/releases/tags/0.12.2",
        code=404,
        msg="Not Found",
        hdrs=None,
        fp=None,
    )
    _mock_release(monkeypatch, api_error=err)

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert f"no release found for version {CLI_VERSION}" in result.output
    assert "Error: assets half failed" in result.output


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
    _mock_release(monkeypatch, api_error=api_error)

    result = _run_setup(_only("claude"))

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
    _mock_release(
        monkeypatch, zip_bytes=_make_assets_zip(), download_error=download_error
    )

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    assert "could not reach the github api" in result.output.lower()


# --------------------------------------------------------------------------- #
# Install-time filesystem errors (claude-only invocation)                      #
# --------------------------------------------------------------------------- #


def test_unwritable_target_permission_error(homes, registry_db, monkeypatch):
    """A ``PermissionError`` during install surfaces as a ClickException."""
    from cafleet.cli import setup as setup_module

    _mock_release(monkeypatch, zip_bytes=_make_assets_zip())

    def deny_copytree(src, dst, *args, **kwargs):
        raise PermissionError(13, "Permission denied", str(dst))

    monkeypatch.setattr(setup_module.shutil, "copytree", deny_copytree)

    result = _run_setup(_only("claude"))

    assert result.exit_code == 1, result.output
    out = result.output.lower()
    assert "permission" in out or "denied" in out
    assert _asset_install_rows(registry_db) == []
