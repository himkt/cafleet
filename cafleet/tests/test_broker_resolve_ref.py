"""Broker-side ID prefix resolution (Workstream A).

``resolve_agent_ref`` / ``resolve_task_ref`` turn a full UUID or a unique
prefix into a full id, scoped to the supplied session. Exact full-UUID match
short-circuits before any prefix scan; an ambiguous prefix and a no-match
both raise ``ValueError`` with distinct messages; ``%`` / ``_`` in the ref
are matched literally (autoescape), never as LIKE wildcards.
"""

import pytest

from cafleet import broker
from cafleet.db.models import Agent, Session as SessionModel, Task

_TS = "2026-05-05T12:00:00.000000+00:00"

SESSION_A = "11111111-1111-1111-1111-aaaaaaaaaaaa"
SESSION_B = "22222222-2222-2222-2222-bbbbbbbbbbbb"

# Active agents in session A. ALPHA and BETA share the 8-char "aaaaaaaa"
# prefix (ambiguous); GAMMA's "bbbbbbbb" prefix is unique.
AGENT_ALPHA = "aaaaaaaa-1111-1111-1111-111111111111"
AGENT_BETA = "aaaaaaaa-2222-2222-2222-222222222222"
AGENT_GAMMA = "bbbbbbbb-3333-3333-3333-333333333333"
# Deregistered agent in session A — outside the active base_where.
AGENT_INACTIVE = "dddddddd-5555-5555-5555-555555555555"
# Active agent that lives only in session B.
AGENT_DELTA = "cccccccc-4444-4444-4444-444444444444"

# Tasks whose endpoints are in session A. ALPHA and BETA share "aaaa0000";
# GAMMA's "bbbb0000" prefix is unique.
TASK_ALPHA = "aaaa0000-1111-1111-1111-111111111111"
TASK_BETA = "aaaa0000-2222-2222-2222-222222222222"
TASK_GAMMA = "bbbb0000-3333-3333-3333-333333333333"
# Task whose only endpoint lives in session B.
TASK_DELTA = "cccc0000-4444-4444-4444-444444444444"


def _add_session(sm, session_id: str) -> None:
    with sm() as s:
        s.add(
            SessionModel(
                session_id=session_id,
                label=None,
                created_at=_TS,
                deleted_at=None,
                director_agent_id=None,
            )
        )
        s.commit()


def _add_agent(sm, *, agent_id: str, session_id: str, status: str = "active") -> None:
    with sm() as s:
        s.add(
            Agent(
                agent_id=agent_id,
                session_id=session_id,
                name="agent",
                description="d",
                status=status,
                registered_at=_TS,
                agent_card_json="{}",
            )
        )
        s.commit()


def _add_task(sm, *, task_id: str, from_agent_id: str, to_agent_id: str) -> None:
    with sm() as s:
        s.add(
            Task(
                task_id=task_id,
                context_id=to_agent_id,
                from_agent_id=from_agent_id,
                to_agent_id=to_agent_id,
                type="unicast",
                created_at=_TS,
                status_state="input_required",
                status_timestamp=_TS,
                origin_task_id=None,
                text="body",
            )
        )
        s.commit()


@pytest.fixture
def populated(broker_session):
    sm = broker_session
    _add_session(sm, SESSION_A)
    _add_session(sm, SESSION_B)
    _add_agent(sm, agent_id=AGENT_ALPHA, session_id=SESSION_A)
    _add_agent(sm, agent_id=AGENT_BETA, session_id=SESSION_A)
    _add_agent(sm, agent_id=AGENT_GAMMA, session_id=SESSION_A)
    _add_agent(sm, agent_id=AGENT_INACTIVE, session_id=SESSION_A, status="deregistered")
    _add_agent(sm, agent_id=AGENT_DELTA, session_id=SESSION_B)
    _add_task(sm, task_id=TASK_ALPHA, from_agent_id=AGENT_ALPHA, to_agent_id=AGENT_GAMMA)
    _add_task(sm, task_id=TASK_BETA, from_agent_id=AGENT_ALPHA, to_agent_id=AGENT_GAMMA)
    _add_task(sm, task_id=TASK_GAMMA, from_agent_id=AGENT_ALPHA, to_agent_id=AGENT_GAMMA)
    _add_task(sm, task_id=TASK_DELTA, from_agent_id=AGENT_DELTA, to_agent_id=AGENT_DELTA)
    return sm


# --- resolve_agent_ref ---------------------------------------------------


def test_resolve_agent_ref__exact_full_uuid_returns_unchanged(populated):
    assert broker.resolve_agent_ref(SESSION_A, AGENT_ALPHA) == AGENT_ALPHA


def test_resolve_agent_ref__unique_prefix_resolves(populated):
    assert broker.resolve_agent_ref(SESSION_A, "bbbbbbbb") == AGENT_GAMMA


def test_resolve_agent_ref__ambiguous_prefix_raises(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_agent_ref(SESSION_A, "aaaaaaaa")
    assert str(excinfo.value) == (
        "id prefix 'aaaaaaaa' is ambiguous; supply more characters or the full UUID."
    )


def test_resolve_agent_ref__no_match_raises(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_agent_ref(SESSION_A, "zzzzzzzz")
    assert str(excinfo.value) == "no agent matches id 'zzzzzzzz' in this session."


def test_resolve_agent_ref__other_session_invisible_by_prefix(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_agent_ref(SESSION_A, "cccccccc")
    assert str(excinfo.value) == "no agent matches id 'cccccccc' in this session."


def test_resolve_agent_ref__other_session_invisible_by_full_uuid(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_agent_ref(SESSION_A, AGENT_DELTA)
    assert str(excinfo.value) == f"no agent matches id '{AGENT_DELTA}' in this session."


def test_resolve_agent_ref__inactive_agent_invisible(populated):
    with pytest.raises(ValueError):
        broker.resolve_agent_ref(SESSION_A, AGENT_INACTIVE)


@pytest.mark.parametrize("wildcard", ["_", "%"])
def test_resolve_agent_ref__wildcard_matched_literally(populated, wildcard):
    # Without autoescape, "_"/"%" would LIKE-match every active agent and
    # report ambiguous; with autoescape they match literally → no match.
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_agent_ref(SESSION_A, wildcard)
    assert str(excinfo.value) == f"no agent matches id '{wildcard}' in this session."


# --- resolve_task_ref ----------------------------------------------------


def test_resolve_task_ref__exact_full_uuid_returns_unchanged(populated):
    assert broker.resolve_task_ref(SESSION_A, TASK_ALPHA) == TASK_ALPHA


def test_resolve_task_ref__unique_prefix_resolves(populated):
    assert broker.resolve_task_ref(SESSION_A, "bbbb0000") == TASK_GAMMA


def test_resolve_task_ref__ambiguous_prefix_raises(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_task_ref(SESSION_A, "aaaa0000")
    assert str(excinfo.value) == (
        "id prefix 'aaaa0000' is ambiguous; supply more characters or the full UUID."
    )


def test_resolve_task_ref__no_match_raises(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_task_ref(SESSION_A, "zzzzzzzz")
    assert str(excinfo.value) == "no task matches id 'zzzzzzzz' in this session."


def test_resolve_task_ref__other_session_invisible_by_prefix(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_task_ref(SESSION_A, "cccc0000")
    assert str(excinfo.value) == "no task matches id 'cccc0000' in this session."


def test_resolve_task_ref__other_session_invisible_by_full_uuid(populated):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_task_ref(SESSION_A, TASK_DELTA)
    assert str(excinfo.value) == f"no task matches id '{TASK_DELTA}' in this session."


@pytest.mark.parametrize("wildcard", ["_", "%"])
def test_resolve_task_ref__wildcard_matched_literally(populated, wildcard):
    with pytest.raises(ValueError) as excinfo:
        broker.resolve_task_ref(SESSION_A, wildcard)
    assert str(excinfo.value) == f"no task matches id '{wildcard}' in this session."
