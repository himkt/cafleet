"""Per-command tests for the ``--full`` truncation flag on ``cafleet message *``.

Helper-level tests in ``test_output.py`` already cover ``truncate_text`` and
``truncate_task_text``. These tests verify the user-facing message subcommands
exercise truncation correctly:

- ``message poll`` / ``message show`` / ``message send`` truncate the body to
  10 codepoints + ``...`` by default and emit the full body when ``--full`` is
  supplied. Tests assert the regression guard from the design doc: non-text
  fields (``id``, ``status.state``, ``metadata.fromAgentId``,
  ``metadata.toAgentId``, ``metadata.type``) are byte-identical between the
  two modes, proving the helper does not mutate siblings of ``part['text']``.
- ``message broadcast`` is intentionally exempt. The broker returns a single
  ``broadcast_summary`` envelope whose text is generated server-side and
  carries no user-supplied body — truncating it would only obscure
  operator-relevant detail (notifications-sent count, etc.). Its tests assert
  the summary text passes through verbatim and ``notifications_sent_count``
  is preserved in ``--json`` output, regardless of ``--full``.

``message ack`` and ``message cancel`` reuse the same ``_client_command``
wiring as ``message send`` / ``message show`` — the design doc Step 4 last
bullet explicitly skips per-command tests for them; the helper-level tests
cover the truncate_task_text behavior on the same task shape.
"""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli

LONG_BODY = "abcdefghijklmnopqrstuvwxyz"
TRUNCATED_BODY = "abcdefghij..."


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
        "id": task_id,
        "status": {"state": "input_required", "timestamp": "2026-05-01T00:00:00+00:00"},
        "metadata": {
            "fromAgentId": sender,
            "toAgentId": recipient,
            "type": type_,
        },
        "artifacts": [{"parts": [{"text": text}]}],
    }


def test_message_poll_truncation__default_truncates_text_in_text_output(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _task_payload("t-1", sender="from-1", recipient=agent_id, text=LONG_BODY)
        ],
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "poll",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert TRUNCATED_BODY in result.output
    assert LONG_BODY not in result.output


def test_message_poll_truncation__full_emits_full_text_in_text_output(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _task_payload("t-1", sender="from-1", recipient=agent_id, text=LONG_BODY)
        ],
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "poll",
            "--agent-id",
            agent_id,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    assert LONG_BODY in result.output
    assert TRUNCATED_BODY not in result.output


def test_message_poll_truncation__default_truncates_text_in_json_output(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _task_payload("t-1", sender="from-1", recipient=agent_id, text=LONG_BODY)
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
    assert payload[0]["artifacts"][0]["parts"][0]["text"] == TRUNCATED_BODY


def test_message_poll_truncation__full_emits_full_text_in_json_output(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _task_payload("t-1", sender="from-1", recipient=agent_id, text=LONG_BODY)
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
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload[0]["artifacts"][0]["parts"][0]["text"] == LONG_BODY


def test_message_poll_truncation__empty_inbox_unchanged_by_full_flag(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(broker, "poll_tasks", lambda *_a, **_k: [])
    default = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "poll",
            "--agent-id",
            agent_id,
        ],
    )
    full = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "poll",
            "--agent-id",
            agent_id,
            "--full",
        ],
    )
    assert default.exit_code == 0, default.output
    assert full.exit_code == 0, full.output
    assert default.output == full.output


def test_message_poll_truncation__list_of_three_tasks_each_truncated(
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
        assert item["artifacts"][0]["parts"][0]["text"] == TRUNCATED_BODY


def test_message_poll_truncation__non_text_fields_byte_identical_between_default_and_full(
    runner, session_id, agent_id, monkeypatch
):
    def fresh_payload():
        return [
            _task_payload("t-1", sender="from-1", recipient=agent_id, text=LONG_BODY)
        ]

    monkeypatch.setattr(broker, "poll_tasks", lambda *_a, **_k: fresh_payload())
    default_res = runner.invoke(
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
    full_res = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "poll",
            "--agent-id",
            agent_id,
            "--full",
        ],
    )
    assert default_res.exit_code == 0, default_res.output
    assert full_res.exit_code == 0, full_res.output

    default_task = json.loads(default_res.output)[0]
    full_task = json.loads(full_res.output)[0]
    assert default_task["id"] == full_task["id"]
    assert default_task["status"]["state"] == full_task["status"]["state"]
    assert (
        default_task["metadata"]["fromAgentId"] == full_task["metadata"]["fromAgentId"]
    )
    assert default_task["metadata"]["toAgentId"] == full_task["metadata"]["toAgentId"]
    assert default_task["metadata"]["type"] == full_task["metadata"]["type"]


def test_message_show_truncation__default_truncates_text_in_text_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_task",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=other_agent_id, recipient=agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert TRUNCATED_BODY in result.output
    assert LONG_BODY not in result.output


def test_message_show_truncation__full_emits_full_text_in_text_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_task",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=other_agent_id, recipient=agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    assert LONG_BODY in result.output


def test_message_show_truncation__default_truncates_text_in_json_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_task",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=other_agent_id, recipient=agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["task"]["artifacts"][0]["parts"][0]["text"] == TRUNCATED_BODY


def test_message_show_truncation__full_emits_full_text_in_json_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_task",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=other_agent_id, recipient=agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["task"]["artifacts"][0]["parts"][0]["text"] == LONG_BODY


def test_message_show_truncation__non_text_fields_byte_identical_between_default_and_full(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    def fresh_payload():
        return {
            "task": _task_payload(
                task_id, sender=other_agent_id, recipient=agent_id, text=LONG_BODY
            )
        }

    monkeypatch.setattr(broker, "get_task", lambda *_a, **_k: fresh_payload())
    default_res = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
        ],
    )
    full_res = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "show",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
            "--full",
        ],
    )
    assert default_res.exit_code == 0, default_res.output
    assert full_res.exit_code == 0, full_res.output

    default_task = json.loads(default_res.output)["task"]
    full_task = json.loads(full_res.output)["task"]
    assert default_task["id"] == full_task["id"]
    assert default_task["status"]["state"] == full_task["status"]["state"]
    assert (
        default_task["metadata"]["fromAgentId"] == full_task["metadata"]["fromAgentId"]
    )
    assert default_task["metadata"]["toAgentId"] == full_task["metadata"]["toAgentId"]
    assert default_task["metadata"]["type"] == full_task["metadata"]["type"]


def test_message_send_truncation__default_truncates_echo_in_text_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=agent_id, recipient=other_agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "send",
            "--agent-id",
            agent_id,
            "--to",
            other_agent_id,
            "--text",
            LONG_BODY,
        ],
    )
    assert result.exit_code == 0, result.output
    assert "Message sent." in result.output
    assert TRUNCATED_BODY in result.output
    assert LONG_BODY not in result.output


def test_message_send_truncation__full_emits_full_echo_in_text_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=agent_id, recipient=other_agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "send",
            "--agent-id",
            agent_id,
            "--to",
            other_agent_id,
            "--text",
            LONG_BODY,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    assert LONG_BODY in result.output


def test_message_send_truncation__default_truncates_echo_in_json_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=agent_id, recipient=other_agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "send",
            "--agent-id",
            agent_id,
            "--to",
            other_agent_id,
            "--text",
            LONG_BODY,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["task"]["artifacts"][0]["parts"][0]["text"] == TRUNCATED_BODY


def test_message_send_truncation__full_emits_full_echo_in_json_output(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _task_payload(
                task_id, sender=agent_id, recipient=other_agent_id, text=LONG_BODY
            )
        },
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "send",
            "--agent-id",
            agent_id,
            "--to",
            other_agent_id,
            "--text",
            LONG_BODY,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["task"]["artifacts"][0]["parts"][0]["text"] == LONG_BODY


def test_message_send_truncation__non_text_fields_byte_identical_between_default_and_full(
    runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
):
    def fresh_payload():
        return {
            "task": _task_payload(
                task_id, sender=agent_id, recipient=other_agent_id, text=LONG_BODY
            )
        }

    monkeypatch.setattr(broker, "send_message", lambda *_a, **_k: fresh_payload())
    common = [
        "--json",
        "message",
        "send",
        "--agent-id",
        agent_id,
        "--to",
        other_agent_id,
        "--text",
        LONG_BODY,
    ]
    default_res = runner.invoke(cli, ["--session-id", session_id, *common])
    full_res = runner.invoke(cli, ["--session-id", session_id, *common, "--full"])
    assert default_res.exit_code == 0, default_res.output
    assert full_res.exit_code == 0, full_res.output

    default_task = json.loads(default_res.output)["task"]
    full_task = json.loads(full_res.output)["task"]
    assert default_task["id"] == full_task["id"]
    assert default_task["status"]["state"] == full_task["status"]["state"]
    assert (
        default_task["metadata"]["fromAgentId"] == full_task["metadata"]["fromAgentId"]
    )
    assert default_task["metadata"]["toAgentId"] == full_task["metadata"]["toAgentId"]
    assert default_task["metadata"]["type"] == full_task["metadata"]["type"]


SUMMARY_TEXT = "Broadcast sent to 3 recipients"


def _broadcast_summary_payload(task_id, *, sender, count):
    """Single-element envelope list mirroring ``broker.broadcast_message``.

    The real broker does not return a per-recipient task list — it returns a
    single ``broadcast_summary`` task plus a sibling ``notifications_sent_count``
    on the envelope. The summary text is intentionally longer than the
    truncation limit (~30 codepoints) so any accidental truncation would
    surface in these tests.
    """
    return [
        {
            "task": {
                "id": task_id,
                "status": {
                    "state": "completed",
                    "timestamp": "2026-05-01T00:00:00+00:00",
                },
                "metadata": {
                    "fromAgentId": sender,
                    "type": "broadcast_summary",
                    "notificationsSentCount": count,
                },
                "artifacts": [{"parts": [{"text": SUMMARY_TEXT}]}],
            },
            "notifications_sent_count": count,
        }
    ]


# --- message_broadcast_no_truncation: ``message broadcast`` returns a
# ``broadcast_summary`` envelope, not a list of per-recipient delivery tasks.
# The summary text is generated by the broker and carries no user-supplied
# body, so truncating it would only obscure operator-relevant detail
# (notifications-sent count, etc.). The subcommand therefore disables
# ``truncates_task_text`` and the ``--full`` flag is a no-op here. ---


def test_message_broadcast_no_truncation__summary_text_emitted_verbatim_in_text_output(
    runner, session_id, agent_id, task_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(task_id, sender=agent_id, count=3),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            LONG_BODY,
        ],
    )
    assert result.exit_code == 0, result.output
    assert "Broadcast sent." in result.output
    assert SUMMARY_TEXT in result.output
    assert TRUNCATED_BODY not in result.output


def test_message_broadcast_no_truncation__summary_text_emitted_verbatim_with_full_in_text_output(
    runner, session_id, agent_id, task_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(task_id, sender=agent_id, count=3),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            LONG_BODY,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    assert SUMMARY_TEXT in result.output


def test_message_broadcast_no_truncation__summary_text_emitted_verbatim_in_json_output(
    runner, session_id, agent_id, task_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(task_id, sender=agent_id, count=3),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            LONG_BODY,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert len(payload) == 1
    assert payload[0]["task"]["artifacts"][0]["parts"][0]["text"] == SUMMARY_TEXT


def test_message_broadcast_no_truncation__summary_text_emitted_verbatim_with_full_in_json_output(
    runner, session_id, agent_id, task_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(task_id, sender=agent_id, count=3),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            LONG_BODY,
            "--full",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload[0]["task"]["artifacts"][0]["parts"][0]["text"] == SUMMARY_TEXT


def test_message_broadcast_no_truncation__notifications_sent_count_preserved_in_json_output(
    runner, session_id, agent_id, task_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "broadcast_message",
        lambda *_a, **_k: _broadcast_summary_payload(task_id, sender=agent_id, count=7),
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "broadcast",
            "--agent-id",
            agent_id,
            "--text",
            LONG_BODY,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload[0]["notifications_sent_count"] == 7
    assert payload[0]["task"]["metadata"]["notificationsSentCount"] == 7


def test_message_broadcast_no_truncation__default_and_full_json_output_byte_identical(
    runner, session_id, agent_id, task_id, monkeypatch
):
    def fresh_payload():
        return _broadcast_summary_payload(task_id, sender=agent_id, count=5)

    monkeypatch.setattr(broker, "broadcast_message", lambda *_a, **_k: fresh_payload())
    common = [
        "--json",
        "message",
        "broadcast",
        "--agent-id",
        agent_id,
        "--text",
        LONG_BODY,
    ]
    default_res = runner.invoke(cli, ["--session-id", session_id, *common])
    full_res = runner.invoke(cli, ["--session-id", session_id, *common, "--full"])
    assert default_res.exit_code == 0, default_res.output
    assert full_res.exit_code == 0, full_res.output
    assert default_res.output == full_res.output
