"""Tests for RegistryStore — SQL-backed agent + API key store.

Replaces the previous fakeredis-based test_registry_store.py. Uses
the conftest.py ``store`` and ``db_engine`` fixtures (populated by
Step 5 Phase B per design-docs/0000010-sqlite-store-migration/
design-doc.md §"Fixture stack").

## Test isolation strategy

The conftest fixture stack uses a session-scoped in-memory aiosqlite
engine that persists across tests within a single pytest session. To
prevent cross-test contamination without per-test cleanup, every
test generates a fresh ``owner_sub`` via ``_unique_owner()`` (UUID-
based) and queries data scoped to that owner. Two tests can never
see each other's api_keys, agents, or tasks because their scoping
identifiers are disjoint.

This is the **only** isolation mechanism in this file. Tests must
NEVER call ``list_active_agents(tenant_id=None)`` (the unfiltered
form) — that would return ALL active agents from ALL tests in the
session. The unfiltered form is documented in the design doc
Operation Mapping as "rare path; only used by tests" and is not
exercised here.

## Coverage map

  | Method                              | Test class                       |
  |-------------------------------------|----------------------------------|
  | create_api_key                      | TestCreateApiKey                 |
  | create_agent                        | TestCreateAgent                  |
  | get_agent                           | TestGetAgent                     |
  | list_active_agents                  | TestListActiveAgents             |
  | deregister_agent                    | TestDeregisterAgent              |
  | verify_agent_tenant                 | TestVerifyAgentTenant            |
  | list_api_keys                       | TestListApiKeys                  |
  | revoke_api_key                      | TestRevokeApiKey                 |
  | get_api_key_status                  | TestGetApiKeyStatus              |
  | is_api_key_active                   | TestIsApiKeyActive               |
  | is_key_owner                        | TestIsKeyOwner                   |
  | get_agent_name                      | TestGetAgentName                 |
  | list_deregistered_agents_with_tasks | TestListDeregisteredAgentsWithTasks |

## Design-doc-named tests

  - ``TestCreateAgent::test_rejects_unknown_tenant`` — verifies the FK
    enforcement on ``agents.tenant_id -> api_keys.api_key_hash``.
  - ``TestRevokeApiKey::test_atomic`` (happy path) and
    ``TestRevokeApiKey::test_atomic_failure_rolls_back`` (failure half)
    — together verify the multi-statement transaction atomicity.
  - ``TestListApiKeys::test_no_n_plus_one`` — verifies the
    LEFT JOIN + GROUP BY collapses N+1 into a single query.
"""

import hashlib
import json
import uuid
from datetime import UTC, datetime

import pytest
from sqlalchemy import event, text
from sqlalchemy.exc import IntegrityError


# ---------------------------------------------------------------------------
# Helpers
#
# All test data is scoped to a unique owner_sub per call so that tests
# sharing the session-scoped in-memory engine cannot contaminate each
# other. The helpers below are intentionally minimal: they only do the
# work that every test needs and avoid hiding meaningful setup behind
# layers of indirection.
# ---------------------------------------------------------------------------


def _unique_owner() -> str:
    """Generate a unique ``owner_sub`` for test isolation."""
    return f"auth0|test-{uuid.uuid4().hex[:12]}"


async def _make_owner_with_key(store) -> tuple[str, str, str]:
    """Create a fresh owner + api_key. Returns ``(api_key, api_key_hash, owner_sub)``."""
    owner = _unique_owner()
    api_key, api_key_hash, _created_at = await store.create_api_key(owner)
    return api_key, api_key_hash, owner


async def _seed_task_for_agent(db_engine, *, agent_id: str) -> None:
    """Insert one task into ``tasks`` with ``context_id = agent_id``.

    Used by ``TestListDeregisteredAgentsWithTasks`` to set up the
    "agent has at least one task" precondition. Goes through raw
    SQL rather than ``TaskStore`` because ``TaskStore`` is rewritten
    in Step 6 and is not yet in scope.
    """
    now = datetime.now(UTC).isoformat()
    async with db_engine.begin() as conn:
        await conn.execute(
            text(
                "INSERT INTO tasks "
                "(task_id, context_id, from_agent_id, to_agent_id, "
                " type, created_at, status_state, status_timestamp, task_json) "
                "VALUES (:tid, :ctx, :from_, :to, 'unicast', :now, "
                "        'submitted', :now, '{}')"
            ),
            {
                "tid": str(uuid.uuid4()),
                "ctx": agent_id,
                "from_": agent_id,
                "to": agent_id,
                "now": now,
            },
        )


# ---------------------------------------------------------------------------
# create_api_key
# ---------------------------------------------------------------------------


class TestCreateApiKey:
    """Tests for ``RegistryStore.create_api_key(owner_sub)``."""

    async def test_returns_three_tuple(self, store):
        """Returns ``(api_key, api_key_hash, created_at)``."""
        owner = _unique_owner()
        result = await store.create_api_key(owner)
        assert isinstance(result, tuple)
        assert len(result) == 3
        api_key, api_key_hash, created_at = result
        assert isinstance(api_key, str)
        assert isinstance(api_key_hash, str)
        assert isinstance(created_at, str)
        # created_at must be ISO-8601 parseable
        datetime.fromisoformat(created_at)

    async def test_each_call_returns_unique_key_and_hash(self, store):
        """Successive calls return distinct api_keys with matching hashes."""
        owner = _unique_owner()
        a_key, a_hash, _ = await store.create_api_key(owner)
        b_key, b_hash, _ = await store.create_api_key(owner)
        assert a_key != b_key
        assert a_hash != b_hash
        assert a_hash == hashlib.sha256(a_key.encode()).hexdigest()
        assert b_hash == hashlib.sha256(b_key.encode()).hexdigest()

    async def test_persists_and_listable(self, store):
        """A newly created key is visible via ``list_api_keys``."""
        owner = _unique_owner()
        _, api_key_hash, _ = await store.create_api_key(owner)
        keys = await store.list_api_keys(owner)
        assert len(keys) == 1
        assert keys[0]["tenant_id"] == api_key_hash
        assert keys[0]["status"] == "active"


# ---------------------------------------------------------------------------
# create_agent
# ---------------------------------------------------------------------------


class TestCreateAgent:
    """Tests for ``RegistryStore.create_agent``."""

    async def test_returns_required_fields(self, store):
        """Result contains ``agent_id``, ``api_key``, ``name``, ``registered_at``."""
        api_key, _, _ = await _make_owner_with_key(store)
        result = await store.create_agent(
            "Test Agent", "A test agent", None, api_key=api_key
        )
        assert "agent_id" in result
        assert "api_key" in result
        assert "name" in result
        assert "registered_at" in result
        assert result["api_key"] == api_key
        assert result["name"] == "Test Agent"
        # registered_at must be ISO-8601 parseable
        datetime.fromisoformat(result["registered_at"])

    async def test_with_skills_persists_in_agent_card(self, store):
        """Skills passed to create_agent appear in ``agent_card_json``."""
        api_key, _, _ = await _make_owner_with_key(store)
        skills = [
            {
                "id": "py",
                "name": "Python",
                "description": "writes Python",
                "tags": ["lang"],
            }
        ]
        result = await store.create_agent(
            "Skilled", "desc", skills, api_key=api_key
        )
        agent = await store.get_agent(result["agent_id"])
        assert agent is not None
        card = json.loads(agent["agent_card_json"])
        assert card.get("skills") and card["skills"][0]["id"] == "py"

    async def test_unique_agent_ids(self, store):
        """Multiple agents under the same key get distinct ``agent_id``s."""
        api_key, _, _ = await _make_owner_with_key(store)
        a = await store.create_agent("a", "d", None, api_key=api_key)
        b = await store.create_agent("b", "d", None, api_key=api_key)
        assert a["agent_id"] != b["agent_id"]

    async def test_rejects_unknown_tenant(self, store):
        """Creating an agent with an api_key whose hash is not in api_keys raises ``IntegrityError``.

        DESIGN-DOC-NAMED test: verifies the FK constraint
        ``agents.tenant_id -> api_keys.api_key_hash`` enforces
        tenant existence at INSERT time. Without the FK + PRAGMA
        ``foreign_keys=ON``, an orphan agent could be inserted with
        no parent api_key row, breaking referential integrity.

        The test uses a fabricated api_key (``hky_`` + 32 ``f``s) whose
        SHA-256 hash is statistically guaranteed not to collide with
        any real api_key in the test session.
        """
        fake_api_key = "hky_" + "f" * 32
        with pytest.raises(IntegrityError):
            await store.create_agent(
                "Orphan", "desc", None, api_key=fake_api_key
            )


# ---------------------------------------------------------------------------
# get_agent
# ---------------------------------------------------------------------------


class TestGetAgent:
    """Tests for ``RegistryStore.get_agent``."""

    async def test_returns_existing_agent(self, store):
        api_key, _, _ = await _make_owner_with_key(store)
        created = await store.create_agent(
            "Test Agent", "A test agent", None, api_key=api_key
        )
        agent = await store.get_agent(created["agent_id"])
        assert agent is not None
        assert agent["agent_id"] == created["agent_id"]
        assert agent["name"] == "Test Agent"
        assert agent["description"] == "A test agent"
        assert agent["status"] == "active"

    async def test_returns_none_for_missing(self, store):
        agent = await store.get_agent("00000000-0000-4000-8000-000000000000")
        assert agent is None

    async def test_returns_record_for_deregistered_agent(self, store):
        """Deregistered agents still have a get_agent record (soft delete)."""
        api_key, _, _ = await _make_owner_with_key(store)
        created = await store.create_agent("a", "d", None, api_key=api_key)
        await store.deregister_agent(created["agent_id"])
        agent = await store.get_agent(created["agent_id"])
        assert agent is not None
        assert agent["status"] == "deregistered"


# ---------------------------------------------------------------------------
# list_active_agents
# ---------------------------------------------------------------------------


class TestListActiveAgents:
    """Tests for ``RegistryStore.list_active_agents(tenant_id=...)``.

    Always passes ``tenant_id`` so that tests are scoped to their own
    api_key and immune to cross-test contamination from the shared
    in-memory DB.
    """

    async def test_empty_for_new_tenant(self, store):
        _, hash_a, _ = await _make_owner_with_key(store)
        agents = await store.list_active_agents(tenant_id=hash_a)
        assert agents == []

    async def test_returns_all_active_for_tenant(self, store):
        api_key, hash_a, _ = await _make_owner_with_key(store)
        await store.create_agent("a1", "d", None, api_key=api_key)
        await store.create_agent("a2", "d", None, api_key=api_key)
        agents = await store.list_active_agents(tenant_id=hash_a)
        assert len(agents) == 2
        assert {a["name"] for a in agents} == {"a1", "a2"}

    async def test_excludes_deregistered_agents(self, store):
        api_key, hash_a, _ = await _make_owner_with_key(store)
        a = await store.create_agent("a", "d", None, api_key=api_key)
        b = await store.create_agent("b", "d", None, api_key=api_key)
        await store.deregister_agent(a["agent_id"])
        agents = await store.list_active_agents(tenant_id=hash_a)
        assert len(agents) == 1
        assert agents[0]["agent_id"] == b["agent_id"]

    async def test_filters_by_tenant_id_isolates_tenants(self, store):
        """Cross-tenant isolation: agents in tenant A do not appear in tenant B."""
        api_key_a, hash_a, _ = await _make_owner_with_key(store)
        api_key_b, hash_b, _ = await _make_owner_with_key(store)
        await store.create_agent("a", "d", None, api_key=api_key_a)
        await store.create_agent("b", "d", None, api_key=api_key_b)

        a_list = await store.list_active_agents(tenant_id=hash_a)
        b_list = await store.list_active_agents(tenant_id=hash_b)
        assert {x["name"] for x in a_list} == {"a"}
        assert {x["name"] for x in b_list} == {"b"}


# ---------------------------------------------------------------------------
# deregister_agent
# ---------------------------------------------------------------------------


class TestDeregisterAgent:
    """Tests for ``RegistryStore.deregister_agent``."""

    async def test_sets_status_and_timestamp(self, store):
        api_key, _, _ = await _make_owner_with_key(store)
        created = await store.create_agent("a", "d", None, api_key=api_key)
        result = await store.deregister_agent(created["agent_id"])
        assert result is True

        agent = await store.get_agent(created["agent_id"])
        assert agent["status"] == "deregistered"
        assert agent.get("deregistered_at"), "deregistered_at must be set"
        datetime.fromisoformat(agent["deregistered_at"])  # parses

    async def test_returns_false_for_already_deregistered(self, store):
        """Idempotency: a second deregister call returns False (no-op).

        The new SQL implementation uses ``UPDATE ... WHERE status='active'``,
        so the second call's rowcount is zero and the method returns False.
        This matches the design doc's `single-statement update returning
        affected row count` semantics.
        """
        api_key, _, _ = await _make_owner_with_key(store)
        created = await store.create_agent("a", "d", None, api_key=api_key)
        first = await store.deregister_agent(created["agent_id"])
        second = await store.deregister_agent(created["agent_id"])
        assert first is True
        assert second is False

    async def test_returns_false_for_missing_agent(self, store):
        result = await store.deregister_agent(
            "00000000-0000-4000-8000-000000000000"
        )
        assert result is False


# ---------------------------------------------------------------------------
# verify_agent_tenant
# ---------------------------------------------------------------------------


class TestVerifyAgentTenant:
    """Tests for ``RegistryStore.verify_agent_tenant``."""

    async def test_matching_tenant_returns_true(self, store):
        api_key, hash_a, _ = await _make_owner_with_key(store)
        agent = await store.create_agent("a", "d", None, api_key=api_key)
        assert await store.verify_agent_tenant(agent["agent_id"], hash_a) is True

    async def test_wrong_tenant_returns_false(self, store):
        api_key_a, _, _ = await _make_owner_with_key(store)
        _, hash_b, _ = await _make_owner_with_key(store)
        agent = await store.create_agent("a", "d", None, api_key=api_key_a)
        assert await store.verify_agent_tenant(agent["agent_id"], hash_b) is False

    async def test_nonexistent_agent_returns_false(self, store):
        _, hash_a, _ = await _make_owner_with_key(store)
        assert (
            await store.verify_agent_tenant(
                "00000000-0000-4000-8000-000000000000", hash_a
            )
            is False
        )

    async def test_deregistered_agent_still_verifiable(self, store):
        """A deregistered agent's row still has tenant_id; verify must still work."""
        api_key, hash_a, _ = await _make_owner_with_key(store)
        agent = await store.create_agent("a", "d", None, api_key=api_key)
        await store.deregister_agent(agent["agent_id"])
        assert await store.verify_agent_tenant(agent["agent_id"], hash_a) is True


# ---------------------------------------------------------------------------
# list_api_keys
# ---------------------------------------------------------------------------


class TestListApiKeys:
    """Tests for ``RegistryStore.list_api_keys(owner_sub)``."""

    async def test_returns_empty_for_new_owner(self, store):
        owner = _unique_owner()
        keys = await store.list_api_keys(owner)
        assert keys == []

    async def test_returns_zero_count_when_no_agents(self, store):
        owner = _unique_owner()
        await store.create_api_key(owner)
        keys = await store.list_api_keys(owner)
        assert len(keys) == 1
        assert keys[0]["agent_count"] == 0

    async def test_excludes_deregistered_agents_from_count(self, store):
        """``agent_count`` only counts agents with status='active'."""
        api_key, _, owner = await _make_owner_with_key(store)
        a = await store.create_agent("a", "d", None, api_key=api_key)
        await store.create_agent("b", "d", None, api_key=api_key)
        await store.deregister_agent(a["agent_id"])

        keys = await store.list_api_keys(owner)
        assert len(keys) == 1
        assert keys[0]["agent_count"] == 1, (
            f"only the still-active agent should be counted, got {keys}"
        )

    async def test_no_n_plus_one(self, store, db_engine):
        """``list_api_keys`` emits exactly one SELECT against ``api_keys``.

        DESIGN-DOC-NAMED test (Testing Strategy):

          "Snapshot the SQL emitted by list_api_keys and assert it is
           exactly one query"

        Background: the legacy Redis implementation was N+1 — one
        ``SMEMBERS`` followed by ``HGETALL + SCARD`` per key. With 3
        keys that's 1 + 3*2 = 7 round trips. The new SQL impl MUST
        collapse this into a single ``SELECT`` with ``LEFT JOIN`` +
        ``GROUP BY``.

        Mechanism: install a ``before_cursor_execute`` listener that
        captures every statement during the call, then filter to
        ``SELECT ... api_keys ...`` (case-insensitive substring).
        The substring filter excludes any internal SQLAlchemy
        bookkeeping statements like ``SELECT sqlite_version()`` that
        fire on first connection.
        """
        owner = _unique_owner()
        for i in range(3):
            api_key, _, _ = await store.create_api_key(owner)
            for j in range(5):
                await store.create_agent(
                    f"a{i}-{j}", "d", None, api_key=api_key
                )

        captured: list[str] = []

        @event.listens_for(db_engine.sync_engine, "before_cursor_execute")
        def _capture(conn, cursor, statement, parameters, context, executemany):
            captured.append(statement)

        try:
            result = await store.list_api_keys(owner)
        finally:
            event.remove(
                db_engine.sync_engine, "before_cursor_execute", _capture
            )

        api_key_selects = [
            s
            for s in captured
            if s.lstrip().upper().startswith("SELECT")
            and "api_keys" in s.lower()
        ]
        assert len(api_key_selects) == 1, (
            f"list_api_keys must emit exactly ONE SELECT against api_keys "
            f"(no N+1), got {len(api_key_selects)}. all captured "
            f"statements:\n"
            + "\n".join(f"  {i + 1}: {s}" for i, s in enumerate(captured))
        )

        assert len(result) == 3, (
            f"list_api_keys should return 3 entries, got {len(result)}"
        )
        total = sum(item["agent_count"] for item in result)
        assert total == 15, (
            f"total agent_count across all keys should be 15 "
            f"(3 keys × 5 agents), got {total}"
        )


# ---------------------------------------------------------------------------
# revoke_api_key
# ---------------------------------------------------------------------------


class TestRevokeApiKey:
    """Tests for ``RegistryStore.revoke_api_key``."""

    async def test_atomic(self, store):
        """Revoking with N agents commits all updates atomically.

        DESIGN-DOC-NAMED test (Testing Strategy):

          "Revoking a key with N agents results in all N agents
           status='deregistered' after a single transaction"

        Happy-path half of the atomicity claim. See
        ``test_atomic_failure_rolls_back`` below for the rollback half.
        """
        api_key, api_key_hash, owner = await _make_owner_with_key(store)
        for i in range(3):
            await store.create_agent(f"a{i}", "d", None, api_key=api_key)

        result = await store.revoke_api_key(api_key_hash, owner)
        assert result is True

        assert await store.get_api_key_status(api_key_hash) == "revoked"
        active = await store.list_active_agents(tenant_id=api_key_hash)
        assert active == [], (
            f"all agents under the revoked tenant should be deregistered, "
            f"got {active}"
        )

    async def test_atomic_failure_rolls_back(self, store, db_engine):
        """A failure during ``UPDATE agents`` rolls back ``UPDATE api_keys`` too.

        Verifies the second half of the atomicity claim:

          "injecting a failure mid-loop rolls back the API key flip too"

        Mechanism: install a one-shot SQLAlchemy
        ``before_cursor_execute`` listener that raises ``RuntimeError``
        on any ``UPDATE agents`` statement. Per the design doc
        pseudocode in §"Store ownership of sessions",
        ``revoke_api_key`` issues ``UPDATE api_keys`` first and
        ``UPDATE agents`` second; the listener fires AFTER the
        api_keys flip is queued but BEFORE the transaction commits.
        The injected error must cause the entire ``session.begin()``
        block to roll back, leaving the api_keys flip uncommitted.

        If this test fails with ``api_key.status == 'revoked'`` after
        the failed call, the implementation has an atomicity bug —
        the two UPDATE statements are NOT inside the same transaction.
        """
        api_key, api_key_hash, owner = await _make_owner_with_key(store)
        for i in range(3):
            await store.create_agent(f"a{i}", "d", None, api_key=api_key)

        sentinel = "INJECTED ROLLBACK FAILURE"

        @event.listens_for(db_engine.sync_engine, "before_cursor_execute")
        def _intercept(conn, cursor, statement, parameters, context, executemany):
            normalized = " ".join(statement.split()).upper()
            if normalized.startswith("UPDATE AGENTS"):
                raise RuntimeError(sentinel)

        try:
            with pytest.raises(Exception, match=sentinel):
                await store.revoke_api_key(api_key_hash, owner)
        finally:
            event.remove(
                db_engine.sync_engine, "before_cursor_execute", _intercept
            )

        post_status = await store.get_api_key_status(api_key_hash)
        assert post_status == "active", (
            f"api_key flip must have been rolled back when the agent "
            f"UPDATE failed; expected 'active', got {post_status!r}. "
            f"This indicates the UPDATE api_keys statement was not "
            f"inside the same transaction as the UPDATE agents "
            f"statement (atomicity bug)."
        )
        active = await store.list_active_agents(tenant_id=api_key_hash)
        assert len(active) == 3, (
            f"agents must not have been deregistered when the "
            f"transaction failed; expected 3 active, got {len(active)}"
        )

    async def test_returns_false_for_non_owner(self, store):
        """A non-owner cannot revoke another owner's key."""
        _, api_key_hash, _ = await _make_owner_with_key(store)
        other_owner = _unique_owner()
        result = await store.revoke_api_key(api_key_hash, other_owner)
        assert result is False
        assert await store.get_api_key_status(api_key_hash) == "active"

    async def test_idempotent_on_already_revoked(self, store):
        """A second revoke call on an already-revoked key still returns True.

        revoke_api_key returns True iff the key ends the call in revoked
        state (whether or not it was already revoked). False is reserved
        for owner-mismatch / missing-key, which are authorization
        failures, not state-change failures.
        """
        _, api_key_hash, owner = await _make_owner_with_key(store)
        first = await store.revoke_api_key(api_key_hash, owner)
        second = await store.revoke_api_key(api_key_hash, owner)
        assert first is True
        assert second is True


# ---------------------------------------------------------------------------
# get_api_key_status
# ---------------------------------------------------------------------------


class TestGetApiKeyStatus:
    """Tests for ``RegistryStore.get_api_key_status``."""

    async def test_returns_active_then_revoked(self, store):
        _, api_key_hash, owner = await _make_owner_with_key(store)
        assert await store.get_api_key_status(api_key_hash) == "active"
        await store.revoke_api_key(api_key_hash, owner)
        assert await store.get_api_key_status(api_key_hash) == "revoked"

    async def test_returns_none_for_missing(self, store):
        assert await store.get_api_key_status("missing-hash-xyz") is None


# ---------------------------------------------------------------------------
# is_api_key_active
# ---------------------------------------------------------------------------


class TestIsApiKeyActive:
    """Tests for ``RegistryStore.is_api_key_active`` (new leak-fixing method)."""

    async def test_true_for_active_false_after_revoke(self, store):
        _, api_key_hash, owner = await _make_owner_with_key(store)
        assert await store.is_api_key_active(api_key_hash) is True
        await store.revoke_api_key(api_key_hash, owner)
        assert await store.is_api_key_active(api_key_hash) is False

    async def test_false_for_missing(self, store):
        assert await store.is_api_key_active("missing-hash-xyz") is False


# ---------------------------------------------------------------------------
# is_key_owner
# ---------------------------------------------------------------------------


class TestIsKeyOwner:
    """Tests for ``RegistryStore.is_key_owner`` (new leak-fixing method)."""

    async def test_true_for_owner_false_for_non_owner(self, store):
        _, api_key_hash, owner = await _make_owner_with_key(store)
        assert await store.is_key_owner(api_key_hash, owner) is True
        assert await store.is_key_owner(api_key_hash, _unique_owner()) is False

    async def test_false_for_missing_key(self, store):
        assert await store.is_key_owner("missing-hash", _unique_owner()) is False


# ---------------------------------------------------------------------------
# get_agent_name
# ---------------------------------------------------------------------------


class TestGetAgentName:
    """Tests for ``RegistryStore.get_agent_name`` (new leak-fixing method)."""

    async def test_returns_name_for_existing(self, store):
        api_key, _, _ = await _make_owner_with_key(store)
        agent = await store.create_agent("My Name", "d", None, api_key=api_key)
        assert await store.get_agent_name(agent["agent_id"]) == "My Name"

    async def test_returns_empty_string_for_missing(self, store):
        """Per the design doc contract: returns ``''`` (NOT ``None``) for missing.

        The contract matches today's ``or ""`` fallback in
        ``webui_api.py`` so call sites can drop the fallback once they
        switch to this method.
        """
        result = await store.get_agent_name(
            "00000000-0000-4000-8000-000000000000"
        )
        assert result == "", (
            f"get_agent_name must return '' for missing agents (not None), "
            f"got {result!r}"
        )


# ---------------------------------------------------------------------------
# list_deregistered_agents_with_tasks
# ---------------------------------------------------------------------------


class TestListDeregisteredAgentsWithTasks:
    """Tests for ``RegistryStore.list_deregistered_agents_with_tasks``.

    Per the design doc Operation Mapping table:

      SELECT a.agent_id, a.name, a.description, a.registered_at
        FROM agents a
       WHERE a.tenant_id = ?
         AND a.status = 'deregistered'
         AND EXISTS (SELECT 1 FROM tasks t WHERE t.context_id = a.agent_id LIMIT 1)

    Replaces the legacy ``_redis.scan(match='agent:*') + HGETALL filter``
    pattern in ``webui_api.py``.
    """

    async def test_excludes_active_agents(self, store, db_engine):
        """Active agents (with or without tasks) are excluded from the result."""
        api_key, api_key_hash, _ = await _make_owner_with_key(store)
        active = await store.create_agent(
            "active", "d", None, api_key=api_key
        )
        await _seed_task_for_agent(db_engine, agent_id=active["agent_id"])

        result = await store.list_deregistered_agents_with_tasks(api_key_hash)
        result_ids = {r["agent_id"] for r in result}
        assert active["agent_id"] not in result_ids

    async def test_excludes_deregistered_without_tasks(self, store):
        """Deregistered agents with no tasks are excluded (the ``EXISTS`` filter)."""
        api_key, api_key_hash, _ = await _make_owner_with_key(store)
        agent = await store.create_agent("a", "d", None, api_key=api_key)
        await store.deregister_agent(agent["agent_id"])

        result = await store.list_deregistered_agents_with_tasks(api_key_hash)
        assert result == []

    async def test_includes_deregistered_with_tasks(self, store, db_engine):
        """The matching case: deregistered AND has at least one task."""
        api_key, api_key_hash, _ = await _make_owner_with_key(store)
        agent = await store.create_agent("a", "d", None, api_key=api_key)
        await _seed_task_for_agent(db_engine, agent_id=agent["agent_id"])
        await store.deregister_agent(agent["agent_id"])

        result = await store.list_deregistered_agents_with_tasks(api_key_hash)
        assert len(result) == 1
        assert result[0]["agent_id"] == agent["agent_id"]
        assert result[0]["name"] == "a"
