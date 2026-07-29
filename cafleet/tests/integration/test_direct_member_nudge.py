"""End-to-end contracts for Director-gated member nudge reporting."""

from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from threading import Barrier

import click
import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import cli
from cafleet.db import models
from cafleet.multiplexer.tmux import TmuxMultiplexer
from tests._helpers import _init_registry
from tests.broker._helpers import (
    _create_fleet,
    _member_placement,
    _register_member,
    _register_monitoring_member,
)

HASH_A = "a" * 64
HASH_B = "b" * 64


@pytest.fixture(autouse=True)
def _fixed_stall_interval(monkeypatch):
    monkeypatch.setattr(config.settings, "monitor_stall_interval", 60)


def _ordinary(
    fleet: dict,
    *,
    name: str = "member",
    pane_id: str = "%5",
) -> dict:
    return _register_member(
        fleet["fleet_id"],
        name=name,
        placement=_member_placement(pane_id),
    )


def _capture_pair() -> tuple[str, str]:
    now = datetime.now(UTC)
    return (
        (now - timedelta(seconds=130)).isoformat(),
        (now - timedelta(seconds=60)).isoformat(),
    )


def _claim_nudge(fleet: dict, member: dict) -> dict:
    first, second = _capture_pair()
    seeded = broker.observe_stall_episode(
        fleet["fleet_id"],
        member["member_id"],
        classification="stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
        stall_check=True,
    )
    assert seeded["classification"] == "unknown"
    return broker.observe_stall_episode(
        fleet["fleet_id"],
        member["member_id"],
        classification="stall_candidate",
        captured_at=second,
        content_sha256=HASH_A,
        stall_check=True,
    )


def _queue_ping_failure(fleet: dict, member: dict) -> None:
    claimed = _claim_nudge(fleet, member)
    assert claimed["action"] == "ping"
    broker.record_stall_ping_result(
        fleet["fleet_id"],
        member["member_id"],
        success=False,
    )


def _director_token(fleet: dict, *, classification: str = "finished") -> str:
    result = broker.observe_stall_episode(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        classification=classification,
        captured_at=datetime.now(UTC).isoformat(),
        content_sha256=HASH_A,
        director_gate=True,
    )
    token = result["director_gate_token"]
    assert isinstance(token, str)
    return token


def _report_fleet() -> tuple[dict, dict]:
    fleet = _create_fleet()
    _register_monitoring_member(fleet)
    return fleet, _ordinary(fleet)


def test_scan_live_then_capture_loss_preserves_claim_and_ping_failure_order(
    broker_session,
):
    fleet = _create_fleet()
    member = _ordinary(fleet)
    claimed = _claim_nudge(fleet, member)
    assert claimed["episode_state"] == "nudge_claimed"

    with broker_session() as session, session.begin():
        placement = session.get(models.MemberPlacement, member["member_id"])
        placement.mux_pane_id = None

    lost = broker.observe_stall_episode(
        fleet["fleet_id"],
        member["member_id"],
        classification="unknown",
    )
    failed = broker.record_stall_ping_result(
        fleet["fleet_id"],
        member["member_id"],
        success=False,
    )

    assert lost["action"] == "escalate"
    assert lost["episode_state"] == "escalation_pending"
    assert lost["escalation_reason"] == "ping_interrupted"
    assert failed["episode_state"] == "escalation_pending"
    assert failed["escalation_reason"] == "ping_failed"


def test_ordinary_member_ping_remains_allowed_while_director_awaits(
    broker_session,
    monkeypatch,
):
    fleet = _create_fleet()
    member = _ordinary(fleet)
    director = broker.observe_stall_episode(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        classification="awaiting_user",
        captured_at=datetime.now(UTC).isoformat(),
        content_sha256=HASH_B,
        director_gate=True,
    )
    claimed = _claim_nudge(fleet, member)

    calls: list[dict] = []
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_poll_trigger",
        lambda self, **kwargs: calls.append(kwargs) or True,
    )
    result = CliRunner().invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(member["member_id"]),
        ],
    )

    assert director["director_gate_token"] is None
    assert claimed["action"] == "ping"
    assert result.exit_code == 0, result.output
    assert calls == [
        {
            "target_pane_id": "%5",
            "fleet_id": fleet["fleet_id"],
            "member_id": member["member_id"],
        }
    ]


def test_two_full_spacing_director_captures_allow_exactly_one_preview(
    broker_session,
    monkeypatch,
):
    fleet, member = _report_fleet()
    _queue_ping_failure(fleet, member)
    previews: list[dict] = []
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: previews.append(kwargs) or True,
    )
    first, second = _capture_pair()

    seeded = broker.observe_stall_episode(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        classification="stall_candidate",
        captured_at=first,
        content_sha256=HASH_B,
        director_gate=True,
    )
    gated = broker.observe_stall_episode(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        classification="stall_candidate",
        captured_at=second,
        content_sha256=HASH_B,
        director_gate=True,
    )
    report = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=gated["director_gate_token"],
        finished_member_ids=[],
    )

    assert seeded["classification"] == "unknown"
    assert seeded["director_gate_token"] is None
    assert gated["classification"] == "stalled"
    assert report["preview_outcome"] == "awaiting_ack"
    assert report["escalated_member_ids"] == [member["member_id"]]
    assert [preview["message_id"] for preview in previews] == [
        report["open_message_id"]
    ]
    with pytest.raises(click.ClickException, match="gate"):
        broker.report_monitor_batch(
            fleet["fleet_id"],
            director_gate_token=gated["director_gate_token"],
            finished_member_ids=[],
        )
    assert len(previews) == 1
    director_config = broker.get_monitor_config(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
    )
    assert director_config["stall_episode_state"] == "clear"


@pytest.mark.parametrize("classification", ["working", "awaiting_user"])
def test_unsafe_director_observation_invalidates_gate_without_preview(
    classification,
    broker_session,
    monkeypatch,
):
    fleet, member = _report_fleet()
    _queue_ping_failure(fleet, member)
    token = _director_token(fleet)
    previews: list[dict] = []
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: previews.append(kwargs) or True,
    )

    unsafe = broker.observe_stall_episode(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        classification=classification,
        captured_at=datetime.now(UTC).isoformat(),
        content_sha256=HASH_B,
        director_gate=True,
    )

    assert unsafe["director_gate_token"] is None
    with pytest.raises(click.ClickException, match="gate"):
        broker.report_monitor_batch(
            fleet["fleet_id"],
            director_gate_token=token,
            finished_member_ids=[],
        )
    assert previews == []
    assert [
        row["member_id"]
        for row in broker.list_pending_stall_escalations(fleet["fleet_id"])
    ] == [member["member_id"]]


def test_repeated_failed_preview_reuses_one_message_for_same_finished_member(
    broker_session,
    monkeypatch,
):
    fleet, member = _report_fleet()
    preview_ids: list[int] = []
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: preview_ids.append(kwargs["message_id"]) or False,
    )

    first = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[member["member_id"]],
    )
    retried = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[member["member_id"]],
    )

    assert first["created"] is True
    assert retried["created"] is False
    assert retried["open_message_id"] == first["open_message_id"]
    assert preview_ids == [first["open_message_id"], first["open_message_id"]]
    timeline = broker.list_timeline(fleet["fleet_id"])
    report_messages = [
        row for row in timeline if row["text"].startswith("monitor report batch:")
    ]
    assert len(report_messages) == 1
    assert report_messages[0]["text"].count(str(member["member_id"])) == 1
    with broker_session() as session:
        delivery = session.get(
            models.MonitorReportDelivery,
            first["open_message_id"],
        )
        assert delivery.preview_state == "pending"
        assert delivery.attempt_count == 2


def test_interval_stale_preview_retries_same_id_and_defers_later_finished(
    broker_session,
    monkeypatch,
):
    fleet, first_finished = _report_fleet()
    second_finished = _ordinary(fleet, name="later-finished", pane_id="%6")
    preview_ids: list[int] = []
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: preview_ids.append(kwargs["message_id"]) or True,
    )
    initial = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[first_finished["member_id"]],
    )
    with broker_session() as session, session.begin():
        delivery = session.get(
            models.MonitorReportDelivery,
            initial["open_message_id"],
        )
        delivery.last_attempt_at = (
            datetime.now(UTC) - timedelta(seconds=181)
        ).isoformat()

    recovered = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[second_finished["member_id"]],
    )

    assert recovered["created"] is False
    assert recovered["open_message_id"] == initial["open_message_id"]
    assert recovered["preview_message_id"] == initial["open_message_id"]
    assert preview_ids == [initial["open_message_id"], initial["open_message_id"]]
    body = broker.get_message(
        fleet["fleet_id"],
        initial["open_message_id"],
    )["message"]["text"]
    assert str(first_finished["member_id"]) in body
    assert str(second_finished["member_id"]) not in body


def test_ack_reconciliation_drains_failure_queued_behind_open_report(
    broker_session,
    monkeypatch,
):
    fleet, first = _report_fleet()
    second = _ordinary(fleet, name="queued", pane_id="%6")
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: True,
    )
    initial = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[first["member_id"]],
    )
    _queue_ping_failure(fleet, second)
    blocked = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    broker.ack_message(
        fleet["director"]["member_id"],
        initial["open_message_id"],
    )
    recovered = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )

    assert blocked["created"] is False
    assert blocked["escalated_member_ids"] == []
    assert recovered["created"] is True
    assert recovered["open_message_id"] != initial["open_message_id"]
    assert recovered["escalated_member_ids"] == [second["member_id"]]


def test_over_cap_preview_keeps_full_body_available_until_ack(
    broker_session,
    monkeypatch,
):
    fleet, first = _report_fleet()
    finished = [
        first,
        *[
            _ordinary(
                fleet,
                name=f"member-{index}-with-a-long-descriptive-name",
                pane_id=f"%{index + 6}",
            )
            for index in range(5)
        ],
    ]
    monkeypatch.setattr(config.settings, "max_text_len", 80)
    preview_texts: list[str] = []
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: preview_texts.append(kwargs["text"]) or True,
    )

    report = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[member["member_id"] for member in finished],
    )
    full_body = broker.get_message(
        fleet["fleet_id"],
        report["open_message_id"],
    )["message"]["text"]

    assert len(preview_texts) == 1
    assert len(preview_texts[0]) == 81
    assert preview_texts[0].endswith("…")
    assert len(full_body) > len(preview_texts[0])
    assert all(str(member["member_id"]) in full_body for member in finished)

    broker.ack_message(
        fleet["director"]["member_id"],
        report["open_message_id"],
    )
    broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    with broker_session() as session:
        delivery = session.get(
            models.MonitorReportDelivery,
            report["open_message_id"],
        )
        assert delivery.preview_state == "delivered"
        assert delivery.delivered_at is not None


@pytest.mark.usefixtures("_reset_engine_singletons")
def test_concurrent_gate_replay_has_one_report_and_one_preview(
    tmp_path,
    monkeypatch,
):
    database_path = tmp_path / "concurrent-gate" / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{database_path}",
    )
    _init_registry()
    fleet, member = _report_fleet()
    _queue_ping_failure(fleet, member)
    token = _director_token(fleet)
    preview_ids: list[int] = []
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda self, **kwargs: preview_ids.append(kwargs["message_id"]) or True,
    )
    barrier = Barrier(2)

    def report() -> tuple[str, object]:
        barrier.wait()
        try:
            return (
                "ok",
                broker.report_monitor_batch(
                    fleet["fleet_id"],
                    director_gate_token=token,
                    finished_member_ids=[],
                ),
            )
        except click.ClickException as exc:
            return "rejected", str(exc)

    with ThreadPoolExecutor(max_workers=2) as pool:
        outcomes = list(pool.map(lambda _index: report(), range(2)))

    assert sorted(outcome[0] for outcome in outcomes) == ["ok", "rejected"]
    successful = next(outcome[1] for outcome in outcomes if outcome[0] == "ok")
    assert successful["created"] is True
    assert preview_ids == [successful["open_message_id"]]
    timeline = broker.list_timeline(fleet["fleet_id"])
    assert (
        len(
            [row for row in timeline if row["text"].startswith("monitor report batch:")]
        )
        == 1
    )
