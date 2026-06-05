"""CLI wiring for ID prefix resolution (Workstream A).

Each target-ID input is resolved at the CLI boundary before the existing
broker call: ``--to`` (message send) and ``--id`` (agent show) via
``broker.resolve_agent_ref``; ``--member-id`` (member delete/capture/
send-input/exec/ping, through ``_load_authorized_member``) via
``broker.resolve_agent_ref``; ``--task-id`` (message ack/cancel/show) via
``broker.resolve_task_ref``. A full UUID is accepted on every one of them.
An ambiguous / no-match ref exits 1 with the resolver's message. The acting
``--agent-id`` is never prefix-resolved — a prefix there is rejected by the
existing acting-agent validation.
"""

import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxMultiplexer
from tests._member_cli_helpers import DIRECTOR_ID, MEMBER_ID, _agent

_TS = "2026-05-05T12:00:00.000000+00:00"

ACTING_AGENT = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
TARGET_FULL = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
TARGET_PREFIX = "bbbbbbbb"
TASK_FULL = "cccccccc-cccc-cccc-cccc-cccccccccccc"
TASK_PREFIX = "cccccccc"

AMBIGUOUS_MSG = "id prefix 'aa' is ambiguous; supply more characters or the full UUID."
NO_AGENT_MSG = "no agent matches id 'zz' in this session."
NO_TASK_MSG = "no task matches id 'zz' in this session."


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


def _typed_task(*, task_id: str = TASK_FULL) -> dict:
    return {
        "task_id": task_id,
        "context_id": TARGET_FULL,
        "from_agent_id": ACTING_AGENT,
        "to_agent_id": TARGET_FULL,
        "type": "unicast",
        "created_at": _TS,
        "status_state": "input_required",
        "status_timestamp": _TS,
        "origin_task_id": None,
        "text": "body",
    }


def _record_resolver(monkeypatch, attr: str, return_value: str) -> list[tuple]:
    calls: list[tuple] = []

    def fake(session_id, ref):
        calls.append((session_id, ref))
        return return_value

    monkeypatch.setattr(broker, attr, fake)
    return calls


def _raise_resolver(monkeypatch, attr: str, exc: Exception) -> None:
    def fake(session_id, ref):
        raise exc

    monkeypatch.setattr(broker, attr, fake)


# --- --to (message send) -> resolve_agent_ref ----------------------------


@pytest.mark.parametrize("to_ref", [TARGET_PREFIX, TARGET_FULL])
def test_message_send__to_resolved_before_send_message(
    runner, session_id, monkeypatch, to_ref
):
    resolve_calls = _record_resolver(monkeypatch, "resolve_agent_ref", TARGET_FULL)
    send_calls: list[tuple] = []

    def fake_send(sid, agent_id, to, text):
        send_calls.append((sid, agent_id, to, text))
        return {"task": _typed_task(), "notification_sent": True}

    monkeypatch.setattr(broker, "send_message", fake_send)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "send",
            "--agent-id",
            ACTING_AGENT,
            "--to",
            to_ref,
            "--text",
            "hi",
        ],
    )
    assert result.exit_code == 0, result.output
    # Only --to is resolved; the acting --agent-id is passed through untouched.
    assert resolve_calls == [(session_id, to_ref)]
    assert send_calls == [(session_id, ACTING_AGENT, TARGET_FULL, "hi")]


@pytest.mark.parametrize("msg", [AMBIGUOUS_MSG, NO_AGENT_MSG])
def test_message_send__to_resolver_error_exits_one(runner, session_id, monkeypatch, msg):
    _raise_resolver(monkeypatch, "resolve_agent_ref", ValueError(msg))
    send_calls: list[tuple] = []
    monkeypatch.setattr(
        broker, "send_message", lambda *a, **k: send_calls.append(a)
    )

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "send",
            "--agent-id",
            ACTING_AGENT,
            "--to",
            "aa",
            "--text",
            "hi",
        ],
    )
    assert result.exit_code == 1, result.output
    assert msg in result.output
    assert send_calls == []


# --- --id (agent show) -> resolve_agent_ref ------------------------------


@pytest.mark.parametrize("id_ref", [TARGET_PREFIX, TARGET_FULL])
def test_agent_show__id_resolved_before_get_agent(
    runner, session_id, monkeypatch, id_ref
):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *a, **k: True)
    resolve_calls = _record_resolver(monkeypatch, "resolve_agent_ref", TARGET_FULL)
    get_calls: list[tuple] = []

    def fake_get(agent_id, sid):
        get_calls.append((agent_id, sid))
        return {
            "agent_id": TARGET_FULL,
            "name": "target",
            "description": "d",
            "status": "active",
            "placement": None,
        }

    monkeypatch.setattr(broker, "get_agent", fake_get)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "agent",
            "show",
            "--agent-id",
            ACTING_AGENT,
            "--id",
            id_ref,
        ],
    )
    assert result.exit_code == 0, result.output
    assert resolve_calls == [(session_id, id_ref)]
    assert get_calls == [(TARGET_FULL, session_id)]


@pytest.mark.parametrize("msg", [AMBIGUOUS_MSG, NO_AGENT_MSG])
def test_agent_show__id_resolver_error_exits_one(runner, session_id, monkeypatch, msg):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *a, **k: True)
    _raise_resolver(monkeypatch, "resolve_agent_ref", ValueError(msg))
    get_calls: list[tuple] = []
    monkeypatch.setattr(broker, "get_agent", lambda *a, **k: get_calls.append(a))

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "agent",
            "show",
            "--agent-id",
            ACTING_AGENT,
            "--id",
            "aa",
        ],
    )
    assert result.exit_code == 1, result.output
    assert msg in result.output
    assert get_calls == []


def test_agent_show__prefix_on_acting_agent_id_is_rejected(
    runner, session_id, monkeypatch
):
    # verify_agent_session accepts the full acting UUID only; a prefix is not a
    # member, so it is rejected before the handler body — and never resolved.
    monkeypatch.setattr(
        broker, "verify_agent_session", lambda aid, sid: aid == ACTING_AGENT
    )
    resolve_calls = _record_resolver(monkeypatch, "resolve_agent_ref", TARGET_FULL)
    get_calls: list[tuple] = []
    monkeypatch.setattr(broker, "get_agent", lambda *a, **k: get_calls.append(a))

    acting_prefix = "aaaaaaaa"
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "agent",
            "show",
            "--agent-id",
            acting_prefix,
            "--id",
            TARGET_FULL,
        ],
    )
    assert result.exit_code == 1, result.output
    assert "is not a member of session" in result.output
    assert acting_prefix in result.output
    assert session_id in result.output
    assert resolve_calls == []
    assert get_calls == []


# --- --task-id (message ack) -> resolve_task_ref -------------------------


@pytest.mark.parametrize("task_ref", [TASK_PREFIX, TASK_FULL])
def test_message_ack__task_id_resolved_before_ack_task(
    runner, session_id, monkeypatch, task_ref
):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *a, **k: True)
    resolve_calls = _record_resolver(monkeypatch, "resolve_task_ref", TASK_FULL)
    ack_calls: list[tuple] = []

    def fake_ack(agent_id, task_id):
        ack_calls.append((agent_id, task_id))
        return {"task": _typed_task(task_id=TASK_FULL)}

    monkeypatch.setattr(broker, "ack_task", fake_ack)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "ack",
            "--agent-id",
            ACTING_AGENT,
            "--task-id",
            task_ref,
        ],
    )
    assert result.exit_code == 0, result.output
    assert resolve_calls == [(session_id, task_ref)]
    assert ack_calls == [(ACTING_AGENT, TASK_FULL)]


@pytest.mark.parametrize("msg", [AMBIGUOUS_MSG, NO_TASK_MSG])
def test_message_ack__task_id_resolver_error_exits_one(
    runner, session_id, monkeypatch, msg
):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *a, **k: True)
    _raise_resolver(monkeypatch, "resolve_task_ref", ValueError(msg))
    ack_calls: list[tuple] = []
    monkeypatch.setattr(broker, "ack_task", lambda *a: ack_calls.append(a))

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "ack",
            "--agent-id",
            ACTING_AGENT,
            "--task-id",
            "zz",
        ],
    )
    assert result.exit_code == 1, result.output
    assert msg in result.output
    assert ack_calls == []


# --- --member-id (member delete, via _load_authorized_member) ------------


@pytest.fixture
def _stub_member_tmux(monkeypatch):
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)
    monkeypatch.setattr(TmuxMultiplexer, "send_exit", lambda self, **_: None)
    monkeypatch.setattr(
        TmuxMultiplexer, "wait_for_pane_gone", lambda self, **_: True, raising=False
    )
    monkeypatch.setattr(
        TmuxMultiplexer, "capture_pane", lambda self, **_: "", raising=False
    )


def test_member_delete__member_id_resolved_and_full_id_used_downstream(
    runner, session_id, monkeypatch, _stub_member_tmux
):
    resolve_calls = _record_resolver(monkeypatch, "resolve_agent_ref", MEMBER_ID)
    get_calls: list[tuple] = []

    def fake_get(member_id, sid):
        get_calls.append((member_id, sid))
        return _agent()

    monkeypatch.setattr(broker, "get_agent", fake_get)
    deregister_calls: list[str] = []
    monkeypatch.setattr(
        broker, "deregister_agent", lambda mid: deregister_calls.append(mid) or True
    )

    member_prefix = MEMBER_ID[:8]
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "delete",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            member_prefix,
        ],
    )
    assert result.exit_code == 0, result.output
    assert resolve_calls == [(session_id, member_prefix)]
    # get_agent receives the resolved full id; the member is actually deleted
    # by the resolved full id, not the pasted prefix.
    assert get_calls == [(MEMBER_ID, session_id)]
    assert deregister_calls == [MEMBER_ID]


def test_member_delete__member_id_full_uuid_still_accepted(
    runner, session_id, monkeypatch, _stub_member_tmux
):
    resolve_calls = _record_resolver(monkeypatch, "resolve_agent_ref", MEMBER_ID)
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_k: _agent())
    monkeypatch.setattr(broker, "deregister_agent", lambda *_a, **_k: True)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "delete",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
        ],
    )
    assert result.exit_code == 0, result.output
    assert resolve_calls == [(session_id, MEMBER_ID)]


@pytest.mark.parametrize("msg", [AMBIGUOUS_MSG, NO_AGENT_MSG])
def test_member_delete__member_id_resolver_error_exits_one(
    runner, session_id, monkeypatch, _stub_member_tmux, msg
):
    _raise_resolver(monkeypatch, "resolve_agent_ref", ValueError(msg))
    get_calls: list[tuple] = []
    monkeypatch.setattr(broker, "get_agent", lambda *a, **k: get_calls.append(a))

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "delete",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            "aa",
        ],
    )
    assert result.exit_code == 1, result.output
    assert msg in result.output
    # The resolver ValueError surfaces raw, not via the get_agent fetch wrapper.
    assert "failed to fetch member" not in result.output
    assert get_calls == []
