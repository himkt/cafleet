"""Regression tests for the SQL-backed RegistryStore.

Two regression tests called out in the design doc Testing Strategy
(design-docs/0000010-sqlite-store-migration/design-doc.md §"New
regression tests"):

  | Test                              | Verifies                            |
  |-----------------------------------|-------------------------------------|
  | ``test_revoke_api_key_atomic``    | Revoking with N agents commits all  |
  |                                   | updates as a single transaction.    |
  | ``test_revoke_api_key_atomic_``   | A failure mid-transaction rolls     |
  | ``failure_rolls_back``            | back the api_key flip too.          |
  | ``test_list_api_keys_no_n_``      | ``list_api_keys`` emits exactly one |
  | ``plus_one``                      | SELECT against ``api_keys``.        |

These tests target the two pain points the migration is designed to fix
(see Background bullets 3 and 4 in the design doc). They are intentionally
NARROW: each test checks one observable invariant of the new SQL-backed
RegistryStore. The broader behavioral coverage of ``RegistryStore``
methods will land when ``test_registry_store.py`` is rewritten in a
later step.

Test isolation strategy:

  Function-scoped in-memory aiosqlite engine + ``Base.metadata.create_all``.
  Each test gets a fresh DB, so cross-test contamination is impossible.
  These fixtures are LOCAL to this file (not pulled from ``conftest.py``)
  for two reasons:

  1. ``conftest.py`` still imports ``fakeredis.aioredis`` from the old
     setup and is broken at module load. Until Step 12 rewrites it, any
     test that depends on conftest fixtures cannot collect.
  2. The tests below need to install per-test SQLAlchemy event listeners
     on the engine. A function-scoped engine guarantees clean teardown
     of those listeners (the engine is disposed at the end of each
     test), avoiding listener leakage across tests.

Why setup uses raw SQL instead of store methods:

  ``test_revoke_api_key_atomic_failure_rolls_back`` injects a failure
  via a SQLAlchemy event listener that raises on ``UPDATE agents``
  statements. If the test setup itself uses ``store.create_agent``
  (which under the new implementation will emit an INSERT against
  agents — distinct from UPDATE — but the listener is narrowly scoped
  to UPDATEs anyway), the test would still work. But to keep setup
  decoupled from any future change in store method signatures or
  internal SQL, all setup goes through the four ``_seed_*`` /
  ``_read_*`` helpers below, which use the table column names directly.
"""

import json
import uuid
from datetime import UTC, datetime

import pytest
from sqlalchemy import event, text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

from hikyaku_registry.db.models import Base
from hikyaku_registry.registry_store import RegistryStore


@pytest.fixture
async def engine():
    """Function-scoped in-memory aiosqlite engine with FK enforcement."""
    e = create_async_engine("sqlite+aiosqlite:///:memory:")

    @event.listens_for(e.sync_engine, "connect")
    def _enable_fk(dbapi_conn, _):
        dbapi_conn.execute("PRAGMA foreign_keys=ON")

    async with e.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

    yield e
    await e.dispose()


@pytest.fixture
async def store(engine):
    """A fresh RegistryStore bound to the function-scoped engine."""
    sessionmaker = async_sessionmaker(engine, expire_on_commit=False)
    return RegistryStore(sessionmaker)


# ---------------------------------------------------------------------------
# Setup / verification helpers
#
# These talk directly to the schema rather than going through store
# methods, so the tests are immune to changes in store method signatures
# or internal SQL choices. The only coupling is to the table/column
# names from db/models.py — which IS what we want to verify against.
# ---------------------------------------------------------------------------


async def _seed_api_key(
    engine,
    *,
    api_key_hash: str,
    owner_sub: str,
    key_prefix: str = "hky_test",
) -> None:
    """Insert one row into ``api_keys`` via raw SQL."""
    async with engine.begin() as conn:
        await conn.execute(
            text(
                "INSERT INTO api_keys "
                "(api_key_hash, key_prefix, owner_sub, created_at, status) "
                "VALUES (:hash, :prefix, :sub, :created, 'active')"
            ),
            {
                "hash": api_key_hash,
                "prefix": key_prefix,
                "sub": owner_sub,
                "created": datetime.now(UTC).isoformat(),
            },
        )


async def _seed_agent(engine, *, tenant_id: str, name: str) -> str:
    """Insert one row into ``agents`` via raw SQL. Returns the agent_id."""
    agent_id = str(uuid.uuid4())
    async with engine.begin() as conn:
        await conn.execute(
            text(
                "INSERT INTO agents "
                "(agent_id, tenant_id, name, description, "
                " agent_card_json, status, registered_at) "
                "VALUES (:id, :tenant, :name, :desc, :card, 'active', :reg)"
            ),
            {
                "id": agent_id,
                "tenant": tenant_id,
                "name": name,
                "desc": f"description for {name}",
                "card": json.dumps({"name": name}),
                "reg": datetime.now(UTC).isoformat(),
            },
        )
    return agent_id


async def _read_api_key_status(engine, api_key_hash: str) -> str | None:
    async with engine.connect() as conn:
        row = (
            await conn.execute(
                text("SELECT status FROM api_keys WHERE api_key_hash = :h"),
                {"h": api_key_hash},
            )
        ).first()
    return row[0] if row else None


async def _read_agent_statuses(engine, tenant_id: str) -> list[tuple[str, str]]:
    """Return ``[(name, status), ...]`` ordered by name for a tenant."""
    async with engine.connect() as conn:
        rows = (
            await conn.execute(
                text(
                    "SELECT name, status FROM agents "
                    "WHERE tenant_id = :t ORDER BY name"
                ),
                {"t": tenant_id},
            )
        ).all()
    return [(name, status) for (name, status) in rows]


# ---------------------------------------------------------------------------
# test_revoke_api_key_atomic — happy path
# ---------------------------------------------------------------------------


async def test_revoke_api_key_atomic(store, engine):
    """Revoking a key with N active agents commits all updates atomically.

    Verifies the design doc Testing Strategy claim:

      "Revoking a key with N agents results in all N agents
       status='deregistered' after a single transaction"

    This test exercises the SUCCESS half of the atomicity claim — the
    failure-rollback half is covered by
    ``test_revoke_api_key_atomic_failure_rolls_back`` below.

    The test seeds 3 active agents under one tenant and verifies that
    after ``revoke_api_key`` returns ``True``:

      1. The api_key row's status is ``'revoked'`` (the api_key UPDATE
         committed).
      2. All 3 agent rows have status ``'deregistered'`` (the agent
         UPDATE committed).
      3. The agents are returned in alphabetical order, confirming the
         row count and that no agents were dropped or added.

    Together, these assertions confirm that the multi-statement
    transaction (UPDATE api_keys + UPDATE agents) committed both
    statements as one logical unit.
    """
    api_key_hash = "tenant-revoke-happy"
    owner_sub = "user-revoke-happy"

    await _seed_api_key(engine, api_key_hash=api_key_hash, owner_sub=owner_sub)
    for i in range(3):
        await _seed_agent(engine, tenant_id=api_key_hash, name=f"agent-{i}")

    assert await _read_api_key_status(engine, api_key_hash) == "active"
    pre_agents = await _read_agent_statuses(engine, api_key_hash)
    assert pre_agents == [
        ("agent-0", "active"),
        ("agent-1", "active"),
        ("agent-2", "active"),
    ]

    result = await store.revoke_api_key(api_key_hash, owner_sub)
    assert result is True, (
        f"revoke_api_key should return True on success, got {result!r}"
    )

    post_status = await _read_api_key_status(engine, api_key_hash)
    assert post_status == "revoked", (
        f"api_key status should be 'revoked' after a successful revoke, "
        f"got {post_status!r}"
    )

    post_agents = await _read_agent_statuses(engine, api_key_hash)
    assert post_agents == [
        ("agent-0", "deregistered"),
        ("agent-1", "deregistered"),
        ("agent-2", "deregistered"),
    ], (
        f"all agents under the revoked tenant should be deregistered, "
        f"got {post_agents}"
    )


# ---------------------------------------------------------------------------
# test_revoke_api_key_atomic — failure rollback
# ---------------------------------------------------------------------------


async def test_revoke_api_key_atomic_failure_rolls_back(store, engine):
    """A failure during ``UPDATE agents`` rolls back the ``UPDATE api_keys`` too.

    Verifies the second half of the design doc Testing Strategy claim:

      "injecting a failure mid-loop rolls back the API key flip too"

    Mechanism:

      Install a one-shot SQLAlchemy ``before_cursor_execute`` listener
      on the engine. The listener inspects each SQL statement and
      raises ``RuntimeError`` if the statement is an ``UPDATE agents``
      (case-insensitive). Per the design doc pseudocode in §"Store
      ownership of sessions", ``revoke_api_key`` issues
      ``UPDATE api_keys`` first and ``UPDATE agents`` second; the
      listener therefore fires AFTER the api_keys flip has been queued
      in the transaction but BEFORE COMMIT. The injected error
      propagates out of ``session.execute``, causing the
      ``async with session.begin():`` context manager to roll back the
      entire transaction.

    The atomicity invariant is verified by re-reading the DB state
    AFTER the failed call: the api_key must STILL be ``'active'``
    (rolled back) and all agents must STILL be ``'active'`` (no
    partial deregistration).

    If this test fails with the api_key showing ``'revoked'``, the
    implementation has a bug: the two UPDATE statements are not in
    the same transaction (probably split across separate
    ``session.begin()`` blocks or auto-committing per statement).
    """
    api_key_hash = "tenant-revoke-fail"
    owner_sub = "user-revoke-fail"

    await _seed_api_key(engine, api_key_hash=api_key_hash, owner_sub=owner_sub)
    for i in range(3):
        await _seed_agent(engine, tenant_id=api_key_hash, name=f"agent-{i}")

    sentinel = "INJECTED ROLLBACK FAILURE"

    @event.listens_for(engine.sync_engine, "before_cursor_execute")
    def _intercept(conn, cursor, statement, parameters, context, executemany):
        normalized = " ".join(statement.split()).upper()
        if normalized.startswith("UPDATE AGENTS"):
            raise RuntimeError(sentinel)

    try:
        with pytest.raises(Exception, match=sentinel):
            await store.revoke_api_key(api_key_hash, owner_sub)
    finally:
        event.remove(engine.sync_engine, "before_cursor_execute", _intercept)

    post_status = await _read_api_key_status(engine, api_key_hash)
    assert post_status == "active", (
        f"api_key flip must have been rolled back when the agent UPDATE "
        f"failed; expected 'active', got {post_status!r}. This indicates "
        f"the UPDATE api_keys statement was not inside the same "
        f"transaction as the UPDATE agents statement (atomicity bug)."
    )

    post_agents = await _read_agent_statuses(engine, api_key_hash)
    expected = [
        ("agent-0", "active"),
        ("agent-1", "active"),
        ("agent-2", "active"),
    ]
    assert post_agents == expected, (
        f"agents must not have been deregistered when the transaction "
        f"failed; expected {expected}, got {post_agents}"
    )


# ---------------------------------------------------------------------------
# test_list_api_keys_no_n_plus_one
# ---------------------------------------------------------------------------


async def test_list_api_keys_no_n_plus_one(store, engine):
    """``list_api_keys`` emits exactly one SELECT against ``api_keys``.

    Verifies the design doc Testing Strategy claim:

      "Snapshot the SQL emitted by list_api_keys and assert it is
       exactly one query"

    Background: the legacy Redis implementation was N+1 — one
    ``SMEMBERS`` followed by ``HGETALL + SCARD`` per key. With 3 keys
    that's 1 + 3*2 = 7 round trips. The new SQL implementation MUST
    collapse this into a single ``SELECT`` with a ``LEFT JOIN`` and
    ``GROUP BY``, per the Operation Mapping table:

      "SELECT k.api_key_hash, k.key_prefix, k.created_at, k.status,
              COUNT(a.agent_id) AS agent_count
         FROM api_keys k
    LEFT JOIN agents a ON a.tenant_id = k.api_key_hash
                       AND a.status = 'active'
        WHERE k.owner_sub = ?
     GROUP BY k.api_key_hash"

    Test setup: 3 keys, each with 5 active agents. The legacy
    implementation would emit 7 statements; the new implementation
    must emit exactly 1.

    Mechanism: install a ``before_cursor_execute`` listener that
    captures every statement issued during the ``list_api_keys``
    call. After the call returns, count statements that look like
    ``SELECT ... api_keys ...`` (case-insensitive substring) — this
    filter excludes any internal SQLAlchemy bookkeeping statements
    (e.g., ``SELECT sqlite_version()`` on first connection) and
    focuses on user-visible queries against the api_keys table.

    The result is also sanity-checked: 3 entries returned, total
    ``agent_count`` across all entries equals 15. This guards
    against the implementation accidentally returning fewer keys
    or miscounting agents.
    """
    owner_sub = "user-no-n-plus-one"

    for i in range(3):
        api_key_hash = f"tenant-{i}"
        await _seed_api_key(
            engine, api_key_hash=api_key_hash, owner_sub=owner_sub
        )
        for j in range(5):
            await _seed_agent(
                engine, tenant_id=api_key_hash, name=f"agent-{i}-{j}"
            )

    captured: list[str] = []

    @event.listens_for(engine.sync_engine, "before_cursor_execute")
    def _capture(conn, cursor, statement, parameters, context, executemany):
        captured.append(statement)

    try:
        result = await store.list_api_keys(owner_sub)
    finally:
        event.remove(engine.sync_engine, "before_cursor_execute", _capture)

    api_key_selects = [
        s
        for s in captured
        if s.lstrip().upper().startswith("SELECT") and "api_keys" in s.lower()
    ]
    assert len(api_key_selects) == 1, (
        f"list_api_keys must emit exactly ONE SELECT against api_keys "
        f"(no N+1), got {len(api_key_selects)}. all captured statements:\n"
        + "\n".join(f"  {i + 1}: {s}" for i, s in enumerate(captured))
    )

    assert len(result) == 3, (
        f"list_api_keys should return 3 entries (one per seeded key), "
        f"got {len(result)}: {result}"
    )

    total_agents = sum(item["agent_count"] for item in result)
    assert total_agents == 15, (
        f"total agent_count across all returned keys should be 15 "
        f"(3 keys × 5 active agents each), got {total_agents}: {result}"
    )
