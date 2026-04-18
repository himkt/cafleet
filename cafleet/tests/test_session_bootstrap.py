"""Broker-level session bootstrap tests (design doc 0000026)."""

import uuid
from unittest.mock import Mock

import click
import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.broker import _is_administrator_card
from cafleet.db.models import (
    Agent,
    AgentPlacement,
    Base,
    Task,
)
from cafleet.db.models import (
    Session as SessionModel,
)
from cafleet.tmux import DirectorContext


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    return sync_sessionmaker


@pytest.fixture
def director_context():
    return DirectorContext(session="main", window_id="@1", pane_id="%0")


def _bootstrap(label: str | None = None, ctx: DirectorContext | None = None) -> dict:
    return broker.create_session(
        label=label,
        director_context=ctx
        or DirectorContext(session="main", window_id="@1", pane_id="%0"),
    )


class TestCreateSessionBootstrap:
    """``broker.create_session`` writes session + Director + placement + Admin."""

    def test_returns_nested_dict_with_required_top_level_keys(self, director_context):
        result = _bootstrap(label="bootstrap-1", ctx=director_context)
        assert isinstance(result, dict)
        for key in (
            "session_id",
            "label",
            "created_at",
            "administrator_agent_id",
            "director",
        ):
            assert key in result, f"missing top-level key {key!r} in {result!r}"

    def test_director_sub_dict_has_required_keys(self, director_context):
        result = _bootstrap(ctx=director_context)
        director = result["director"]
        for key in ("agent_id", "name", "description", "registered_at", "placement"):
            assert key in director, f"missing director key {key!r} in {director!r}"

    def test_director_name_and_description_are_hardcoded(self, director_context):
        result = _bootstrap(ctx=director_context)
        assert result["director"]["name"] == "director"
        assert result["director"]["description"] == "Root Director for this session"

    def test_placement_sub_dict_matches_director_context_and_unknown_coding_agent(
        self, director_context
    ):
        result = _bootstrap(ctx=director_context)
        placement = result["director"]["placement"]
        assert placement["director_agent_id"] is None
        assert placement["tmux_session"] == director_context.session
        assert placement["tmux_window_id"] == director_context.window_id
        assert placement["tmux_pane_id"] == director_context.pane_id
        assert placement["coding_agent"] == "unknown"
        assert "created_at" in placement

    def test_writes_exactly_one_session_row(self, broker_session, director_context):
        result = _bootstrap(ctx=director_context)
        with broker_session() as s:
            rows = s.query(SessionModel).all()
        assert len(rows) == 1
        assert rows[0].session_id == result["session_id"]

    def test_writes_two_agent_rows_director_and_administrator(
        self, broker_session, director_context
    ):
        """After bootstrap: one Director row and one Administrator row, both active."""
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]

        with broker_session() as s:
            rows = s.query(Agent).filter(Agent.session_id == sid).all()

        assert len(rows) == 2
        by_name = {r.name: r for r in rows}
        assert "director" in by_name
        assert "Administrator" in by_name

        director_row = by_name["director"]
        admin_row = by_name["Administrator"]
        assert director_row.status == "active"
        assert admin_row.status == "active"
        assert director_row.agent_id == result["director"]["agent_id"]
        assert admin_row.agent_id == result["administrator_agent_id"]
        assert _is_administrator_card(admin_row.agent_card_json)
        assert not _is_administrator_card(director_row.agent_card_json)

    def test_writes_one_placement_row_for_the_director_only(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        director_id = result["director"]["agent_id"]

        with broker_session() as s:
            placements = s.query(AgentPlacement).all()

        assert len(placements) == 1
        placement = placements[0]
        assert placement.agent_id == director_id
        assert placement.director_agent_id is None
        assert placement.tmux_session == director_context.session
        assert placement.tmux_window_id == director_context.window_id
        assert placement.tmux_pane_id == director_context.pane_id
        assert placement.coding_agent == "unknown"

    def test_sessions_director_agent_id_is_set_to_the_director(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]
        director_id = result["director"]["agent_id"]

        with broker_session() as s:
            row = s.query(SessionModel).filter(SessionModel.session_id == sid).one()

        assert row.director_agent_id == director_id

    def test_administrator_registered_at_equals_session_created_at(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]
        admin_id = result["administrator_agent_id"]

        with broker_session() as s:
            session_row = (
                s.query(SessionModel).filter(SessionModel.session_id == sid).one()
            )
            admin_row = s.query(Agent).filter(Agent.agent_id == admin_id).one()

        assert admin_row.registered_at == session_row.created_at

    def test_sessions_deleted_at_is_null_after_bootstrap(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        with broker_session() as s:
            row = (
                s.query(SessionModel)
                .filter(SessionModel.session_id == result["session_id"])
                .one()
            )
        assert row.deleted_at is None

    def test_label_is_preserved(self, director_context):
        result = _bootstrap(label="hello-world", ctx=director_context)
        assert result["label"] == "hello-world"

    def test_label_may_be_none(self, director_context):
        result = _bootstrap(label=None, ctx=director_context)
        assert result["label"] is None

    def test_each_call_mints_unique_ids(self, director_context):
        a = _bootstrap(ctx=director_context)
        b = _bootstrap(ctx=director_context)
        assert a["session_id"] != b["session_id"]
        assert a["director"]["agent_id"] != b["director"]["agent_id"]
        assert a["administrator_agent_id"] != b["administrator_agent_id"]


class TestCreateSessionRollback:
    """Any exception during the 5-step bootstrap rolls back the whole transaction."""

    def test_rollback_when_placement_insert_fails_leaves_no_rows(
        self, broker_session, director_context, monkeypatch
    ):
        """Inject an exception after step 2 (INSERT agents) by making the
        ``AgentPlacement`` constructor raise. The ``with session.begin():``
        wrapper must roll back the prior ``sessions`` and ``agents`` INSERTs.
        """

        class _BoomPlacement:
            def __init__(self, *args, **kwargs):
                raise RuntimeError("injected failure after INSERT agents")

        # Patch the reference the broker module looks up during create_session.
        monkeypatch.setattr(broker, "AgentPlacement", _BoomPlacement)

        with pytest.raises(RuntimeError, match="injected failure"):
            broker.create_session(label="rollback", director_context=director_context)

        with broker_session() as s:
            sessions = s.query(SessionModel).count()
            agents = s.query(Agent).count()
            placements = s.query(AgentPlacement).count()

        assert sessions == 0
        assert agents == 0
        assert placements == 0


class TestDeleteSessionCascade:
    def test_sets_sessions_deleted_at_and_returns_count_including_director(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]

        ret = broker.delete_session(sid)

        assert isinstance(ret, dict)
        assert ret["deregistered_count"] == 2

        with broker_session() as s:
            row = s.query(SessionModel).filter(SessionModel.session_id == sid).one()
        assert row.deleted_at is not None

    def test_deregisters_all_active_agents_in_the_session(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]

        broker.delete_session(sid)

        with broker_session() as s:
            statuses = {
                r.name: r.status
                for r in s.query(Agent).filter(Agent.session_id == sid).all()
            }
        assert statuses["director"] == "deregistered"
        assert statuses["Administrator"] == "deregistered"

    def test_deletes_placement_rows_in_the_session(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]

        with broker_session() as s:
            assert s.query(AgentPlacement).count() == 1

        broker.delete_session(sid)

        with broker_session() as s:
            assert s.query(AgentPlacement).count() == 0

    def test_tasks_are_preserved_after_soft_delete(
        self, broker_session, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]
        admin_id = result["administrator_agent_id"]
        director_id = result["director"]["agent_id"]

        sent = broker.send_message(sid, admin_id, director_id, "audit me")
        task_id = sent["task"]["id"]

        broker.delete_session(sid)

        with broker_session() as s:
            tasks = s.query(Task).all()
        assert any(t.task_id == task_id for t in tasks)

    def test_idempotent_rerun_returns_zero_deregistered(self, director_context):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]

        first = broker.delete_session(sid)
        second = broker.delete_session(sid)

        assert first["deregistered_count"] == 2
        assert second["deregistered_count"] == 0

    def test_unknown_session_raises_click_exception(self):
        """Deleting a session that was never created raises not-found.

        The broker raises ``click.ClickException`` (exit 1 when the CLI layer
        catches it) rather than ``click.UsageError`` (exit 2 with a Usage:
        banner). Design 0000026 pins the exit code to 1 — same as
        ``session show`` — because "not found" is a runtime condition, not a
        CLI usage error.
        """
        fake_sid = str(uuid.uuid4())
        with pytest.raises(click.ClickException) as exc_info:
            broker.delete_session(fake_sid)
        msg = str(exc_info.value)
        assert "not found" in msg.lower()
        assert fake_sid in msg


class TestRegisterAgentOnSoftDeletedSession:
    """Registration into a soft-deleted session fails with a specific message."""

    def test_rejects_soft_deleted_session_with_expected_error_string(
        self, director_context
    ):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]
        broker.delete_session(sid)

        with pytest.raises(click.UsageError) as exc_info:
            broker.register_agent(
                session_id=sid,
                name="late-comer",
                description="registering after soft delete",
            )
        msg = str(exc_info.value)
        # Design doc exact wording: "session X is deleted"
        assert "is deleted" in msg
        assert sid in msg
        # Must NOT use the "not found" path — the row still exists.
        assert "not found" not in msg.lower()

    def test_unknown_session_still_says_not_found(self):
        """Make sure the soft-delete guard did not replace the not-found path."""
        with pytest.raises(click.UsageError) as exc_info:
            broker.register_agent(
                session_id=str(uuid.uuid4()),
                name="stranger",
                description="no such session",
            )
        assert "not found" in str(exc_info.value).lower()


class TestListSessionsFiltersSoftDeleted:
    def test_hides_soft_deleted_sessions(self, director_context):
        alive = _bootstrap(label="alive", ctx=director_context)
        dead = _bootstrap(label="dead", ctx=director_context)
        broker.delete_session(dead["session_id"])

        sessions = broker.list_sessions()
        ids = {s["session_id"] for s in sessions}

        assert alive["session_id"] in ids
        assert dead["session_id"] not in ids

    def test_get_session_still_returns_soft_deleted_row(self, director_context):
        result = _bootstrap(label="dead-but-visible", ctx=director_context)
        sid = result["session_id"]
        broker.delete_session(sid)

        row = broker.get_session(sid)
        assert row is not None
        assert row["deleted_at"] is not None


class TestDeregisterAgentRootDirector:
    def test_rejects_root_director_with_specific_error(self, director_context):
        result = _bootstrap(ctx=director_context)
        director_id = result["director"]["agent_id"]

        with pytest.raises(click.UsageError) as exc_info:
            broker.deregister_agent(director_id)

        msg = str(exc_info.value)
        assert "cannot deregister the root Director" in msg
        assert "cafleet session delete" in msg

    def test_state_unchanged_after_rejection(self, broker_session, director_context):
        result = _bootstrap(ctx=director_context)
        director_id = result["director"]["agent_id"]
        sid = result["session_id"]

        with pytest.raises(click.UsageError):
            broker.deregister_agent(director_id)

        with broker_session() as s:
            d_row = s.query(Agent).filter(Agent.agent_id == director_id).one()
            p_row = (
                s.query(AgentPlacement)
                .filter(AgentPlacement.agent_id == director_id)
                .one()
            )
            sess_row = (
                s.query(SessionModel).filter(SessionModel.session_id == sid).one()
            )

        assert d_row.status == "active"
        assert d_row.deregistered_at is None
        assert p_row.tmux_pane_id == director_context.pane_id
        assert sess_row.director_agent_id == director_id

    def test_non_root_director_agent_can_still_be_deregistered(self, director_context):
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]

        member = broker.register_agent(
            session_id=sid,
            name="regular-member",
            description="regular member",
        )

        assert broker.deregister_agent(member["agent_id"]) is True


class TestMemberToDirectorNotification:
    """After bootstrap, sending to the root Director triggers a tmux push."""

    def test_send_message_invokes_send_poll_trigger_with_director_pane(
        self, director_context, monkeypatch
    ):
        mock_trigger = Mock(return_value=True)
        monkeypatch.setattr("cafleet.tmux.send_poll_trigger", mock_trigger)

        result = broker.create_session(
            label="notify", director_context=director_context
        )
        sid = result["session_id"]
        root_director_id = result["director"]["agent_id"]

        member = broker.register_agent(
            session_id=sid,
            name="member",
            description="member under the root Director",
            placement={
                "director_agent_id": root_director_id,
                "tmux_session": "main",
                "tmux_window_id": "@1",
                "tmux_pane_id": "%1",
                "coding_agent": "claude",
            },
        )

        response = broker.send_message(
            sid, member["agent_id"], to=root_director_id, text="hi director"
        )

        assert response["notification_sent"] is True
        assert mock_trigger.call_count == 1
        kwargs = mock_trigger.call_args.kwargs
        assert kwargs["target_pane_id"] == director_context.pane_id
        assert kwargs["session_id"] == sid
        assert kwargs["agent_id"] == root_director_id
