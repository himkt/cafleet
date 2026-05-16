"""CLI integration tests for ``--pretty`` flag + ``_client_command`` wiring.

Per principle (iii) of design 0000061: per-flag fragmentation collapses to
parametrized "shape + behaviour" tests.
"""

import json
import re
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli

LONG_BODY = "x" * 300
TRUNCATED_BODY = "x" * 200 + "…"


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def _stub_verify(monkeypatch):
    monkeypatch.setattr(broker, "verify_agent_session", lambda *_a, **_k: True)


def _typed_task(
    *,
    task_id: str = "abcdef0123456789-tail",
    sender: str = "ffffffff11112222-tail",
    recipient: str = "rrrrrrrr11112222-tail",
    text: str = LONG_BODY,
    type_: str = "unicast",
    origin_task_id: str | None = None,
) -> dict:
    return {
        "task_id": task_id,
        "context_id": recipient,
        "from_agent_id": sender,
        "to_agent_id": recipient,
        "type": type_,
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": origin_task_id,
        "text": text,
    }


@pytest.mark.parametrize(
    "scenario",
    ["default_off_compact", "explicit_on_indented", "can_precede_session_id", "listed_in_root_help"],
)
def test_pretty_flag(runner, session_id, agent_id, monkeypatch, scenario):
    if scenario == "listed_in_root_help":
        result = runner.invoke(cli, ["--help"])
        assert result.exit_code == 0
        assert "--pretty" in result.output
        return

    monkeypatch.setattr(broker, "poll_tasks", lambda *_a, **_k: [_typed_task(text="short")])
    if scenario == "default_off_compact":
        args = ["--session-id", session_id, "--json", "message", "poll", "--agent-id", agent_id]
    elif scenario == "explicit_on_indented":
        args = [
            "--session-id", session_id, "--json", "--pretty",
            "message", "poll", "--agent-id", agent_id,
        ]
    else:  # can_precede_session_id
        args = [
            "--pretty", "--session-id", session_id, "--json",
            "message", "poll", "--agent-id", agent_id,
        ]

    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    payload = result.output.strip()
    parsed = json.loads(payload)
    assert isinstance(parsed, list)
    if scenario == "default_off_compact":
        assert "\n" not in payload
        assert ", " not in payload
        assert ": " not in payload
    else:
        assert "\n" in payload
        assert re.search(r"\n  ", payload)


@pytest.mark.parametrize("subcommand", ["poll", "show", "send"])
def test_compact_json__envelope_shape(
    runner, session_id, agent_id, monkeypatch, subcommand
):
    if subcommand == "poll":
        monkeypatch.setattr(
            broker, "poll_tasks",
            lambda *_a, **_k: [
                _typed_task(
                    task_id="abcdef0123456789-tail-stuff-here",
                    sender="zyxwvutsrqponmlk-tail-stuff",
                )
            ],
        )
        args = ["message", "poll", "--agent-id", agent_id]
    elif subcommand == "show":
        monkeypatch.setattr(
            broker, "get_task",
            lambda *_a, **_k: {
                "task": _typed_task(task_id="abcdef0123456789-tail-stuff-here")
            },
        )
        args = [
            "message", "show", "--agent-id", agent_id,
            "--task-id", "abcdef0123456789-tail",
        ]
    else:  # send
        monkeypatch.setattr(
            broker, "send_message",
            lambda *_a, **_k: {
                "task": _typed_task(task_id="abcdef0123456789-tail-stuff-here"),
                "notification_sent": True,
            },
        )
        args = [
            "message", "send", "--agent-id", agent_id,
            "--to", str(uuid.uuid4()), "--text", "ignored",
        ]

    result = runner.invoke(cli, ["--session-id", session_id, "--json", *args])
    assert result.exit_code == 0, result.output
    parsed = json.loads(result.output)
    if subcommand == "poll":
        rendered = parsed[0]
    else:
        rendered = parsed.get("task", parsed)
    assert set(rendered.keys()).issubset({"id", "from", "ts", "text", "kind", "origin"})
    assert rendered["id"] == "abcdef01"
    for forbidden in (
        "task_id", "context_id", "from_agent_id", "to_agent_id",
        "status_state", "created_at", "origin_task_id", "type",
    ):
        assert forbidden not in rendered


@pytest.mark.parametrize(
    "scenario",
    ["restores_typed_columns", "disables_body_truncation"],
)
def test_full_flag__restores_long_form(
    runner, session_id, agent_id, monkeypatch, scenario
):
    monkeypatch.setattr(
        broker, "poll_tasks",
        lambda *_a, **_k: [_typed_task(task_id="abcdef0123456789-tail", text=LONG_BODY)],
    )
    result = runner.invoke(
        cli,
        [
            "--session-id", session_id, "--json", "message", "poll",
            "--agent-id", agent_id, "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    [rendered] = json.loads(result.output)
    if scenario == "restores_typed_columns":
        assert rendered["task_id"] == "abcdef0123456789-tail"
        assert "context_id" in rendered
        assert "to_agent_id" in rendered
        assert "status_state" in rendered
    else:
        assert rendered["text"] == LONG_BODY


def test_default_mode__body_truncation_under_compact_envelope(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker, "poll_tasks",
        lambda *_a, **_k: [_typed_task(text=LONG_BODY)],
    )
    result = runner.invoke(
        cli,
        [
            "--session-id", session_id, "--json", "message", "poll",
            "--agent-id", agent_id,
        ],
    )
    assert result.exit_code == 0, result.output
    [rendered] = json.loads(result.output)
    assert rendered["text"] == TRUNCATED_BODY


@pytest.mark.parametrize("scenario", ["default_two_lines", "full_legacy_verbose"])
def test_text_mode(runner, session_id, agent_id, monkeypatch, scenario):
    monkeypatch.setattr(
        broker, "poll_tasks",
        lambda *_a, **_k: [_typed_task(text="hello world body")],
    )
    args = ["--session-id", session_id, "message", "poll", "--agent-id", agent_id]
    if scenario == "full_legacy_verbose":
        args.append("--full")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    if scenario == "default_two_lines":
        assert "hello world body" in result.output
        assert re.search(r"\[abcdef01 \| from:[a-zA-Z0-9]{8} \| 2026-", result.output)
    else:
        for needle in ("id:", "state:", "from:", "to:", "type:", "text:"):
            assert needle in result.output
