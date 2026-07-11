"""Per-command tests for the ``--full`` truncation flag on ``cafleet message *``."""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli

LONG_BODY = "x" * 300
TRUNCATED_BODY = "x" * 200 + "…"
SUMMARY_TEXT = "Broadcast sent to 3 recipients"


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture
def member_id():
    return 200


@pytest.fixture
def other_member_id():
    return 300


@pytest.fixture
def task_id():
    return 400


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def _stub_verify(monkeypatch):
    monkeypatch.setattr(broker, "verify_member_fleet", lambda *_a, **_k: True)


def _task_payload(task_id, *, sender, recipient, text, type_="unicast"):
    return {
        "task_id": task_id,
        "context_id": recipient,
        "from_member_id": sender,
        "to_member_id": recipient,
        "type": type_,
        "created_at": "2026-05-01T00:00:00+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-01T00:00:00+00:00",
        "origin_task_id": None,
        "text": text,
    }


def _broadcast_summary_payload(task_id, *, sender, count):
    return [
        {
            "task": {
                "task_id": task_id,
                "context_id": sender,
                "from_member_id": sender,
                "to_member_id": None,
                "type": "broadcast_summary",
                "created_at": "2026-05-01T00:00:00+00:00",
                "status_state": "completed",
                "status_timestamp": "2026-05-01T00:00:00+00:00",
                "origin_task_id": task_id,
                "text": SUMMARY_TEXT,
            },
            "recipients": count,
            "delivered": count,
        }
    ]


def _setup_subcommand(
    monkeypatch, subcommand, fleet_id, member_id, other_member_id, task_id
):
    """Patch the broker call and return the CLI argv prefix for the chosen subcommand."""
    if subcommand == "poll":
        monkeypatch.setattr(
            broker,
            "poll_tasks",
            lambda *_a, **_k: [
                _task_payload(1, sender=11, recipient=member_id, text=LONG_BODY)
            ],
        )
        return (
            [
                "message",
                "poll",
                "--fleet-id",
                str(fleet_id),
                "--member-id",
                str(member_id),
            ],
            "list",
        )
    if subcommand == "show":
        monkeypatch.setattr(
            broker,
            "get_task",
            lambda *_a, **_k: {
                "task": _task_payload(
                    task_id,
                    sender=other_member_id,
                    recipient=member_id,
                    text=LONG_BODY,
                )
            },
        )
        return [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
        ], "envelope"
    # send
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=member_id, recipient=other_member_id, text=LONG_BODY
            )
        },
    )
    return [
        "message",
        "send",
        "--fleet-id",
        str(fleet_id),
        "--from-member-id",
        str(member_id),
        "--to-member-id",
        str(other_member_id),
        "--text",
        LONG_BODY,
    ], "envelope"


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
@pytest.mark.parametrize("full", [False, True])
def test_truncation__poll_show_send_text_output(
    runner, fleet_id, member_id, other_member_id, task_id, monkeypatch, subcommand, full
):
    args, _shape = _setup_subcommand(
        monkeypatch, subcommand, fleet_id, member_id, other_member_id, task_id
    )
    full_args = args + (["--full"] if full else [])
    result = runner.invoke(cli, full_args)
    assert result.exit_code == 0, result.output
    if full:
        assert LONG_BODY in result.output
    else:
        assert TRUNCATED_BODY in result.output
        assert LONG_BODY not in result.output


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
@pytest.mark.parametrize("full", [False, True])
def test_truncation__poll_show_send_json_output(
    runner, fleet_id, member_id, other_member_id, task_id, monkeypatch, subcommand, full
):
    args, shape = _setup_subcommand(
        monkeypatch, subcommand, fleet_id, member_id, other_member_id, task_id
    )
    full_args = args + (["--full"] if full else [])
    result = runner.invoke(cli, [*full_args, "--json"])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    task = payload[0] if shape == "list" else payload["task"]
    expected = LONG_BODY if full else TRUNCATED_BODY
    assert task["text"] == expected


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
def test_truncation__non_text_fields_byte_identical_between_default_and_full(
    runner, fleet_id, member_id, other_member_id, task_id, monkeypatch, subcommand
):
    args, shape = _setup_subcommand(
        monkeypatch, subcommand, fleet_id, member_id, other_member_id, task_id
    )
    default_res = runner.invoke(cli, [*args, "--json"])
    full_res = runner.invoke(cli, [*args, "--full", "--json"])
    assert default_res.exit_code == 0, default_res.output
    assert full_res.exit_code == 0, full_res.output
    if shape == "list":
        default_task = json.loads(default_res.output)[0]
        full_task = json.loads(full_res.output)[0]
    else:
        default_task = json.loads(default_res.output)["task"]
        full_task = json.loads(full_res.output)["task"]
    assert default_task["id"] == full_task["task_id"]
    assert default_task["from"] == full_task["from_member_id"]
    assert default_task["ts"] == full_task["status_timestamp"]


@pytest.mark.parametrize("output_mode", ["text", "json"])
@pytest.mark.parametrize("full", [False, True])
def test_truncation__broadcast_summary_emitted_verbatim(
    runner, fleet_id, member_id, task_id, monkeypatch, output_mode, full
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(
            task_id, sender=member_id, count=3
        ),
    )
    args = [
        "message",
        "broadcast",
        "--fleet-id",
        str(fleet_id),
        "--from-member-id",
        str(member_id),
        "--text",
        LONG_BODY,
    ]
    if full:
        args.append("--full")
    if output_mode == "json":
        result = runner.invoke(cli, [*args, "--json"])
    else:
        result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    if output_mode == "json":
        payload = json.loads(result.output)
        assert len(payload) == 1
        # Summary text fits inside default 200-codepoint limit → identical for both.
        assert payload[0]["task"]["text"] == SUMMARY_TEXT
        # Split recipient/delivery counts preserved in JSON.
        assert payload[0]["recipients"] == 3
        assert payload[0]["delivered"] == 3
        if full:
            assert "task_id" in payload[0]["task"]
        else:
            assert "id" in payload[0]["task"]
    else:
        if full:
            assert SUMMARY_TEXT in result.output
        else:
            # Compact one-line summary with recipient count.
            assert "broadcast id=" in result.output
            assert "recipients=" in result.output


def test_truncation__poll_list_of_three_tasks_each_truncated(
    runner, fleet_id, member_id, monkeypatch
):
    bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _task_payload(
                f"t-{i}", sender=f"from-{i}", recipient=member_id, text=bodies[i]
            )
            for i in range(3)
        ],
    )
    result = runner.invoke(
        cli,
        [
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert len(payload) == 3
    for item in payload:
        assert item["text"] == TRUNCATED_BODY
