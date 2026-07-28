"""Broker contracts for durable monitor stall episodes and aggregate delivery."""

from datetime import UTC, datetime, timedelta

import click
import pytest
from sqlalchemy import text
from sqlalchemy.exc import IntegrityError

from cafleet import broker
from cafleet.config import settings
from cafleet.db import models
from cafleet.multiplexer.tmux import TmuxMultiplexer
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
    monkeypatch.setattr(settings, "monitor_stall_interval", 60)


def _ordinary(fleet: dict, name: str = "member", pane_id: str | None = "%5") -> dict:
    return _register_member(
        fleet["fleet_id"],
        name=name,
        placement=_member_placement(pane_id),
    )


def _capture_times() -> tuple[str, str, str]:
    now = datetime.now(UTC)
    return (
        (now - timedelta(seconds=130)).isoformat(),
        (now - timedelta(seconds=100)).isoformat(),
        (now - timedelta(seconds=60)).isoformat(),
    )


def _observe(
    fleet_id: int,
    member_id: int,
    classification: str,
    *,
    captured_at: str | None = None,
    content_sha256: str | None = None,
    stall_check: bool = False,
    director_gate: bool = False,
) -> dict:
    return broker.observe_stall_episode(
        fleet_id,
        member_id,
        classification=classification,
        captured_at=captured_at,
        content_sha256=content_sha256,
        stall_check=stall_check,
        director_gate=director_gate,
    )


def _claim_nudge(fleet: dict, member_id: int, capture_hash: str = HASH_A) -> dict:
    first, _early, full = _capture_times()
    seeded = _observe(
        fleet["fleet_id"],
        member_id,
        "stall_candidate",
        captured_at=first,
        content_sha256=capture_hash,
        stall_check=True,
    )
    assert seeded["classification"] == "unknown"
    return _observe(
        fleet["fleet_id"],
        member_id,
        "stall_candidate",
        captured_at=full,
        content_sha256=capture_hash,
        stall_check=True,
    )


def _queue_ping_failure(fleet: dict, member_id: int) -> None:
    claimed = _claim_nudge(fleet, member_id)
    assert claimed["action"] == "ping"
    broker.record_stall_ping_result(
        fleet["fleet_id"],
        member_id,
        success=False,
    )


def _director_token(fleet: dict) -> str:
    result = _observe(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        "finished",
        captured_at=datetime.now(UTC).isoformat(),
        content_sha256=HASH_A,
        director_gate=True,
    )
    token = result["director_gate_token"]
    assert isinstance(token, str)
    assert len(token) == 64
    return token


def _setup_report_fleet() -> tuple[dict, dict]:
    fleet = _create_fleet()
    _register_monitoring_member(fleet)
    member = _ordinary(fleet)
    return fleet, member


def _write_invalid_config(session, assignment: str, member_id: int) -> None:
    session.execute(
        text(
            f"UPDATE monitor_config SET {assignment} "  # noqa: S608
            "WHERE member_id = :member_id"
        ),
        {"member_id": member_id},
    )
    session.commit()


def _commit_model(session, instance) -> None:
    session.add(instance)
    session.commit()


def test_enrollment_initializes_clear_episode_defaults():
    fleet = _create_fleet()
    member = _ordinary(fleet)

    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])

    assert config["last_stall_check_at"] is None
    assert config["last_stall_candidate_at"] is None
    assert config["last_stall_capture_sha256"] is None
    assert config["stall_episode_state"] == "clear"
    assert config["stall_escalation_reason"] is None


@pytest.mark.parametrize(
    "assignment",
    [
        "stall_episode_state = 'invalid'",
        "stall_escalation_reason = 'invalid'",
        "last_stall_candidate_at = '2026-07-28T00:00:00+00:00'",
        "stall_episode_state = 'nudged'",
        "stall_episode_state = 'escalation_pending'",
    ],
)
def test_monitor_config_database_checks_reject_invalid_episode_rows(
    broker_session, assignment
):
    fleet = _create_fleet()
    member = _ordinary(fleet)

    with broker_session() as session, pytest.raises(IntegrityError):
        _write_invalid_config(session, assignment, member["member_id"])


@pytest.mark.parametrize(
    "overrides",
    [
        pytest.param({"preview_state": "invalid"}, id="invalid-state"),
        pytest.param({"attempt_count": -1}, id="negative-attempt-count"),
        pytest.param(
            {
                "attempt_count": 0,
                "last_attempt_at": "2026-07-28T00:00:01+00:00",
            },
            id="zero-attempt-with-timestamp",
        ),
        pytest.param(
            {"preview_state": "awaiting_ack", "attempt_count": 0},
            id="awaiting-ack-without-attempt",
        ),
        pytest.param(
            {"preview_state": "delivered", "attempt_count": 1},
            id="delivered-without-delivered-at",
        ),
        pytest.param(
            {
                "preview_state": "pending",
                "delivered_at": "2026-07-28T00:00:02+00:00",
            },
            id="pending-with-delivered-at",
        ),
    ],
)
def test_report_delivery_database_checks_reject_invalid_rows(broker_session, overrides):
    fleet = _create_fleet()
    member = _ordinary(fleet)
    sent = broker.send_message(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        member["member_id"],
        "delivery row",
    )
    values = {
        "message_id": sent["message"]["message_id"],
        "fleet_id": fleet["fleet_id"],
        "preview_state": "pending",
        "attempt_count": 0,
        "last_attempt_at": None,
        "delivered_at": None,
        **overrides,
    }

    with broker_session() as session, pytest.raises(IntegrityError):
        _commit_model(session, models.MonitorReportDelivery(**values))


def test_report_delivery_allows_at_most_one_open_row_per_fleet(broker_session):
    fleet = _create_fleet()
    member = _ordinary(fleet)
    message_ids = [
        broker.send_message(
            fleet["fleet_id"],
            fleet["director"]["member_id"],
            member["member_id"],
            text_body,
        )["message"]["message_id"]
        for text_body in ("one", "two")
    ]

    with broker_session() as session:
        session.add(
            models.MonitorReportDelivery(
                message_id=message_ids[0],
                fleet_id=fleet["fleet_id"],
                preview_state="pending",
                attempt_count=0,
            )
        )
        session.commit()
        with pytest.raises(IntegrityError):
            _commit_model(
                session,
                models.MonitorReportDelivery(
                    message_id=message_ids[1],
                    fleet_id=fleet["fleet_id"],
                    preview_state="awaiting_ack",
                    attempt_count=1,
                    last_attempt_at="2026-07-28T00:00:01+00:00",
                ),
            )


@pytest.mark.parametrize(
    "overrides",
    [
        pytest.param({"token_sha256": "short"}, id="malformed-digest"),
        pytest.param({"classification": "working"}, id="unsafe-classification"),
        pytest.param(
            {
                "expires_at": "2026-07-28T00:00:00+00:00",
            },
            id="non-increasing-expiry",
        ),
    ],
)
def test_director_gate_database_checks_reject_invalid_rows(broker_session, overrides):
    fleet = _create_fleet()
    values = {
        "fleet_id": fleet["fleet_id"],
        "director_member_id": fleet["director"]["member_id"],
        "token_sha256": HASH_A,
        "classification": "finished",
        "issued_at": "2026-07-28T00:00:00+00:00",
        "expires_at": "2026-07-28T00:00:30+00:00",
        **overrides,
    }

    with broker_session() as session, pytest.raises(IntegrityError):
        _commit_model(session, models.MonitorDirectorGate(**values))


def test_first_stall_candidate_seeds_baseline_without_action():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    first, _early, _full = _capture_times()

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
        stall_check=True,
    )

    assert result["classification"] == "unknown"
    assert result["action"] == "none"
    assert result["episode_state"] == "clear"
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["last_stall_candidate_at"] == first
    assert config["last_stall_capture_sha256"] == HASH_A


def test_too_early_identical_candidate_does_not_advance_baseline():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    first, early, _full = _capture_times()
    _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
        stall_check=True,
    )

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=early,
        content_sha256=HASH_A,
        stall_check=True,
    )

    assert result["classification"] == "unknown"
    assert result["action"] == "none"
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["last_stall_candidate_at"] == first


def test_full_spacing_changed_candidate_resolves_working_and_reseeds():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    first, _early, full = _capture_times()
    _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
        stall_check=True,
    )

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=full,
        content_sha256=HASH_B,
        stall_check=True,
    )

    assert result["classification"] == "working"
    assert result["action"] == "none"
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["last_stall_candidate_at"] == full
    assert config["last_stall_capture_sha256"] == HASH_B


def test_full_spacing_identical_candidate_atomically_claims_one_ping():
    fleet = _create_fleet()
    member = _ordinary(fleet)

    result = _claim_nudge(fleet, member["member_id"])

    assert result["classification"] == "stalled"
    assert result["action"] == "ping"
    assert result["episode_state"] == "nudge_claimed"
    assert result["escalation_reason"] is None
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["stall_episode_state"] == "nudge_claimed"


def test_candidate_without_stall_check_cannot_seed_or_promote():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    first, _early, _full = _capture_times()

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
    )

    assert result["classification"] == "unknown"
    assert result["action"] == "none"
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["last_stall_candidate_at"] is None
    assert config["last_stall_capture_sha256"] is None


@pytest.mark.parametrize(
    ("captured_at", "content_sha256"),
    [
        pytest.param(
            datetime.now(UTC).isoformat(),
            None,
            id="timestamp-without-hash",
        ),
        pytest.param(
            None,
            HASH_A,
            id="hash-without-timestamp",
        ),
        pytest.param(
            datetime.now(UTC).isoformat(),
            "A" * 64,
            id="uppercase-hash",
        ),
        pytest.param(
            (datetime.now(UTC) + timedelta(hours=1)).isoformat(),
            HASH_A,
            id="future-timestamp",
        ),
    ],
)
def test_observe_rejects_invalid_capture_identity(captured_at, content_sha256):
    fleet = _create_fleet()
    member = _ordinary(fleet)

    with pytest.raises(click.ClickException):
        _observe(
            fleet["fleet_id"],
            member["member_id"],
            "stall_candidate",
            captured_at=captured_at,
            content_sha256=content_sha256,
            stall_check=True,
        )


def test_explicit_working_resets_claim_even_with_identical_hash():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    _first, _early, full = _capture_times()

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "working",
        captured_at=full,
        content_sha256=HASH_A,
        stall_check=True,
    )

    assert result["classification"] == "working"
    assert result["action"] == "none"
    assert result["episode_state"] == "clear"
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["last_stall_candidate_at"] is None
    assert config["last_stall_capture_sha256"] is None


def test_successful_ping_result_is_idempotent_and_records_nudged():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])

    first = broker.record_stall_ping_result(
        fleet["fleet_id"], member["member_id"], success=True
    )
    replay = broker.record_stall_ping_result(
        fleet["fleet_id"], member["member_id"], success=True
    )

    assert first == replay
    assert first["episode_state"] == "nudged"
    assert first["escalation_reason"] is None


def test_contradictory_ping_result_is_rejected_without_erasing_success():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    broker.record_stall_ping_result(
        fleet["fleet_id"], member["member_id"], success=True
    )

    with pytest.raises(click.ClickException):
        broker.record_stall_ping_result(
            fleet["fleet_id"], member["member_id"], success=False
        )

    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["stall_episode_state"] == "nudged"
    assert config["stall_escalation_reason"] is None


def test_failed_ping_is_sticky_and_pending_list_includes_disabled_member():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _queue_ping_failure(fleet, member["member_id"])
    broker.update_monitor_config(fleet["fleet_id"], member["member_id"], enabled=False)

    pending = broker.list_pending_stall_escalations(fleet["fleet_id"])

    assert pending == [
        {
            "member_id": member["member_id"],
            "name": member["name"],
            "escalation_reason": "ping_failed",
        }
    ]
    config = broker.get_monitor_config(fleet["fleet_id"], member["member_id"])
    assert config["stall_episode_state"] == "escalation_pending"
    assert config["stall_escalation_reason"] == "ping_failed"


def test_unchanged_followup_after_unrecorded_claim_queues_interruption():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    followup = (datetime.now(UTC) - timedelta(seconds=1)).isoformat()

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "stall_candidate",
        captured_at=followup,
        content_sha256=HASH_A,
        stall_check=True,
    )

    assert result["action"] == "escalate"
    assert result["episode_state"] == "escalation_pending"
    assert result["escalation_reason"] == "ping_interrupted"


def test_disabling_claimed_episode_queues_interruption_and_clears_dispatch(
    broker_session,
):
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    with broker_session() as session:
        session.execute(
            text(
                "UPDATE monitor_config SET last_stall_check_at = :stamp "
                "WHERE member_id = :member_id"
            ),
            {
                "stamp": datetime.now(UTC).isoformat(),
                "member_id": member["member_id"],
            },
        )
        session.commit()

    config = broker.update_monitor_config(
        fleet["fleet_id"], member["member_id"], enabled=False
    )

    assert config["enabled"] is False
    assert config["last_stall_check_at"] is None
    assert config["stall_episode_state"] == "escalation_pending"
    assert config["stall_escalation_reason"] == "ping_interrupted"
    assert config["last_stall_candidate_at"] is not None
    assert config["last_stall_capture_sha256"] == HASH_A


def test_loss_tolerant_unknown_converts_claim_to_interruption():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])

    result = _observe(
        fleet["fleet_id"],
        member["member_id"],
        "unknown",
    )

    assert result["classification"] == "unknown"
    assert result["action"] == "escalate"
    assert result["episode_state"] == "escalation_pending"
    assert result["escalation_reason"] == "ping_interrupted"


def test_failure_after_interruption_upgrades_reason_to_ping_failed():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    _observe(fleet["fleet_id"], member["member_id"], "unknown")

    result = broker.record_stall_ping_result(
        fleet["fleet_id"], member["member_id"], success=False
    )

    assert result["episode_state"] == "escalation_pending"
    assert result["escalation_reason"] == "ping_failed"


def test_late_success_after_interruption_is_idempotent_and_keeps_escalation():
    fleet = _create_fleet()
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    _observe(fleet["fleet_id"], member["member_id"], "unknown")

    result = broker.record_stall_ping_result(
        fleet["fleet_id"], member["member_id"], success=True
    )

    assert result["episode_state"] == "escalation_pending"
    assert result["escalation_reason"] == "ping_interrupted"


def test_readable_observation_rejects_pending_placement():
    fleet = _create_fleet()
    member = _ordinary(fleet, pane_id=None)
    first, _early, _full = _capture_times()

    with pytest.raises(click.ClickException, match="live placement"):
        _observe(
            fleet["fleet_id"],
            member["member_id"],
            "working",
            captured_at=first,
            content_sha256=HASH_A,
        )


def test_ordinary_observation_rejects_director_and_monitoring_member():
    fleet = _create_fleet()
    watcher = _register_monitoring_member(fleet)
    first, _early, _full = _capture_times()

    for member_id in (fleet["director"]["member_id"], watcher["member_id"]):
        with pytest.raises(click.ClickException):
            _observe(
                fleet["fleet_id"],
                member_id,
                "working",
                captured_at=first,
                content_sha256=HASH_A,
            )


def test_finished_director_gate_returns_raw_token_but_stores_only_digest(
    broker_session,
):
    fleet = _create_fleet()

    token = _director_token(fleet)

    gate_type = models.MonitorDirectorGate
    with broker_session() as session:
        gate = session.get(gate_type, fleet["fleet_id"])
    assert gate is not None
    assert gate.director_member_id == fleet["director"]["member_id"]
    assert gate.classification == "finished"
    assert gate.token_sha256 != token
    assert len(gate.token_sha256) == 64
    assert datetime.fromisoformat(gate.expires_at) - datetime.fromisoformat(
        gate.issued_at
    ) == timedelta(seconds=30)


def test_full_spacing_identical_director_candidate_issues_gate_without_ping():
    fleet = _create_fleet()
    director_id = fleet["director"]["member_id"]
    first, _early, full = _capture_times()
    seeded = _observe(
        fleet["fleet_id"],
        director_id,
        "stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
        director_gate=True,
    )

    result = _observe(
        fleet["fleet_id"],
        director_id,
        "stall_candidate",
        captured_at=full,
        content_sha256=HASH_A,
        director_gate=True,
    )

    assert seeded["classification"] == "unknown"
    assert seeded["director_gate_token"] is None
    assert result["classification"] == "stalled"
    assert result["action"] == "none"
    assert len(result["director_gate_token"]) == 64
    config = broker.get_monitor_config(fleet["fleet_id"], director_id)
    assert config["stall_episode_state"] == "clear"


def test_unsafe_director_observation_invalidates_prior_gate(broker_session):
    fleet = _create_fleet()
    _director_token(fleet)

    result = _observe(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        "working",
        captured_at=datetime.now(UTC).isoformat(),
        content_sha256=HASH_B,
        director_gate=True,
    )

    assert result["director_gate_token"] is None
    gate_type = models.MonitorDirectorGate
    with broker_session() as session:
        assert session.get(gate_type, fleet["fleet_id"]) is None


def test_disabling_director_deletes_outstanding_gate(broker_session):
    fleet = _create_fleet()
    _director_token(fleet)

    broker.update_monitor_config(
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        enabled=False,
    )

    with broker_session() as session:
        assert session.get(models.MonitorDirectorGate, fleet["fleet_id"]) is None


def test_report_batch_creates_sorted_sanitized_aggregate_and_escalates_rows():
    fleet, first = _setup_report_fleet()
    second = _ordinary(fleet, name="line\ninject", pane_id="%6")
    finished = _ordinary(fleet, name="finished", pane_id="%7")
    _queue_ping_failure(fleet, second["member_id"])
    _queue_ping_failure(fleet, first["member_id"])

    result = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[finished["member_id"], finished["member_id"]],
    )

    assert result["created"] is True
    assert result["escalated_member_ids"] == sorted(
        [first["member_id"], second["member_id"]]
    )
    assert result["finished_member_ids"] == [finished["member_id"]]
    message = broker.get_message(
        fleet["fleet_id"],
        result["created_message_id"],
    )
    assert (
        message["from_member_id"]
        == broker.find_monitoring_member(fleet["fleet_id"])["member_id"]
    )
    assert message["to_member_id"] == fleet["director"]["member_id"]
    assert message["status_state"] == "input_required"
    assert message["text"].startswith("monitor report batch:\n")
    assert "\ninject" not in message["text"]
    assert message["text"].count("\n- ") == 3


def test_open_aggregate_backpressure_leaves_new_escalation_pending():
    fleet, first = _setup_report_fleet()
    _queue_ping_failure(fleet, first["member_id"])
    initial = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    second = _ordinary(fleet, name="second", pane_id="%6")
    _queue_ping_failure(fleet, second["member_id"])

    blocked = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[second["member_id"]],
    )

    assert blocked["created"] is False
    assert blocked["open_message_id"] == initial["open_message_id"]
    assert blocked["escalated_member_ids"] == []
    assert blocked["finished_member_ids"] == []
    pending = broker.list_pending_stall_escalations(fleet["fleet_id"])
    assert [row["member_id"] for row in pending] == [second["member_id"]]


def test_ack_reconciliation_allows_next_aggregate_to_drain_pending():
    fleet, first = _setup_report_fleet()
    _queue_ping_failure(fleet, first["member_id"])
    initial = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    second = _ordinary(fleet, name="second", pane_id="%6")
    _queue_ping_failure(fleet, second["member_id"])
    broker.ack_message(
        fleet["director"]["member_id"],
        initial["open_message_id"],
    )

    recovered = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )

    assert recovered["created"] is True
    assert recovered["open_message_id"] != initial["open_message_id"]
    assert recovered["escalated_member_ids"] == [second["member_id"]]
    assert broker.list_pending_stall_escalations(fleet["fleet_id"]) == []


def test_director_gate_token_is_single_use():
    fleet, member = _setup_report_fleet()
    _queue_ping_failure(fleet, member["member_id"])
    token = _director_token(fleet)
    broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=token,
        finished_member_ids=[],
    )

    with pytest.raises(click.ClickException, match="gate"):
        broker.report_monitor_batch(
            fleet["fleet_id"],
            director_gate_token=token,
            finished_member_ids=[],
        )


def test_report_validation_failure_rolls_back_gate_consumption():
    fleet, _member = _setup_report_fleet()
    token = _director_token(fleet)

    with pytest.raises(click.ClickException):
        broker.report_monitor_batch(
            fleet["fleet_id"],
            director_gate_token=token,
            finished_member_ids=[999999],
        )

    recovered = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=token,
        finished_member_ids=[],
    )
    assert recovered["created"] is False
    assert recovered["open_message_id"] is None
    assert recovered["preview_outcome"] == "none"


def test_failed_preview_retries_same_message_id_and_records_each_attempt(
    broker_session, monkeypatch
):
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: False,
    )
    fleet, member = _setup_report_fleet()
    _queue_ping_failure(fleet, member["member_id"])

    first = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    retried = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )

    assert first["preview_outcome"] == "failed"
    assert retried["created"] is False
    assert retried["open_message_id"] == first["open_message_id"]
    assert retried["preview_message_id"] == first["preview_message_id"]
    assert retried["preview_outcome"] == "failed"
    with broker_session() as session:
        delivery = session.get(
            models.MonitorReportDelivery,
            first["open_message_id"],
        )
    assert delivery.preview_state == "pending"
    assert delivery.attempt_count == 2
    assert delivery.last_attempt_at is not None
    assert delivery.delivered_at is None


def test_successful_preview_waits_for_ack_before_delivery_or_retry(
    broker_session, monkeypatch
):
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: True,
    )
    fleet, member = _setup_report_fleet()
    _queue_ping_failure(fleet, member["member_id"])

    first = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    immediate = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )

    assert first["preview_outcome"] == "awaiting_ack"
    assert immediate["created"] is False
    assert immediate["open_message_id"] == first["open_message_id"]
    assert immediate["preview_message_id"] is None
    assert immediate["preview_outcome"] == "none"
    with broker_session() as session:
        delivery = session.get(
            models.MonitorReportDelivery,
            first["open_message_id"],
        )
    assert delivery.preview_state == "awaiting_ack"
    assert delivery.attempt_count == 1
    assert delivery.delivered_at is None


def test_fleet_teardown_explicitly_removes_delivery_and_gate_rows(broker_session):
    fleet, member = _setup_report_fleet()
    _queue_ping_failure(fleet, member["member_id"])
    report = broker.report_monitor_batch(
        fleet["fleet_id"],
        director_gate_token=_director_token(fleet),
        finished_member_ids=[],
    )
    _director_token(fleet)

    broker.delete_fleet(fleet["fleet_id"])

    with broker_session() as session:
        assert (
            session.get(
                models.MonitorReportDelivery,
                report["open_message_id"],
            )
            is None
        )
        assert session.get(models.MonitorDirectorGate, fleet["fleet_id"]) is None
