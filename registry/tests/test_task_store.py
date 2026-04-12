"""Tests for the SQL-backed TaskStore.

Covers the seven public methods of ``TaskStore`` listed in the design
doc (design-docs/0000010-sqlite-store-migration/design-doc.md §"Store
ownership of sessions", Operation Mapping for the ``tasks`` table, and
the Step 6 checklist in Implementation Steps):

  | Method             | Responsibility                                     |
  |--------------------|----------------------------------------------------|
  | ``save``           | UPSERT. Preserves ``created_at`` on re-save.       |
  | ``get``            | SELECT task_json, parse with ``Task.model_validate_json``. |
  | ``delete``         | DELETE FROM tasks WHERE task_id=?.                 |
  | ``list``           | SELECT by ``context_id`` ordered by                |
  |                    | ``status_timestamp DESC``.                         |
  | ``list_by_sender`` | SELECT by ``from_agent_id`` ordered by             |
  |                    | ``status_timestamp DESC``. NEW.                    |
  | ``get_endpoints``  | SELECT ``(from_agent_id, to_agent_id)`` by id. NEW.|
  | ``get_created_at`` | SELECT ``created_at`` by id. NEW.                  |

Foreign key setup
-----------------

The ``tasks.context_id`` column has a ``REFERENCES agents(agent_id)``
constraint with ``ON DELETE RESTRICT`` (see ``db/models.py`` Task). Any
test that wants to save a task must first create a real agent row so
the FK is satisfied. Tests that only need one tenant use the
``_seed_agent`` helper; tests that need tenant isolation (e.g.,
``list`` filters by context_id) use ``_seed_two_agents``. Both helpers
go through the conftest ``store`` fixture (``RegistryStore``) rather
than issuing raw INSERTs, so the setup mirrors the production path
exactly — if ``create_agent`` ever stops adding a required column, these
tests break loudly at seed time instead of at FK resolution.

Fixture layout
--------------

* ``store`` — from conftest, a ``RegistryStore`` bound to the shared
  function-scoped ``db_sessionmaker``. Used exclusively for seeding
  tenants + agents in test setup.
* ``task_store`` — the subject under test. The Programmer adds this to
  ``conftest.py`` in Step 6 Phase B as
  ``TaskStore(db_sessionmaker)`` bound to the same sessionmaker as
  ``store``, so both stores share one in-memory SQLite DB per test.

Until Phase B lands the ``task_store`` fixture, this file will fail to
collect — that's the standard TDD-red state we've been running in all
of Step 3-5.
"""

import uuid
from datetime import UTC, datetime, timedelta

from a2a.types import (
    Artifact,
    Part,
    Task,
    TaskState,
    TaskStatus,
    TextPart,
)
from sqlalchemy import select

from hikyaku_registry.db.models import Task as TaskModel
from hikyaku_registry.task_store import TaskStore


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _ts(dt: datetime) -> str:
    """ISO 8601 timestamp string for ``TaskStatus.timestamp``."""
    return dt.isoformat()


def _make_task(
    *,
    task_id: str | None = None,
    context_id: str,
    from_agent_id: str = "sender-default",
    to_agent_id: str = "recipient-default",
    msg_type: str = "unicast",
    state: TaskState = TaskState.input_required,
    timestamp: str | None = None,
    text: str = "hello",
    extra_metadata: dict | None = None,
    origin_task_id: str | None = None,
) -> Task:
    """Construct an A2A ``Task`` with Hikyaku routing metadata.

    Mirrors the production path in ``executor.py``: routing fields
    (``fromAgentId``, ``toAgentId``, ``type``) live in ``task.metadata``
    and drive what the store writes to the indexed columns. The
    ``context_id`` keyword is required because every test in this file
    needs to scope tasks to a specific agent (FK requirement).

    ``origin_task_id`` (when non-None) is injected into
    ``metadata["originTaskId"]`` — ``TaskStore.save`` reads that key to
    populate the dedicated ``tasks.origin_task_id`` column. Leaving the
    kwarg at ``None`` matches the unicast-send path, which writes NULL.
    """
    if task_id is None:
        task_id = str(uuid.uuid4())
    if timestamp is None:
        timestamp = _ts(datetime.now(UTC))

    metadata = {
        "fromAgentId": from_agent_id,
        "toAgentId": to_agent_id,
        "type": msg_type,
    }
    if origin_task_id is not None:
        metadata["originTaskId"] = origin_task_id
    if extra_metadata:
        metadata.update(extra_metadata)

    return Task(
        id=task_id,
        context_id=context_id,
        status=TaskStatus(state=state, timestamp=timestamp),
        artifacts=[
            Artifact(
                artifact_id=str(uuid.uuid4()),
                name="message",
                parts=[Part(root=TextPart(text=text))],
            )
        ],
        metadata=metadata,
    )


async def _seed_agent(store, *, owner_sub: str | None = None) -> str:
    """Create a tenant + one agent, return the agent_id.

    Used as the ``context_id`` for tasks. Goes through the public
    ``RegistryStore`` API so the FK target row is created exactly the
    way production code creates it.
    """
    if owner_sub is None:
        owner_sub = f"auth0|task-test-{uuid.uuid4().hex[:12]}"
    api_key, _api_key_hash, _ = await store.create_api_key(owner_sub)
    result = await store.create_agent(
        "Test Agent",
        "agent for task_store tests",
        skills=None,
        api_key=api_key,
    )
    return result["agent_id"]


async def _seed_two_agents(store) -> tuple[str, str]:
    """Create two tenants, one agent per tenant, return both agent_ids.

    Two SEPARATE owners + api_keys → two SEPARATE agents. Tests that
    care about tenant/context isolation (e.g., ``list`` filtering by
    ``context_id``) use this.
    """
    a = await _seed_agent(store)
    b = await _seed_agent(store)
    return a, b


async def _seed_tenant_with_agents(store, n: int = 1) -> tuple[str, list[str]]:
    """Create one tenant with ``n`` agents; return ``(tenant_id, agent_ids)``.

    Used by ``list_timeline`` tests, which filter by ``tenant_id`` (the
    ``api_key_hash``). Seeding via the public ``RegistryStore`` API
    keeps the fixture path identical to production — if
    ``create_api_key`` or ``create_agent`` ever adds a required field,
    these tests break loudly at seed time.
    """
    owner_sub = f"auth0|timeline-test-{uuid.uuid4().hex[:12]}"
    api_key, tenant_id, _ = await store.create_api_key(owner_sub)
    agent_ids: list[str] = []
    for i in range(n):
        result = await store.create_agent(
            name=f"Timeline Agent {i}",
            description=f"agent {i} for list_timeline tests",
            skills=None,
            api_key=api_key,
        )
        agent_ids.append(result["agent_id"])
    return tenant_id, agent_ids


async def _read_origin_task_id_column(
    task_store: TaskStore, task_id: str
) -> str | None:
    """Return the RAW ``tasks.origin_task_id`` column for a task_id.

    Bypasses ``TaskStore.get`` (which materializes a Task from the
    ``task_json`` blob) to verify the dedicated column was written by
    ``save``. The column — not the blob — is the one queried by
    ``list_timeline`` and surfaced to the client, so it MUST be
    populated even when ``task.metadata["originTaskId"]`` round-trips
    through the JSON payload.
    """
    async with task_store._sessionmaker() as session:
        result = await session.execute(
            select(TaskModel.origin_task_id).where(TaskModel.task_id == task_id)
        )
        row = result.first()
    return row[0] if row else None


# ---------------------------------------------------------------------------
# save + get
# ---------------------------------------------------------------------------


class TestSaveAndGet:
    """save / get happy paths, UPSERT semantics, created_at preservation."""

    async def test_save_new_task(self, task_store, store):
        """A fresh ``save`` is retrievable via ``get`` with identical content."""
        agent_id = await _seed_agent(store)
        task = _make_task(
            task_id="task-001",
            context_id=agent_id,
            text="hello world",
        )

        await task_store.save(task)

        retrieved = await task_store.get("task-001")
        assert retrieved is not None, "get should return the just-saved task"
        assert retrieved.id == "task-001"
        assert retrieved.context_id == agent_id
        assert retrieved.status.state == TaskState.input_required
        assert isinstance(retrieved, Task)

    async def test_save_updates_existing_task(self, task_store, store):
        """Re-saving the same task_id updates status_state and task_json (UPSERT)."""
        agent_id = await _seed_agent(store)
        task = _make_task(
            task_id="task-upsert",
            context_id=agent_id,
            state=TaskState.input_required,
            text="original",
        )
        await task_store.save(task)

        task.status = TaskStatus(
            state=TaskState.completed,
            timestamp=_ts(datetime.now(UTC)),
        )
        task.artifacts[0].parts[0].root.text = "updated"
        await task_store.save(task)

        retrieved = await task_store.get("task-upsert")
        assert retrieved is not None
        assert retrieved.status.state == TaskState.completed, (
            "UPSERT must overwrite status_state with the new value"
        )
        assert retrieved.artifacts[0].parts[0].root.text == "updated", (
            "UPSERT must overwrite task_json (artifact text changed)"
        )

    async def test_save_preserves_created_at_across_updates(self, task_store, store):
        """UPSERT must NOT touch ``created_at`` — it's set once at first save.

        ``INSERT ... ON CONFLICT DO UPDATE`` deliberately omits
        ``created_at`` from the update clause. If this test fails,
        ``created_at`` is probably in the SET clause by accident.
        """
        agent_id = await _seed_agent(store)
        task = _make_task(
            task_id="task-preserve",
            context_id=agent_id,
            state=TaskState.input_required,
        )
        await task_store.save(task)

        created_at_first = await task_store.get_created_at("task-preserve")
        assert created_at_first is not None

        task.status = TaskStatus(
            state=TaskState.completed,
            timestamp=_ts(datetime.now(UTC) + timedelta(hours=1)),
        )
        await task_store.save(task)

        created_at_second = await task_store.get_created_at("task-preserve")
        assert created_at_second == created_at_first, (
            f"created_at must be preserved across save() calls; "
            f"expected {created_at_first!r}, got {created_at_second!r}"
        )

    async def test_get_returns_none_for_missing_task(self, task_store):
        """``get`` on an unknown task_id returns ``None``, not an exception."""
        result = await task_store.get("definitely-not-a-real-task-id")
        assert result is None


# ---------------------------------------------------------------------------
# delete
# ---------------------------------------------------------------------------


class TestDelete:
    """delete happy path + noop on missing rows."""

    async def test_delete_removes_task(self, task_store, store):
        """After ``delete``, ``get`` returns ``None`` for the same id."""
        agent_id = await _seed_agent(store)
        task = _make_task(task_id="task-del", context_id=agent_id)
        await task_store.save(task)

        assert await task_store.get("task-del") is not None

        await task_store.delete("task-del")

        assert await task_store.get("task-del") is None, (
            "get should return None after delete"
        )

    async def test_delete_nonexistent_is_noop(self, task_store):
        """``delete`` on a missing task_id must not raise."""
        await task_store.delete("never-existed")


# ---------------------------------------------------------------------------
# list (by context_id)
# ---------------------------------------------------------------------------


class TestList:
    """list(context_id) ordering, filtering, empty cases."""

    async def test_list_returns_tasks_in_desc_status_timestamp_order(
        self, task_store, store
    ):
        """Tasks come back in descending ``status_timestamp`` order."""
        agent_id = await _seed_agent(store)
        base = datetime(2026, 4, 1, 10, 0, 0, tzinfo=UTC)
        for i in range(3):
            await task_store.save(
                _make_task(
                    task_id=f"task-order-{i}",
                    context_id=agent_id,
                    timestamp=_ts(base + timedelta(minutes=i)),
                )
            )

        tasks = await task_store.list(agent_id)
        ids = [t.id for t in tasks]
        assert ids == ["task-order-2", "task-order-1", "task-order-0"], (
            f"list must return tasks DESC by status_timestamp; got {ids}"
        )

    async def test_list_filters_by_context_id(self, task_store, store):
        """Tasks under a different context_id must not appear in the result."""
        agent_a, agent_b = await _seed_two_agents(store)
        for i in range(2):
            await task_store.save(_make_task(task_id=f"a-{i}", context_id=agent_a))
            await task_store.save(_make_task(task_id=f"b-{i}", context_id=agent_b))

        a_tasks = await task_store.list(agent_a)
        b_tasks = await task_store.list(agent_b)

        assert {t.id for t in a_tasks} == {"a-0", "a-1"}
        assert {t.id for t in b_tasks} == {"b-0", "b-1"}

    async def test_list_empty_returns_empty_list(self, task_store, store):
        """An agent with no tasks yields an empty list (not ``None``)."""
        agent_id = await _seed_agent(store)
        tasks = await task_store.list(agent_id)
        assert tasks == []


# ---------------------------------------------------------------------------
# list_by_sender
# ---------------------------------------------------------------------------


class TestListBySender:
    """list_by_sender ordering + sender isolation."""

    async def test_list_by_sender_returns_tasks_in_desc_order(self, task_store, store):
        """``list_by_sender`` is DESC by status_timestamp, same as ``list``."""
        agent_id = await _seed_agent(store)
        base = datetime(2026, 4, 1, 12, 0, 0, tzinfo=UTC)
        for i in range(3):
            await task_store.save(
                _make_task(
                    task_id=f"s-{i}",
                    context_id=agent_id,
                    from_agent_id="sender-desc",
                    timestamp=_ts(base + timedelta(minutes=i)),
                )
            )

        tasks = await task_store.list_by_sender("sender-desc")
        ids = [t.id for t in tasks]
        assert ids == ["s-2", "s-1", "s-0"], (
            f"list_by_sender must return tasks DESC by status_timestamp; got {ids}"
        )

    async def test_list_by_sender_filters_by_sender(self, task_store, store):
        """Tasks from a different sender are excluded from the result."""
        agent_id = await _seed_agent(store)
        await task_store.save(
            _make_task(
                task_id="from-alpha",
                context_id=agent_id,
                from_agent_id="sender-alpha",
            )
        )
        await task_store.save(
            _make_task(
                task_id="from-beta",
                context_id=agent_id,
                from_agent_id="sender-beta",
            )
        )

        alpha_tasks = await task_store.list_by_sender("sender-alpha")
        beta_tasks = await task_store.list_by_sender("sender-beta")

        assert {t.id for t in alpha_tasks} == {"from-alpha"}
        assert {t.id for t in beta_tasks} == {"from-beta"}


# ---------------------------------------------------------------------------
# get_endpoints
# ---------------------------------------------------------------------------


class TestGetEndpoints:
    """get_endpoints returns the ``(from_agent_id, to_agent_id)`` pair.

    Used by ``main.py::_handle_get_task`` to resolve the authorization
    check (is the caller either end of the conversation?) without
    parsing the task_json blob.
    """

    async def test_get_endpoints_returns_from_and_to(self, task_store, store):
        """Happy path: returns the tuple written at save time."""
        agent_id = await _seed_agent(store)
        await task_store.save(
            _make_task(
                task_id="task-ep",
                context_id=agent_id,
                from_agent_id="alice",
                to_agent_id="bob",
            )
        )

        endpoints = await task_store.get_endpoints("task-ep")
        assert endpoints == ("alice", "bob"), (
            f"get_endpoints should return (from_agent_id, to_agent_id); "
            f"got {endpoints!r}"
        )

    async def test_get_endpoints_returns_none_for_missing(self, task_store):
        """Missing task_id returns ``None`` (callers branch on that)."""
        endpoints = await task_store.get_endpoints("not-a-task")
        assert endpoints is None


# ---------------------------------------------------------------------------
# get_created_at
# ---------------------------------------------------------------------------


class TestGetCreatedAt:
    """get_created_at returns the ISO 8601 timestamp string.

    Used by ``webui_api.py::_format_message`` to label messages with
    their creation time without deserializing the whole task_json.
    """

    async def test_get_created_at_returns_iso_string(self, task_store, store):
        """Returns an ISO 8601 string that round-trips through ``fromisoformat``."""
        agent_id = await _seed_agent(store)
        await task_store.save(_make_task(task_id="task-ca", context_id=agent_id))

        created_at = await task_store.get_created_at("task-ca")
        assert created_at is not None
        parsed = datetime.fromisoformat(created_at)
        assert isinstance(parsed, datetime), (
            f"get_created_at should return an ISO 8601 parseable string; "
            f"got {created_at!r}"
        )

    async def test_get_created_at_returns_none_for_missing(self, task_store):
        """Missing task_id returns ``None``."""
        result = await task_store.get_created_at("missing-task")
        assert result is None


# ---------------------------------------------------------------------------
# Task JSON roundtrip — structural integrity of artifacts + metadata
# ---------------------------------------------------------------------------


class TestTaskJsonRoundtrip:
    """Deep fields (artifacts, metadata) survive save → get unchanged.

    The store only promotes a handful of fields to columns; everything
    else lives in the ``task_json`` TEXT blob. These tests guard the
    invariant that nothing gets silently dropped during serialization.
    """

    async def test_task_json_preserves_artifacts(self, task_store, store):
        """Artifact list (name + parts) comes back intact through save/get."""
        agent_id = await _seed_agent(store)
        task = _make_task(
            task_id="task-art",
            context_id=agent_id,
            text="keep these parts",
        )
        await task_store.save(task)

        retrieved = await task_store.get("task-art")
        assert retrieved is not None
        assert retrieved.artifacts is not None
        assert len(retrieved.artifacts) == 1
        art = retrieved.artifacts[0]
        assert art.name == "message"
        assert len(art.parts) == 1
        assert art.parts[0].root.text == "keep these parts"

    async def test_task_json_preserves_metadata(self, task_store, store):
        """Arbitrary metadata keys survive the JSON roundtrip.

        Hikyaku's routing keys (``fromAgentId``, ``toAgentId``, ``type``)
        are already asserted by ``test_get_endpoints_returns_from_and_to``.
        This test adds an UNRELATED key (``traceId``) to prove the store
        preserves the whole metadata dict, not just the three routing
        keys it promotes to columns.
        """
        agent_id = await _seed_agent(store)
        task = _make_task(
            task_id="task-meta",
            context_id=agent_id,
            from_agent_id="alice",
            to_agent_id="bob",
            extra_metadata={"traceId": "trace-xyz-123"},
        )
        await task_store.save(task)

        retrieved = await task_store.get("task-meta")
        assert retrieved is not None
        assert retrieved.metadata is not None
        assert retrieved.metadata["fromAgentId"] == "alice"
        assert retrieved.metadata["toAgentId"] == "bob"
        assert retrieved.metadata["type"] == "unicast"
        assert retrieved.metadata["traceId"] == "trace-xyz-123", (
            "non-routing metadata keys must survive the task_json roundtrip"
        )


# ---------------------------------------------------------------------------
# origin_task_id column — save / re-save semantics
# ---------------------------------------------------------------------------


class TestOriginTaskId:
    """Tests for the dedicated ``tasks.origin_task_id`` column.

    Design doc 0000013 §"Data model change":

    - Unicast delivery rows write NULL (no ``originTaskId`` in metadata).
    - Broadcast delivery rows + the broadcast summary row itself all
      share one UUID — the summary task's own ``task_id`` — which
      ``TaskStore.save`` reads from ``metadata["originTaskId"]`` and
      writes into the dedicated column.
    - Historical rows (pre-migration) carry NULL; no backfill.
    - Idempotent re-saves preserve the populated value so ACK-flow
      re-saves on broadcast deliveries do not drop the group-membership
      link.
    """

    async def test_save_unicast_path_writes_null_column(self, task_store, store):
        """Saving a task with no ``originTaskId`` in metadata leaves the column NULL.

        This is the unicast default — ``_handle_unicast`` does not set
        ``originTaskId``, so ``metadata.get("originTaskId")`` returns
        ``None`` and the column is written as NULL.
        """
        agent_id = await _seed_agent(store)
        await task_store.save(
            _make_task(task_id="task-uni", context_id=agent_id),
        )

        value = await _read_origin_task_id_column(task_store, "task-uni")
        assert value is None, (
            f"unicast save must leave origin_task_id NULL; got {value!r}"
        )

    async def test_save_broadcast_path_writes_populated_column(
        self, task_store, store
    ):
        """Saving a task with ``metadata["originTaskId"]`` writes that value.

        This is the broadcast-delivery path — ``_handle_broadcast`` sets
        every delivery task's metadata to include the pre-allocated
        summary task id, and ``TaskStore.save`` must promote that value
        from the metadata dict into the dedicated column.
        """
        agent_id = await _seed_agent(store)
        origin = str(uuid.uuid4())
        await task_store.save(
            _make_task(
                task_id="task-bcast",
                context_id=agent_id,
                origin_task_id=origin,
            ),
        )

        value = await _read_origin_task_id_column(task_store, "task-bcast")
        assert value == origin, (
            f"broadcast save must promote metadata['originTaskId'] to the "
            f"dedicated column; expected {origin!r}, got {value!r}"
        )

    async def test_save_summary_self_reference_writes_own_id(
        self, task_store, store
    ):
        """A broadcast summary row self-references: ``origin_task_id == task_id``.

        Per the design doc, the summary task writes its OWN task_id into
        ``origin_task_id`` so every row in a broadcast group — deliveries
        AND summary — shares one non-NULL value, making the whole group
        queryable as ``origin_task_id = '<summary-id>'``.
        """
        agent_id = await _seed_agent(store)
        summary_id = str(uuid.uuid4())
        await task_store.save(
            _make_task(
                task_id=summary_id,
                context_id=agent_id,
                msg_type="broadcast_summary",
                state=TaskState.completed,
                origin_task_id=summary_id,
            ),
        )

        value = await _read_origin_task_id_column(task_store, summary_id)
        assert value == summary_id

    async def test_re_save_preserves_populated_origin_task_id(
        self, task_store, store
    ):
        """Re-saving with the same metadata must not clear ``origin_task_id``.

        The ACK path re-saves a delivery task with a status change from
        ``input_required`` → ``completed``. The task's metadata is
        unchanged (it still contains ``originTaskId``), and the UPSERT
        must therefore preserve the column value.
        """
        agent_id = await _seed_agent(store)
        origin = str(uuid.uuid4())
        task = _make_task(
            task_id="task-resave",
            context_id=agent_id,
            state=TaskState.input_required,
            origin_task_id=origin,
        )
        await task_store.save(task)

        task.status = TaskStatus(
            state=TaskState.completed,
            timestamp=_ts(datetime.now(UTC) + timedelta(minutes=5)),
        )
        await task_store.save(task)

        value = await _read_origin_task_id_column(task_store, "task-resave")
        assert value == origin, (
            f"re-save with unchanged metadata must preserve origin_task_id; "
            f"expected {origin!r}, got {value!r}"
        )


# ---------------------------------------------------------------------------
# list_timeline — tenant-scoped JOIN used by GET /ui/api/timeline
# ---------------------------------------------------------------------------


class TestListTimeline:
    """Tests for ``TaskStore.list_timeline(tenant_id, limit=200)``.

    Design doc 0000013 §"Timeline API" mandates the following query:

        SELECT t.task_json, t.origin_task_id, t.created_at
        FROM tasks t
        JOIN agents a ON a.agent_id = t.context_id
        WHERE a.tenant_id = :tenant_id
          AND t.type != 'broadcast_summary'
        ORDER BY t.status_timestamp DESC
        LIMIT :limit

    Return shape: ``list[tuple[Task, str | None, str]]`` where each
    tuple is ``(Task, origin_task_id, created_at)``. The Task is the
    deserialized ``task_json`` blob; the second element is the raw
    ``origin_task_id`` column (NULL for unicast/historical); the third
    is the ``created_at`` wallclock set at initial INSERT.
    """

    async def test_returns_tenant_tasks(self, task_store, store):
        """Happy path: tasks in the tenant are returned."""
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        await task_store.save(
            _make_task(task_id="tl-1", context_id=agent_id),
        )
        await task_store.save(
            _make_task(task_id="tl-2", context_id=agent_id),
        )

        results = await task_store.list_timeline(tenant_id)
        task_ids = {t.id for (t, _o, _c) in results}
        assert task_ids == {"tl-1", "tl-2"}

    async def test_tuple_shape_is_task_origin_created_at(
        self, task_store, store
    ):
        """Each result is a 3-tuple ``(Task, origin_task_id, created_at)``.

        The Task is a deserialized a2a.types.Task. The origin is either
        a str (broadcast group) or None (unicast). The created_at is an
        ISO 8601 string parseable by ``datetime.fromisoformat``.
        """
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        await task_store.save(
            _make_task(task_id="tl-shape", context_id=agent_id),
        )

        results = await task_store.list_timeline(tenant_id)
        assert len(results) == 1

        row = results[0]
        assert isinstance(row, tuple), f"row must be a tuple, got {type(row)}"
        assert len(row) == 3, f"row must be a 3-tuple, got {len(row)}-tuple"

        task, origin, created_at = row
        assert isinstance(task, Task)
        assert task.id == "tl-shape"
        assert origin is None  # unicast path
        assert isinstance(created_at, str)
        datetime.fromisoformat(created_at)  # must parse

    async def test_returns_origin_task_id_for_broadcast_row(
        self, task_store, store
    ):
        """Broadcast delivery rows surface their ``origin_task_id`` value."""
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        origin = str(uuid.uuid4())
        await task_store.save(
            _make_task(
                task_id="tl-bcast",
                context_id=agent_id,
                origin_task_id=origin,
            ),
        )

        results = await task_store.list_timeline(tenant_id)
        assert len(results) == 1
        _task, origin_val, _created = results[0]
        assert origin_val == origin

    async def test_returns_null_origin_task_id_for_unicast_row(
        self, task_store, store
    ):
        """Unicast rows surface ``origin_task_id = None``."""
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        await task_store.save(
            _make_task(task_id="tl-uni", context_id=agent_id),
        )

        results = await task_store.list_timeline(tenant_id)
        assert len(results) == 1
        _task, origin_val, _created = results[0]
        assert origin_val is None

    async def test_orders_by_status_timestamp_desc(self, task_store, store):
        """Results are ordered newest-first by ``status_timestamp``."""
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        base = datetime(2026, 4, 10, 12, 0, 0, tzinfo=UTC)
        for i in range(3):
            await task_store.save(
                _make_task(
                    task_id=f"tl-order-{i}",
                    context_id=agent_id,
                    timestamp=_ts(base + timedelta(minutes=i)),
                ),
            )

        results = await task_store.list_timeline(tenant_id)
        ids = [t.id for (t, _o, _c) in results]
        assert ids == ["tl-order-2", "tl-order-1", "tl-order-0"], (
            f"list_timeline must be DESC by status_timestamp; got {ids}"
        )

    async def test_excludes_broadcast_summary_rows(self, task_store, store):
        """Rows with ``type = 'broadcast_summary'`` are filtered out.

        Summary rows never appear in the timeline feed — the frontend
        only needs the delivery rows to render reactions. The summary
        row's metadata (recipientIds, recipientCount) is consulted only
        via the group link, not by surfacing the summary row itself.
        """
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        await task_store.save(
            _make_task(
                task_id="tl-regular",
                context_id=agent_id,
                msg_type="unicast",
            ),
        )
        await task_store.save(
            _make_task(
                task_id="tl-summary",
                context_id=agent_id,
                msg_type="broadcast_summary",
                state=TaskState.completed,
            ),
        )

        results = await task_store.list_timeline(tenant_id)
        task_ids = {t.id for (t, _o, _c) in results}
        assert "tl-regular" in task_ids
        assert "tl-summary" not in task_ids, (
            "broadcast_summary rows must be excluded from list_timeline"
        )

    async def test_cross_tenant_isolation(self, task_store, store):
        """Tenant A's list must not include tenant B's tasks."""
        tenant_a, [agent_a] = await _seed_tenant_with_agents(store, n=1)
        tenant_b, [agent_b] = await _seed_tenant_with_agents(store, n=1)

        await task_store.save(
            _make_task(task_id="tl-a", context_id=agent_a),
        )
        await task_store.save(
            _make_task(task_id="tl-b", context_id=agent_b),
        )

        a_results = await task_store.list_timeline(tenant_a)
        b_results = await task_store.list_timeline(tenant_b)

        a_ids = {t.id for (t, _o, _c) in a_results}
        b_ids = {t.id for (t, _o, _c) in b_results}

        assert a_ids == {"tl-a"}
        assert b_ids == {"tl-b"}

    async def test_multi_agent_tenant_includes_all_contexts(
        self, task_store, store
    ):
        """Within one tenant, tasks on ALL member agents' contexts appear."""
        tenant_id, agent_ids = await _seed_tenant_with_agents(store, n=3)
        for i, agent_id in enumerate(agent_ids):
            await task_store.save(
                _make_task(task_id=f"tl-multi-{i}", context_id=agent_id),
            )

        results = await task_store.list_timeline(tenant_id)
        task_ids = {t.id for (t, _o, _c) in results}
        assert task_ids == {"tl-multi-0", "tl-multi-1", "tl-multi-2"}

    async def test_empty_tenant_returns_empty_list(self, task_store, store):
        """A tenant with no tasks yields an empty list (not ``None``)."""
        tenant_id, _agents = await _seed_tenant_with_agents(store, n=1)
        results = await task_store.list_timeline(tenant_id)
        assert results == []

    async def test_respects_explicit_limit(self, task_store, store):
        """``limit=N`` caps the result at N rows."""
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        base = datetime(2026, 4, 10, 0, 0, 0, tzinfo=UTC)
        for i in range(10):
            await task_store.save(
                _make_task(
                    task_id=f"tl-limit-{i}",
                    context_id=agent_id,
                    timestamp=_ts(base + timedelta(seconds=i)),
                ),
            )

        results = await task_store.list_timeline(tenant_id, limit=3)
        assert len(results) == 3

    async def test_default_limit_caps_at_200(self, task_store, store):
        """Calling without ``limit`` caps the result at 200 rows.

        Design doc 0000013 mandates a 200-row hard cap with no
        pagination in v1. The frontend renders whatever the API
        returns; the cap protects the server from unbounded fetches on
        busy tenants.
        """
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        base = datetime(2026, 4, 10, 0, 0, 0, tzinfo=UTC)
        for i in range(205):
            await task_store.save(
                _make_task(
                    task_id=f"tl-cap-{i:03d}",
                    context_id=agent_id,
                    timestamp=_ts(base + timedelta(seconds=i)),
                ),
            )

        results = await task_store.list_timeline(tenant_id)
        assert len(results) == 200, (
            f"default limit must be 200; got {len(results)}"
        )

    async def test_limit_picks_newest_rows(self, task_store, store):
        """When over-capped, the newest-first ORDER BY keeps the top rows."""
        tenant_id, [agent_id] = await _seed_tenant_with_agents(store, n=1)
        base = datetime(2026, 4, 10, 0, 0, 0, tzinfo=UTC)
        for i in range(5):
            await task_store.save(
                _make_task(
                    task_id=f"tl-newest-{i}",
                    context_id=agent_id,
                    timestamp=_ts(base + timedelta(minutes=i)),
                ),
            )

        results = await task_store.list_timeline(tenant_id, limit=2)
        ids = [t.id for (t, _o, _c) in results]
        assert ids == ["tl-newest-4", "tl-newest-3"], (
            f"limit must preserve DESC ordering and keep the newest rows; "
            f"got {ids}"
        )
