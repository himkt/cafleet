"""Tests for the ``hikyaku-registry db init`` CLI command.

Covers four of the six states from the design doc's CLI Specification
behavior matrix (design-docs/0000010-sqlite-store-migration/design-doc.md
section "db init behavior matrix"):

  | # | State                          | Tested by                          |
  |---|--------------------------------|------------------------------------|
  | 1 | DB file does not exist         | ``test_db_init_creates_schema``    |
  | 3 | At head                        | ``test_db_init_idempotent`` (2nd)  |
  | 5 | Ahead of head                  | ``test_db_init_ahead_errors``      |
  | 6 | Legacy (tables, no version)    | ``test_db_init_legacy_errors``     |

The two states *not* explicitly tested here:

  - **Empty schema** (state 2): structurally identical to "DB file
    missing" once parent-dir creation is verified — both call
    ``upgrade head`` and print the "Applied N migration(s)" line. The
    distinction is mostly in the print message, which is not load-
    bearing for downstream behavior.
  - **Behind head** (state 4): would require two migrations to test
    (start at revision X, run init, end at revision X+1). v1 only ships
    one migration, so this case is unreachable until 0002_*.py exists.
    Add a test in a follow-up when a second migration lands.

The fifth test, ``test_cli_db_init_help_loads``, satisfies the design
doc's "Verify the entry point installs by running
``uv run hikyaku-registry db init --help``" task.

Test isolation strategy:

  Each test gets a fresh ``tmp_path`` with a not-yet-existing
  ``data/registry.db`` subdir layout. The fixture monkeypatches
  ``config.settings.database_url`` to point at that path BEFORE the
  CLI loads the URL, so each test sees an isolated DB and the CLI
  is exercised end-to-end (no module-level patching of CLI internals).

  The CLI is invoked via ``click.testing.CliRunner``, which captures
  output and exit codes without spawning a subprocess. This is fast
  and lets us assert on the exact exit code and message text.
"""

import sqlite3

import pytest
from click.testing import CliRunner

from hikyaku_registry import config
from hikyaku_registry.cli import main


@pytest.fixture
def tmp_db(tmp_path, monkeypatch):
    """Build a tempfile DB path under a not-yet-existing parent dir.

    The path is ``{tmp_path}/data/registry.db``. ``data/`` does NOT
    exist when this fixture returns — that's deliberate, so tests can
    verify the CLI's ``Path(db_file).parent.mkdir(parents=True,
    exist_ok=True)`` step actually fires.

    ``config.settings.database_url`` is patched to the
    ``sqlite+aiosqlite://`` form (the production format) for this path.
    The CLI is responsible for converting to the sync driver
    internally; this fixture does not pre-convert.
    """
    db_path = tmp_path / "data" / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_path}",
    )
    return db_path


def _table_names(db_path) -> set[str]:
    """Return the set of user-visible table names in a SQLite file.

    Uses a sync ``sqlite3`` connection rather than SQLAlchemy to keep
    the assertion plumbing minimal — these tests are about the CLI's
    behavior, not about exercising the SQLAlchemy stack.
    """
    conn = sqlite3.connect(str(db_path))
    try:
        rows = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
        return {row[0] for row in rows}
    finally:
        conn.close()


def test_db_init_creates_schema(tmp_db):
    """db init on a missing file: parent dir + DB + four tables created.

    Verifies the design doc's state #1 (DB file does not exist):

      "Create parent directories. Run ``command.upgrade(cfg, 'head')``.
       Print 'Created {path} and applied N migration(s) to head
       ({head_rev})'. Exit 0."

    Asserts:
      1. The parent directory did NOT exist before invocation (sanity
         check on the fixture, so the next assertion is meaningful).
      2. Exit code is 0.
      3. The parent directory exists after invocation (mkdir parents
         worked).
      4. The DB file exists after invocation.
      5. All four expected tables (``api_keys``, ``agents``, ``tasks``,
         ``alembic_version``) are present in the resulting DB.
    """
    assert not tmp_db.parent.exists(), (
        "fixture sanity: data/ subdir should not pre-exist"
    )

    runner = CliRunner()
    result = runner.invoke(main, ["db", "init"])

    assert result.exit_code == 0, (
        f"db init failed.\n"
        f"output: {result.output}\n"
        f"exception: {result.exception}"
    )
    assert tmp_db.parent.exists(), (
        "db init should have created the parent directory via mkdir(parents=True)"
    )
    assert tmp_db.exists(), (
        f"db init should have created the DB file at {tmp_db}"
    )

    tables = _table_names(tmp_db)
    expected = {"api_keys", "agents", "tasks", "alembic_version"}
    missing = expected - tables
    assert not missing, (
        f"db init did not create all expected tables. "
        f"missing: {sorted(missing)}, found: {sorted(tables)}"
    )


def test_db_init_idempotent(tmp_db):
    """Running db init twice on a fresh DB: applies once, then no-ops.

    Verifies the design doc claim:

      "db init is idempotent: running it twice on a fresh DB applies
       migrations once and then becomes a no-op."

    The second invocation transitions the DB through state #3 ("At
    head"):

      "No-op. Print 'Already at head ({head_rev}); nothing to do'."

    Asserts:
      1. First invocation exits 0 (creates schema).
      2. Second invocation exits 0 (no-op path).
      3. Second invocation's output mentions "Already at head" — the
         design-doc-specified message that distinguishes the no-op
         branch from the apply branch.
      4. The DB still has all four tables after the second run (the
         no-op branch did not wipe or corrupt anything).
    """
    runner = CliRunner()

    first = runner.invoke(main, ["db", "init"])
    assert first.exit_code == 0, (
        f"first db init failed.\n"
        f"output: {first.output}\n"
        f"exception: {first.exception}"
    )

    second = runner.invoke(main, ["db", "init"])
    assert second.exit_code == 0, (
        f"second db init failed.\n"
        f"output: {second.output}\n"
        f"exception: {second.exception}"
    )
    assert "Already at head" in second.output, (
        f"second db init should print 'Already at head' per the design "
        f"doc state-3 message. got: {second.output!r}"
    )

    tables = _table_names(tmp_db)
    expected = {"api_keys", "agents", "tasks", "alembic_version"}
    assert expected <= tables, (
        f"DB should still contain all expected tables after the second "
        f"(no-op) db init. missing: {sorted(expected - tables)}"
    )


def test_db_init_legacy_errors(tmp_db):
    """A DB with hand-created tables but no alembic_version: exit non-zero.

    Verifies the design doc's state #6 (Legacy):

      "DB has tables but MigrationContext.get_current_revision() returns
       None. Print 'ERROR: DB has existing tables but no
       alembic_version. Run \"alembic stamp head\" manually if you are
       sure the schema matches.' to stderr. **No auto-stamp.**"

    The "no auto-stamp" rule is critical — silently stamping a legacy
    DB to head would lie about its revision and could mask schema
    mismatches at runtime.

    Asserts:
      1. Exit code is non-zero.
      2. The error message references ``alembic stamp head`` so the
         operator knows the recovery path.
      3. The legacy table is still present (the failing CLI did not
         drop or corrupt it).
    """
    tmp_db.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(tmp_db))
    try:
        conn.execute("CREATE TABLE legacy_squat (id INTEGER PRIMARY KEY)")
        conn.commit()
    finally:
        conn.close()

    runner = CliRunner()
    result = runner.invoke(main, ["db", "init"])

    assert result.exit_code != 0, (
        f"db init should error on a legacy schema (tables but no "
        f"alembic_version), got exit_code={result.exit_code}.\n"
        f"output: {result.output}"
    )
    assert "alembic stamp head" in result.output, (
        f"legacy error message should mention 'alembic stamp head' as the "
        f"recovery path. got: {result.output!r}"
    )

    tables = _table_names(tmp_db)
    assert "legacy_squat" in tables, (
        "the failing CLI must not have dropped the pre-existing legacy table"
    )
    assert "alembic_version" not in tables, (
        "the failing CLI must not have auto-stamped the legacy DB; "
        "alembic_version should still be absent"
    )


def test_db_init_ahead_errors(tmp_db):
    """A DB at an unknown future revision: exit non-zero, no downgrade.

    Verifies the design doc's state #5 (Ahead of head):

      "Current revision exists but is not in the local script
       directory's history (or is downstream of head). Print 'ERROR: DB
       schema is at revision {current_rev} which is unknown to this
       version of hikyaku-registry. Refusing to downgrade automatically.'
       to stderr. Exit 1."

    The fictional revision id ``zzz_future_rev_xyz`` is chosen to be
    obviously unknown to the local Alembic script directory. Any string
    that isn't a real revision works — Alembic looks up the revision in
    the script_directory and treats "not found" as ahead-of-head.

    Asserts:
      1. Exit code is non-zero (refusal, not silent acceptance).
      2. The error message mentions "unknown" or the fictional revision
         id, so the operator knows what they're rolling back from.
      3. The fictional revision is still in alembic_version after the
         failed CLI invocation (the CLI must NOT have rewritten the
         version row, since that would silently mask the version
         mismatch).
    """
    tmp_db.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(tmp_db))
    try:
        conn.execute(
            "CREATE TABLE alembic_version "
            "(version_num VARCHAR(32) NOT NULL PRIMARY KEY)"
        )
        conn.execute(
            "INSERT INTO alembic_version (version_num) "
            "VALUES ('zzz_future_rev_xyz')"
        )
        conn.commit()
    finally:
        conn.close()

    runner = CliRunner()
    result = runner.invoke(main, ["db", "init"])

    assert result.exit_code != 0, (
        f"db init should refuse an ahead-of-head DB, "
        f"got exit_code={result.exit_code}.\n"
        f"output: {result.output}"
    )
    output_lower = result.output.lower()
    assert (
        "unknown" in output_lower
        or "zzz_future_rev_xyz" in result.output
    ), (
        f"ahead-of-head error message should mention 'unknown' or the "
        f"offending revision id. got: {result.output!r}"
    )

    conn = sqlite3.connect(str(tmp_db))
    try:
        rows = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()
    assert rows == [("zzz_future_rev_xyz",)], (
        f"the failing CLI must not have rewritten alembic_version; "
        f"expected [('zzz_future_rev_xyz',)], got {rows}"
    )


def test_cli_db_init_help_loads():
    """The ``hikyaku-registry db init --help`` invocation succeeds.

    A regression guard for the design doc's task:

      "Verify the entry point installs by running ``uv run
       hikyaku-registry db init --help`` and confirming output."

    This test does NOT spawn a subprocess (which would actually
    exercise the installed entry point in ``[project.scripts]``);
    instead it imports ``main`` directly and invokes it via
    ``CliRunner``. The reason: a subprocess test would only verify
    the entry point AS INSTALLED at test runtime, which depends on
    whether ``uv sync`` has run. The in-process invocation verifies
    that the click command tree is wired correctly (``main`` exposes
    a ``db`` subgroup, which exposes an ``init`` subcommand), which
    is what the entry point indirectly relies on. A separate
    end-to-end install check belongs in CI, not in unit tests.
    """
    runner = CliRunner()
    result = runner.invoke(main, ["db", "init", "--help"])

    assert result.exit_code == 0, (
        f"`db init --help` failed.\n"
        f"output: {result.output}\n"
        f"exception: {result.exception}"
    )
    assert "init" in result.output.lower(), (
        f"--help output should describe the init command. "
        f"got: {result.output!r}"
    )
