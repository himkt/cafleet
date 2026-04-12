"""Tests for the ``hikyaku-registry db init`` CLI command.

Covers four states from the design doc's CLI Specification behavior
matrix (design-docs/0000010-sqlite-store-migration/design-doc.md
section "db init behavior matrix"):

  | # | State                          | Tested by                          |
  |---|--------------------------------|------------------------------------|
  | 1 | DB file does not exist         | ``test_db_init_creates_schema``    |
  | 3 | At head                        | ``test_db_init_idempotent`` (2nd)  |
  | 5 | Ahead of head                  | ``test_db_init_ahead_errors``      |
  | 6 | Legacy (tables, no version)    | ``test_db_init_legacy_errors``     |

States NOT exercised here (intentional, per the Step 4 Phase A scope):

  - **State #2 "Empty schema"**: structurally identical to state #1
    once parent-dir creation is verified — both call ``upgrade head``.
    The state #1 happy-path test covers the only behaviorally distinct
    branch (mkdir + apply).
  - **State #4 "Behind head"**: physically unreachable in v1 because
    only one migration script ships. Add a test in a follow-up when a
    second migration lands.

Test isolation strategy:

  Each test crafts its own ``tmp_path`` layout, then monkeypatches
  ``config.settings.database_url`` to point at that path BEFORE
  importing the CLI. The CLI is imported INSIDE each test body so
  any module-level reads of the database URL during ``cli`` import
  see the patched value, not the user's real ``HIKYAKU_DATABASE_URL``.

  Why ``monkeypatch.setattr(config.settings, "database_url", ...)``
  rather than ``monkeypatch.setenv("HIKYAKU_DATABASE_URL", ...)``:
  ``config.settings`` is a module-level singleton constructed at
  ``hikyaku_registry.config`` import time. By the time any test
  runs, the singleton has already been built — env-var changes
  after that point would be ignored. Patching the attribute on the
  existing singleton is the only reliable override.

  Why per-test setup instead of a shared fixture: the four tests
  exercise visibly different DB pre-states (missing file, missing
  parent dir, legacy tables, future revision). Inlining the setup in
  each test makes the precondition obvious at the call site.
"""

import sqlite3

from click.testing import CliRunner

from hikyaku_registry import config


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


def test_db_init_creates_schema(tmp_path, monkeypatch):
    """db init on a missing file: parent dir + DB + four tables created.

    Verifies the design doc's state #1 (DB file does not exist):

      "Create parent directories. Run ``command.upgrade(cfg, 'head')``.
       Print 'Created {path} and applied N migration(s) to head
       ({head_rev})'. Exit 0."

    The DB path is placed under a not-yet-existing ``data/`` subdir
    so the assertion that ``Path.parent.mkdir(parents=True,
    exist_ok=True)`` actually fired is meaningful.
    """
    db_file = tmp_path / "data" / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    assert not db_file.parent.exists(), (
        "fixture sanity: data/ subdir should not pre-exist"
    )

    from hikyaku_registry.cli import main

    runner = CliRunner()
    result = runner.invoke(main, ["db", "init"])

    assert result.exit_code == 0, (
        f"db init failed.\noutput: {result.output}\nexception: {result.exception}"
    )
    assert db_file.parent.exists(), (
        "db init should have created the parent directory via "
        "Path.parent.mkdir(parents=True, exist_ok=True)"
    )
    assert db_file.exists(), f"db init should have created the DB file at {db_file}"

    tables = _table_names(db_file)
    expected = {"sessions", "agents", "tasks", "agent_placements", "alembic_version"}
    missing = expected - tables
    assert not missing, (
        f"db init did not create all expected tables. "
        f"missing: {sorted(missing)}, found: {sorted(tables)}"
    )

    assert "applied" in result.output.lower(), (
        f"db init success output should mention 'applied' (the design "
        f"doc messages are 'Created ... and applied N migration(s)' or "
        f"'Applied N migration(s) ...'). got: {result.output!r}"
    )


def test_db_init_idempotent(tmp_path, monkeypatch):
    """Running db init twice on a fresh DB: applies once, then no-ops.

    Verifies the design doc claim:

      "db init is idempotent: running it twice on a fresh DB applies
       migrations once and then becomes a no-op."

    The second invocation transitions the DB through state #3 ("At
    head"):

      "No-op. Print 'Already at head ({head_rev}); nothing to do'."

    Asserts:
      1. Both invocations exit 0.
      2. Schema is unchanged between the two runs (no tables added,
         no tables dropped, ``alembic_version`` content stable).
      3. Second invocation's output mentions "already at head" (loose
         match), distinguishing the no-op branch from the apply branch.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    from hikyaku_registry.cli import main

    runner = CliRunner()

    first = runner.invoke(main, ["db", "init"])
    assert first.exit_code == 0, (
        f"first db init failed.\noutput: {first.output}\nexception: {first.exception}"
    )

    tables_after_first = _table_names(db_file)
    expected = {"sessions", "agents", "tasks", "agent_placements", "alembic_version"}
    assert expected <= tables_after_first, (
        f"first db init should have produced all expected tables; "
        f"missing: {sorted(expected - tables_after_first)}"
    )

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_first = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()

    second = runner.invoke(main, ["db", "init"])
    assert second.exit_code == 0, (
        f"second db init failed.\n"
        f"output: {second.output}\n"
        f"exception: {second.exception}"
    )
    assert "already at head" in second.output.lower(), (
        f"second db init should print an 'already at head' message per "
        f"the design doc state-3 spec. got: {second.output!r}"
    )

    tables_after_second = _table_names(db_file)
    assert tables_after_second == tables_after_first, (
        f"schema must be unchanged across the two db init runs. "
        f"first: {sorted(tables_after_first)}, "
        f"second: {sorted(tables_after_second)}"
    )

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_second = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()
    assert version_after_second == version_after_first, (
        f"alembic_version row must be stable across runs. "
        f"first: {version_after_first}, second: {version_after_second}"
    )


def test_db_init_legacy_errors(tmp_path, monkeypatch):
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
      2. The error message mentions ``alembic stamp head`` (recovery hint).
      3. The legacy table is still present and ``alembic_version`` is
         still absent — i.e., the failing CLI did NOT mutate the DB.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute("CREATE TABLE legacy_squat (id INTEGER PRIMARY KEY)")
        conn.commit()
    finally:
        conn.close()

    from hikyaku_registry.cli import main

    runner = CliRunner()
    result = runner.invoke(main, ["db", "init"])

    assert result.exit_code != 0, (
        f"db init should error on a legacy schema (tables but no "
        f"alembic_version), got exit_code={result.exit_code}.\n"
        f"output: {result.output}"
    )
    assert "alembic stamp head" in result.output, (
        f"legacy error message should mention 'alembic stamp head' as "
        f"the recovery path. got: {result.output!r}"
    )

    tables = _table_names(db_file)
    assert "legacy_squat" in tables, (
        "the failing CLI must not have dropped the pre-existing legacy table"
    )
    assert "alembic_version" not in tables, (
        "the failing CLI must not have auto-stamped the legacy DB; "
        "alembic_version should still be absent"
    )


def test_db_init_ahead_errors(tmp_path, monkeypatch):
    """A DB at an unknown future revision: exit non-zero, no downgrade.

    Verifies the design doc's state #5 (Ahead of head):

      "Current revision exists but is not in the local script
       directory's history (or is downstream of head). Print 'ERROR: DB
       schema is at revision {current_rev} which is unknown to this
       version of hikyaku-registry. Refusing to downgrade automatically.'
       to stderr. Exit 1."

    The fictional revision id ``9999_future_revision`` is chosen to be
    obviously unknown to the local Alembic script directory. Any string
    that isn't a real revision works — Alembic looks up the revision
    in the script_directory and treats "not found" as ahead-of-head.

    Asserts:
      1. Exit code is non-zero (refusal, not silent acceptance).
      2. The error message mentions "ahead", "unknown", or the offending
         revision id (loose match) so the operator knows what's wrong.
      3. ``alembic_version`` content is unchanged after the failed run
         (CLI must NOT silently rewrite the version row, since that
         would mask the version mismatch).
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "CREATE TABLE alembic_version "
            "(version_num VARCHAR(32) NOT NULL PRIMARY KEY)"
        )
        conn.execute(
            "INSERT INTO alembic_version (version_num) VALUES ('9999_future_revision')"
        )
        conn.commit()
    finally:
        conn.close()

    from hikyaku_registry.cli import main

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
        or "ahead" in output_lower
        or "9999_future_revision" in result.output
    ), (
        f"ahead-of-head error message should mention 'unknown', 'ahead', "
        f"or the offending revision id. got: {result.output!r}"
    )

    conn = sqlite3.connect(str(db_file))
    try:
        rows = conn.execute("SELECT version_num FROM alembic_version").fetchall()
    finally:
        conn.close()
    assert rows == [("9999_future_revision",)], (
        f"the failing CLI must not have rewritten alembic_version; "
        f"expected [('9999_future_revision',)], got {rows}"
    )
