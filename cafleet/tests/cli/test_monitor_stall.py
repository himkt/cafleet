"""CLI contracts for durable monitor stall state and aggregate reporting."""

import json
import sqlite3
from datetime import UTC, datetime, timedelta

import pytest

from cafleet import broker
from cafleet.cli import cli
from cafleet.config import settings
from cafleet.multiplexer.tmux import TmuxMultiplexer

HASH_A = "a" * 64
HASH_B = "b" * 64


@pytest.fixture(autouse=True)
def _fixed_stall_interval(monkeypatch):
    monkeypatch.setattr(settings, "monitor_stall_interval", 60)


@pytest.fixture
def stall_fleet(_cli_registry, make_bootstrapped_fleet):
    runner, fleet = make_bootstrapped_fleet()
    return _cli_registry, runner, fleet


def _placement(pane_id: str | None) -> dict:
    return {
        "backend": "tmux",
        "mux_session": "main",
        "mux_window_id": "@3",
        "mux_pane_id": pane_id,
        "coding_agent": "claude",
    }


def _ordinary(
    fleet: dict,
    *,
    name: str = "member",
    pane_id: str | None = "%5",
) -> dict:
    return broker.register_member(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="ordinary member",
        placement=_placement(pane_id),
    )


def _watcher(fleet: dict, pane_id: str = "%7") -> dict:
    return broker.register_member(
        fleet_id=fleet["fleet_id"],
        name="watcher",
        description="monitoring member",
        placement=_placement(pane_id),
        kind="monitoring-member",
    )


def _capture_times() -> tuple[str, str]:
    now = datetime.now(UTC)
    return (
        (now - timedelta(seconds=130)).isoformat(),
        (now - timedelta(seconds=60)).isoformat(),
    )


def _observe_args(
    fleet_id: int,
    member_id: int,
    classification: str,
    *,
    captured_at: str | None = None,
    capture_sha256: str | None = None,
    stall_check: bool = False,
    director_gate: bool = False,
    json_output: bool = True,
) -> list[str]:
    args = [
        "monitor",
        "stall",
        "observe",
        "--fleet-id",
        str(fleet_id),
        "--member-id",
        str(member_id),
        "--classification",
        classification,
    ]
    if captured_at is not None:
        args.extend(["--captured-at", captured_at])
    if capture_sha256 is not None:
        args.extend(["--capture-sha256", capture_sha256])
    if stall_check:
        args.append("--stall-check")
    if director_gate:
        args.append("--director-gate")
    if json_output:
        args.append("--json")
    return args


def _invoke_observe(
    runner,
    fleet_id: int,
    member_id: int,
    classification: str,
    **kwargs,
):
    return runner.invoke(
        cli,
        _observe_args(
            fleet_id,
            member_id,
            classification,
            **kwargs,
        ),
    )


def _claim_nudge(fleet: dict, member_id: int) -> None:
    first, full = _capture_times()
    seeded = broker.observe_stall_episode(
        fleet["fleet_id"],
        member_id,
        classification="stall_candidate",
        captured_at=first,
        content_sha256=HASH_A,
        stall_check=True,
    )
    assert seeded["classification"] == "unknown"
    claimed = broker.observe_stall_episode(
        fleet["fleet_id"],
        member_id,
        classification="stall_candidate",
        captured_at=full,
        content_sha256=HASH_A,
        stall_check=True,
    )
    assert claimed["action"] == "ping"


def _queue_ping_failure(fleet: dict, member_id: int) -> None:
    _claim_nudge(fleet, member_id)
    broker.record_stall_ping_result(
        fleet["fleet_id"],
        member_id,
        success=False,
    )


def _director_token(runner, fleet: dict) -> str:
    director_id = fleet["director"]["member_id"]
    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        director_id,
        "finished",
        captured_at=datetime.now(UTC).isoformat(),
        capture_sha256=HASH_A,
        director_gate=True,
    )
    assert result.exit_code == 0, result.output
    token = json.loads(result.output)["director_gate_token"]
    assert isinstance(token, str)
    return token


def _report_args(
    fleet_id: int,
    token: str | None,
    *,
    finished_member_ids: list[int] | None = None,
    json_output: bool = True,
) -> list[str]:
    args = ["monitor", "report-batch", "--fleet-id", str(fleet_id)]
    if token is not None:
        args.extend(["--director-gate-token", token])
    for member_id in finished_member_ids or []:
        args.extend(["--finished-member-id", str(member_id)])
    if json_output:
        args.append("--json")
    return args


def _expire_gate(db_file, fleet_id: int) -> None:
    now = datetime.now(UTC)
    with sqlite3.connect(str(db_file)) as connection:
        connection.execute(
            "UPDATE monitor_director_gate SET issued_at = ?, expires_at = ? "
            "WHERE fleet_id = ?",
            (
                (now - timedelta(seconds=31)).isoformat(),
                (now - timedelta(seconds=1)).isoformat(),
                fleet_id,
            ),
        )
        connection.commit()


def _age_open_preview(db_file, message_id: int, seconds: int) -> None:
    with sqlite3.connect(str(db_file)) as connection:
        connection.execute(
            "UPDATE monitor_report_delivery SET last_attempt_at = ? "
            "WHERE message_id = ?",
            (
                (datetime.now(UTC) - timedelta(seconds=seconds)).isoformat(),
                message_id,
            ),
        )
        connection.commit()


@pytest.mark.parametrize(
    ("classification", "with_capture", "stall_check", "expected"),
    [
        ("awaiting_user", True, False, "awaiting_user"),
        ("unknown", False, False, "unknown"),
        ("finished", True, False, "finished"),
        ("working", True, False, "working"),
        ("stall_candidate", True, True, "unknown"),
    ],
)
def test_monitor_stall_observe__all_typed_classifications_json(
    stall_fleet,
    classification,
    with_capture,
    stall_check,
    expected,
):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)
    kwargs = {"stall_check": stall_check}
    if with_capture:
        kwargs.update(
            captured_at=_capture_times()[0],
            capture_sha256=HASH_A,
        )

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        member["member_id"],
        classification,
        **kwargs,
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert list(payload) == [
        "member_id",
        "classification",
        "action",
        "episode_state",
        "escalation_reason",
        "director_gate_token",
    ]
    assert payload == {
        "member_id": member["member_id"],
        "classification": expected,
        "action": "none",
        "episode_state": "clear",
        "escalation_reason": None,
        "director_gate_token": None,
    }


def test_monitor_stall_observe__exact_text_output(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        member["member_id"],
        "working",
        captured_at=_capture_times()[0],
        capture_sha256=HASH_A,
        json_output=False,
    )

    assert result.exit_code == 0, result.output
    assert result.output == (
        f"member {member['member_id']}: working, action none, episode clear, "
        "reason -, director gate -\n"
    )


def test_monitor_stall_observe__invalid_classification_is_usage_error(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        member["member_id"],
        "stalled",
        captured_at=_capture_times()[0],
        capture_sha256=HASH_A,
    )

    assert result.exit_code == 2, result.output
    assert "no such command" not in result.output.lower()


@pytest.mark.parametrize(
    ("classification", "captured_at", "capture_sha256"),
    [
        ("working", None, None),
        ("working", "2026-07-28T00:00:00+00:00", None),
        ("working", None, HASH_A),
        ("unknown", "2026-07-28T00:00:00+00:00", HASH_A),
    ],
)
def test_monitor_stall_observe__capture_identity_flag_errors_exit_two(
    stall_fleet,
    classification,
    captured_at,
    capture_sha256,
):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        member["member_id"],
        classification,
        captured_at=captured_at,
        capture_sha256=capture_sha256,
    )

    assert result.exit_code == 2, result.output
    assert "no such command" not in result.output.lower()


def test_monitor_stall_observe__stall_check_and_director_gate_are_usage_error(
    stall_fleet,
):
    _db_file, runner, fleet = stall_fleet
    director_id = fleet["director"]["member_id"]

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        director_id,
        "working",
        captured_at=_capture_times()[0],
        capture_sha256=HASH_A,
        stall_check=True,
        director_gate=True,
    )

    assert result.exit_code == 2, result.output
    assert "no such command" not in result.output.lower()


@pytest.mark.parametrize(
    "target_mode",
    ["director-as-ordinary", "ordinary-as-director", "watcher-as-ordinary"],
)
def test_monitor_stall_observe__role_and_mode_target_guards(
    stall_fleet,
    target_mode,
):
    _db_file, runner, fleet = stall_fleet
    ordinary = _ordinary(fleet)
    watcher = _watcher(fleet)
    if target_mode == "director-as-ordinary":
        member_id = fleet["director"]["member_id"]
        director_gate = False
    elif target_mode == "ordinary-as-director":
        member_id = ordinary["member_id"]
        director_gate = True
    else:
        member_id = watcher["member_id"]
        director_gate = False

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        member_id,
        "finished",
        captured_at=_capture_times()[0],
        capture_sha256=HASH_A,
        director_gate=director_gate,
    )

    assert result.exit_code == 1, result.output


def test_monitor_stall_observe__finished_director_returns_safe_gate_without_ping(
    stall_fleet,
):
    _db_file, runner, fleet = stall_fleet
    director_id = fleet["director"]["member_id"]

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        director_id,
        "finished",
        captured_at=_capture_times()[0],
        capture_sha256=HASH_A,
        director_gate=True,
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["classification"] == "finished"
    assert payload["action"] == "none"
    assert payload["episode_state"] == "clear"
    assert payload["escalation_reason"] is None
    assert len(payload["director_gate_token"]) == 64


def test_monitor_stall_observe__confident_director_stall_returns_gate_without_ping(
    stall_fleet,
):
    _db_file, runner, fleet = stall_fleet
    director_id = fleet["director"]["member_id"]
    first, full = _capture_times()
    seeded = _invoke_observe(
        runner,
        fleet["fleet_id"],
        director_id,
        "stall_candidate",
        captured_at=first,
        capture_sha256=HASH_A,
        director_gate=True,
    )
    assert seeded.exit_code == 0, seeded.output

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        director_id,
        "stall_candidate",
        captured_at=full,
        capture_sha256=HASH_A,
        director_gate=True,
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["classification"] == "stalled"
    assert payload["action"] == "none"
    assert payload["episode_state"] == "clear"
    assert len(payload["director_gate_token"]) == 64


@pytest.mark.parametrize(
    ("classification", "with_capture"),
    [
        ("awaiting_user", True),
        ("working", True),
        ("unknown", False),
    ],
)
def test_monitor_stall_observe__unsafe_director_results_return_no_gate(
    stall_fleet,
    classification,
    with_capture,
):
    _db_file, runner, fleet = stall_fleet
    kwargs = {"director_gate": True}
    if with_capture:
        kwargs.update(
            captured_at=_capture_times()[0],
            capture_sha256=HASH_A,
        )

    result = _invoke_observe(
        runner,
        fleet["fleet_id"],
        fleet["director"]["member_id"],
        classification,
        **kwargs,
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["action"] == "none"
    assert payload["director_gate_token"] is None


@pytest.mark.parametrize("flags", [[], ["--success", "--failure"]])
def test_monitor_stall_ping_result__requires_exactly_one_outcome(stall_fleet, flags):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)
    result = runner.invoke(
        cli,
        [
            "monitor",
            "stall",
            "ping-result",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(member["member_id"]),
            *flags,
        ],
    )

    assert result.exit_code == 2, result.output
    assert "no such command" not in result.output.lower()


def test_monitor_stall_ping_result__success_json_and_replay_are_idempotent(
    stall_fleet,
):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    args = [
        "monitor",
        "stall",
        "ping-result",
        "--fleet-id",
        str(fleet["fleet_id"]),
        "--member-id",
        str(member["member_id"]),
        "--success",
        "--json",
    ]

    first = runner.invoke(cli, args)
    replay = runner.invoke(cli, args)

    assert first.exit_code == 0, first.output
    assert replay.exit_code == 0, replay.output
    assert json.loads(first.output) == json.loads(replay.output)
    payload = json.loads(first.output)
    assert list(payload) == [
        "member_id",
        "episode_state",
        "escalation_reason",
    ]
    assert payload == {
        "member_id": member["member_id"],
        "episode_state": "nudged",
        "escalation_reason": None,
    }


def test_monitor_stall_ping_result__failure_exact_text_and_pending_state(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])

    result = runner.invoke(
        cli,
        [
            "monitor",
            "stall",
            "ping-result",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(member["member_id"]),
            "--failure",
        ],
    )

    assert result.exit_code == 0, result.output
    assert result.output == (
        f"member {member['member_id']}: episode escalation_pending, "
        "reason ping_failed\n"
    )
    assert (
        broker.get_monitor_config(fleet["fleet_id"], member["member_id"])[
            "stall_episode_state"
        ]
        == "escalation_pending"
    )


def test_monitor_stall_ping_result__clear_state_conflict_exits_one(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)

    result = runner.invoke(
        cli,
        [
            "monitor",
            "stall",
            "ping-result",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--member-id",
            str(member["member_id"]),
            "--success",
        ],
    )

    assert result.exit_code == 1, result.output
    assert "conflict" in result.output.lower()


def test_monitor_stall_ping_result__contradictory_replay_exits_one(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    member = _ordinary(fleet)
    _claim_nudge(fleet, member["member_id"])
    base = [
        "monitor",
        "stall",
        "ping-result",
        "--fleet-id",
        str(fleet["fleet_id"]),
        "--member-id",
        str(member["member_id"]),
    ]
    succeeded = runner.invoke(cli, [*base, "--success"])
    assert succeeded.exit_code == 0, succeeded.output

    contradicted = runner.invoke(cli, [*base, "--failure"])

    assert contradicted.exit_code == 1, contradicted.output
    assert "conflict" in contradicted.output.lower()


def test_monitor_stall_pending__empty_text_and_json_shapes(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    text_result = runner.invoke(
        cli,
        ["monitor", "stall", "pending", "--fleet-id", str(fleet["fleet_id"])],
    )
    json_result = runner.invoke(
        cli,
        [
            "monitor",
            "stall",
            "pending",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--json",
        ],
    )

    assert text_result.exit_code == 0, text_result.output
    assert text_result.output == "(no pending stall escalations)\n"
    assert json_result.exit_code == 0, json_result.output
    assert json.loads(json_result.output) == {"members": []}


def test_monitor_stall_pending__shows_disabled_and_dead_rows_in_member_order(
    stall_fleet,
):
    db_file, runner, fleet = stall_fleet
    first = _ordinary(fleet, name="first", pane_id="%5")
    second = _ordinary(fleet, name="second", pane_id="%6")
    _queue_ping_failure(fleet, second["member_id"])
    _queue_ping_failure(fleet, first["member_id"])
    broker.update_monitor_config(
        fleet["fleet_id"],
        first["member_id"],
        enabled=False,
    )
    with sqlite3.connect(str(db_file)) as connection:
        connection.execute(
            "UPDATE member_placements SET mux_pane_id = NULL WHERE member_id = ?",
            (second["member_id"],),
        )
        connection.commit()

    result = runner.invoke(
        cli,
        [
            "monitor",
            "stall",
            "pending",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert list(payload) == ["members"]
    assert payload["members"] == [
        {
            "member_id": first["member_id"],
            "name": "first",
            "escalation_reason": "ping_failed",
        },
        {
            "member_id": second["member_id"],
            "name": "second",
            "escalation_reason": "ping_failed",
        },
    ]


@pytest.mark.parametrize(
    "token",
    [
        None,
        "short",
        "A" * 64,
        "g" * 64,
    ],
)
def test_monitor_report_batch__missing_or_malformed_token_exits_two(
    stall_fleet,
    token,
):
    _db_file, runner, fleet = stall_fleet
    result = runner.invoke(cli, _report_args(fleet["fleet_id"], token))

    assert result.exit_code == 2, result.output
    assert "no such command" not in result.output.lower()


def test_monitor_report_batch__malformed_finished_member_id_exits_two(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    result = runner.invoke(
        cli,
        [
            "monitor",
            "report-batch",
            "--fleet-id",
            str(fleet["fleet_id"]),
            "--director-gate-token",
            HASH_A,
            "--finished-member-id",
            "not-an-int",
        ],
    )

    assert result.exit_code == 2, result.output
    assert "no such command" not in result.output.lower()


def test_monitor_report_batch__valid_but_mismatched_token_exits_one(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    _director_token(runner, fleet)

    result = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], HASH_B),
    )

    assert result.exit_code == 1, result.output
    assert "token" in result.output.lower()


def test_monitor_report_batch__expired_token_exits_one(stall_fleet):
    db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    token = _director_token(runner, fleet)
    _expire_gate(db_file, fleet["fleet_id"])

    result = runner.invoke(cli, _report_args(fleet["fleet_id"], token))

    assert result.exit_code == 1, result.output
    assert "expired" in result.output.lower()


def test_monitor_report_batch__replayed_token_exits_one(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    token = _director_token(runner, fleet)
    first = runner.invoke(cli, _report_args(fleet["fleet_id"], token))
    assert first.exit_code == 0, first.output

    replay = runner.invoke(cli, _report_args(fleet["fleet_id"], token))

    assert replay.exit_code == 1, replay.output
    assert "gate" in replay.output.lower()


def test_monitor_report_batch__empty_json_and_exact_text_output(stall_fleet):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    json_token = _director_token(runner, fleet)
    json_result = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], json_token),
    )

    assert json_result.exit_code == 0, json_result.output
    payload = json.loads(json_result.output)
    assert list(payload) == [
        "created_message_id",
        "open_message_id",
        "preview_message_id",
        "escalated_member_ids",
        "finished_member_ids",
        "created",
        "preview_outcome",
    ]
    assert payload == {
        "created_message_id": None,
        "open_message_id": None,
        "preview_message_id": None,
        "escalated_member_ids": [],
        "finished_member_ids": [],
        "created": False,
        "preview_outcome": "none",
    }

    text_token = _director_token(runner, fleet)
    text_result = runner.invoke(
        cli,
        _report_args(
            fleet["fleet_id"],
            text_token,
            json_output=False,
        ),
    )
    assert text_result.exit_code == 0, text_result.output
    assert text_result.output == (
        "monitor report batch: created -, open -, preview - none, "
        "0 escalated, 0 finished\n"
    )


def test_monitor_report_batch__sorts_deduplicates_and_sanitizes_aggregate(
    stall_fleet,
    monkeypatch,
):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    first = _ordinary(fleet, name="first", pane_id="%5")
    unsafe = _ordinary(fleet, name="line\n`$(inject", pane_id="%6")
    finished = _ordinary(fleet, name="done\tmember", pane_id="%8")
    _queue_ping_failure(fleet, unsafe["member_id"])
    _queue_ping_failure(fleet, first["member_id"])
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: False,
    )

    result = runner.invoke(
        cli,
        _report_args(
            fleet["fleet_id"],
            _director_token(runner, fleet),
            finished_member_ids=[
                finished["member_id"],
                finished["member_id"],
            ],
        ),
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["escalated_member_ids"] == [
        first["member_id"],
        unsafe["member_id"],
    ]
    assert payload["finished_member_ids"] == [finished["member_id"]]
    message = broker.get_message(
        fleet["fleet_id"],
        payload["created_message_id"],
    )["message"]
    body = message["text"]
    assert body.startswith("monitor report batch:\n")
    assert "\ninject" not in body
    assert "`" not in body
    assert "$(" not in body
    assert "\t" not in body
    assert body.index(f"member {first['member_id']}") < body.index(
        f"member {unsafe['member_id']}"
    )
    assert body.index(f"member {unsafe['member_id']}") < body.index(
        f"member {finished['member_id']}"
    )


def test_monitor_report_batch__open_delivery_backpressures_new_entries(
    stall_fleet,
    monkeypatch,
):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    first = _ordinary(fleet, name="first", pane_id="%5")
    second = _ordinary(fleet, name="second", pane_id="%6")
    _queue_ping_failure(fleet, first["member_id"])
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: True,
    )
    opened = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )
    assert opened.exit_code == 0, opened.output
    open_payload = json.loads(opened.output)
    _queue_ping_failure(fleet, second["member_id"])

    blocked = runner.invoke(
        cli,
        _report_args(
            fleet["fleet_id"],
            _director_token(runner, fleet),
            finished_member_ids=[second["member_id"]],
        ),
    )

    assert blocked.exit_code == 0, blocked.output
    payload = json.loads(blocked.output)
    assert payload["created"] is False
    assert payload["open_message_id"] == open_payload["open_message_id"]
    assert payload["preview_message_id"] is None
    assert payload["escalated_member_ids"] == []
    assert payload["finished_member_ids"] == []
    assert [
        row["member_id"]
        for row in broker.list_pending_stall_escalations(fleet["fleet_id"])
    ] == [second["member_id"]]


def test_monitor_report_batch__failed_pending_preview_retries_same_message(
    stall_fleet,
    monkeypatch,
):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    member = _ordinary(fleet)
    _queue_ping_failure(fleet, member["member_id"])
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: False,
    )

    first = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )
    retry = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )

    assert first.exit_code == 0, first.output
    assert retry.exit_code == 0, retry.output
    first_payload = json.loads(first.output)
    retry_payload = json.loads(retry.output)
    assert first_payload["preview_outcome"] == "failed"
    assert retry_payload["created"] is False
    assert retry_payload["open_message_id"] == first_payload["open_message_id"]
    assert retry_payload["preview_message_id"] == first_payload["preview_message_id"]
    assert retry_payload["preview_outcome"] == "failed"


def test_monitor_report_batch__awaiting_ack_retries_only_after_director_interval(
    stall_fleet,
    monkeypatch,
):
    db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    member = _ordinary(fleet)
    _queue_ping_failure(fleet, member["member_id"])
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: True,
    )

    first = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )
    assert first.exit_code == 0, first.output
    first_payload = json.loads(first.output)
    immediate = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )
    assert immediate.exit_code == 0, immediate.output
    immediate_payload = json.loads(immediate.output)
    assert immediate_payload["preview_message_id"] is None
    assert immediate_payload["preview_outcome"] == "none"

    _age_open_preview(
        db_file,
        first_payload["open_message_id"],
        seconds=181,
    )
    stale_retry = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )

    assert stale_retry.exit_code == 0, stale_retry.output
    retry_payload = json.loads(stale_retry.output)
    assert retry_payload["created"] is False
    assert retry_payload["open_message_id"] == first_payload["open_message_id"]
    assert retry_payload["preview_message_id"] == first_payload["open_message_id"]
    assert retry_payload["preview_outcome"] == "awaiting_ack"


def test_monitor_report_batch__ack_recovery_drains_later_pending(
    stall_fleet,
    monkeypatch,
):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    first = _ordinary(fleet, name="first", pane_id="%5")
    second = _ordinary(fleet, name="second", pane_id="%6")
    _queue_ping_failure(fleet, first["member_id"])
    monkeypatch.setattr(
        TmuxMultiplexer,
        "send_inline_preview",
        lambda *_args, **_kwargs: True,
    )
    opened = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )
    assert opened.exit_code == 0, opened.output
    opened_payload = json.loads(opened.output)
    _queue_ping_failure(fleet, second["member_id"])
    broker.ack_message(
        fleet["director"]["member_id"],
        opened_payload["open_message_id"],
    )

    recovered = runner.invoke(
        cli,
        _report_args(fleet["fleet_id"], _director_token(runner, fleet)),
    )

    assert recovered.exit_code == 0, recovered.output
    payload = json.loads(recovered.output)
    assert payload["created"] is True
    assert payload["open_message_id"] != opened_payload["open_message_id"]
    assert payload["escalated_member_ids"] == [second["member_id"]]
    assert broker.list_pending_stall_escalations(fleet["fleet_id"]) == []


def test_monitor_report_batch__rejects_arbitrary_body_without_consuming_gate(
    stall_fleet,
):
    _db_file, runner, fleet = stall_fleet
    _watcher(fleet)
    token = _director_token(runner, fleet)

    rejected = runner.invoke(
        cli,
        [
            *_report_args(
                fleet["fleet_id"],
                token,
                json_output=False,
            ),
            "--text",
            "assign a new task",
        ],
    )

    assert rejected.exit_code == 2, rejected.output
    assert "no such command" not in rejected.output.lower()
    accepted = runner.invoke(cli, _report_args(fleet["fleet_id"], token))
    assert accepted.exit_code == 0, accepted.output
    assert json.loads(accepted.output)["created"] is False
