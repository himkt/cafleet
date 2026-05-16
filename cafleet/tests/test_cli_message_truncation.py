"""Per-command tests for the ``--full`` truncation flag on ``cafleet message *``.

Per principle (iii) of design 0000061: per-command + per-mode +
per-default/full fragmentation collapses to a small set of parametrized
matrix tests covering ``poll``, ``show``, ``send`` (truncated) and
``broadcast`` (verbatim).
"""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli

LONG_BODY = "x" * 300
TRUNCATED_BODY = "x" * 200 + "…"
SUMMARY_TEXT = "Broadcast sent to 3 recipients"


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def other_agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def task_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def _stub_verify(monkeypatch):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *_a, **_k: True)


def _task_payload(task_id, *, sender, recipient, text, type_="unicast"):
    return {
        "task_id": task_id,
        "context_id": recipient,
        "from_agent_id": sender,
        "to_agent_id": recipient,
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
                "from_agent_id": sender,
                "to_agent_id": "",
                "type": "broadcast_summary",
                "created_at": "2026-05-01T00:00:00+00:00",
                "status_state": "completed",
                "status_timestamp": "2026-05-01T00:00:00+00:00",
                "origin_task_id": task_id,
                "text": SUMMARY_TEXT,
            },
            "notifications_sent_count": count,
        }
    ]


def _setup_subcommand(
    monkeypatch, subcommand, session_id, agent_id, other_agent_id, task_id
):
    """Patch the broker call and return the CLI argv prefix for the chosen subcommand."""
    if subcommand == "poll":
        monkeypatch.setattr(
            broker,
            "poll_tasks",
            lambda *_a, **_k: [
                _task_payload(
                    "t-1", sender="from-1", recipient=agent_id, text=LONG_BODY
                )
            ],
        )
        return ["message", "poll", "--agent-id", agent_id], "list"
    if subcommand == "show":
        monkeypatch.setattr(
            broker,
            "get_task",
            lambda *_a, **_k: {
                "task": _task_payload(
                    task_id, sender=other_agent_id, recipient=agent_id, text=LONG_BODY
                )
            },
        )
        return [
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
        ], "envelope"
    # send
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=agent_id, recipient=other_agent_id, text=LONG_BODY
            )
        },
    )
    return [
        "message",
        "send",
        "--agent-id",
        agent_id,
        "--to",
        other_agent_id,
        "--text",
        LONG_BODY,
    ], "envelope"


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
@pytest.mark.parametrize("full", [False, True])
def test_truncation__poll_show_send_text_output(
    runner, session_id, agent_id, other_agent_id, task_id, monkeypatch, subcommand, full
):
    args, _shape = _setup_subcommand(
        monkeypatch, subcommand, session_id, agent_id, other_agent_id, task_id
    )
    full_args = args + (["--full"] if full else [])
    result = runner.invoke(cli, ["--session-id", session_id, *full_args])
    assert result.exit_code == 0, result.output
    if full:
        assert LONG_BODY in result.output
    else:
        assert TRUNCATED_BODY in result.output
        assert LONG_BODY not in result.output


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
@pytest.mark.parametrize("full", [False, True])
def test_truncation__poll_show_send_json_output(
    runner, session_id, agent_id, other_agent_id, task_id, monkeypatch, subcommand, full
):
    args, shape = _setup_subcommand(
        monkeypatch, subcommand, session_id, agent_id, other_agent_id, task_id
    )
    full_args = args + (["--full"] if full else [])
    result = runner.invoke(cli, ["--session-id", session_id, "--json", *full_args])
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    task = payload[0] if shape == "list" else payload["task"]
    expected = LONG_BODY if full else TRUNCATED_BODY
    assert task["text"] == expected


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
def test_truncation__non_text_fields_byte_identical_between_default_and_full(
    runner, session_id, agent_id, other_agent_id, task_id, monkeypatch, subcommand
):
    args, shape = _setup_subcommand(
        monkeypatch, subcommand, session_id, agent_id, other_agent_id, task_id
    )
    default_res = runner.invoke(cli, ["--session-id", session_id, "--json", *args])
    full_res = runner.invoke(
        cli, ["--session-id", session_id, "--json", *args, "--full"]
    )
    assert default_res.exit_code == 0, default_res.output
    assert full_res.exit_code == 0, full_res.output
    if shape == "list":
        default_task = json.loads(default_res.output)[0]
        full_task = json.loads(full_res.output)[0]
    else:
        default_task = json.loads(default_res.output)["task"]
        full_task = json.loads(full_res.output)["task"]
    assert default_task["id"] == full_task["task_id"][:8]
    assert default_task["from"] == full_task["from_agent_id"][:8]
    assert default_task["ts"] == full_task["status_timestamp"]


@pytest.mark.parametrize("output_mode", ["text", "json"])
@pytest.mark.parametrize("full", [False, True])
def test_truncation__broadcast_summary_emitted_verbatim(
    runner, session_id, agent_id, task_id, monkeypatch, output_mode, full
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(task_id, sender=agent_id, count=3),
    )
    args = ["message", "broadcast", "--agent-id", agent_id, "--text", LONG_BODY]
    if full:
        args.append("--full")
    if output_mode == "json":
        result = runner.invoke(cli, ["--session-id", session_id, "--json", *args])
    else:
        result = runner.invoke(cli, ["--session-id", session_id, *args])
    assert result.exit_code == 0, result.output
    if output_mode == "json":
        payload = json.loads(result.output)
        assert len(payload) == 1
        # Summary text fits inside default 200-codepoint limit → identical for both.
        assert payload[0]["task"]["text"] == SUMMARY_TEXT
        # notifications_sent_count preserved in JSON.
        assert payload[0]["notifications_sent_count"] == 3
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
    runner, session_id, agent_id, monkeypatch
):
    bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _task_payload(
                f"t-{i}", sender=f"from-{i}", recipient=agent_id, text=bodies[i]
            )
            for i in range(3)
        ],
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "poll",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert len(payload) == 3
    for item in payload:
        assert item["text"] == TRUNCATED_BODY
