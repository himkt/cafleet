"""Per-command tests for the ``--full`` truncation flag on ``cafleet message *``.

Helper-level tests in ``test_output.py`` already cover ``truncate_text`` and
``truncate_task_text``. These tests verify the four user-facing message
subcommands explicitly enumerated in design 0000043 Step 4 — ``message poll``,
``message show``, ``message send``, ``message broadcast`` — exercise the
truncation in both default and ``--full`` modes, and in both text and ``--json``
output. Each test also asserts the regression guard from the design doc: the
non-text fields (``id``, ``status.state``, ``metadata.fromAgentId``,
``metadata.toAgentId``, ``metadata.type``) are byte-identical between the two
modes, proving the helper does not mutate siblings of ``part['text']``.

``message ack`` and ``message cancel`` reuse the same ``_client_command``
wiring as ``message send`` and ``message show`` — the design doc Step 4 last
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


def _run(runner, args, *, json_output: bool):
    base = ["--session-id", args["session_id"]]
    if json_output:
        base.append("--json")
    return runner.invoke(cli, base + args["cmd"])


class TestMessagePollTruncation:
    def test_default_truncates_text_in_text_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "poll_tasks",
            lambda *_a, **_k: [
                _task_payload(
                    "t-1", sender="from-1", recipient=agent_id, text=LONG_BODY
                )
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

    def test_full_emits_full_text_in_text_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "poll_tasks",
            lambda *_a, **_k: [
                _task_payload(
                    "t-1", sender="from-1", recipient=agent_id, text=LONG_BODY
                )
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

    def test_default_truncates_text_in_json_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "poll_tasks",
            lambda *_a, **_k: [
                _task_payload(
                    "t-1", sender="from-1", recipient=agent_id, text=LONG_BODY
                )
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

    def test_full_emits_full_text_in_json_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "poll_tasks",
            lambda *_a, **_k: [
                _task_payload(
                    "t-1", sender="from-1", recipient=agent_id, text=LONG_BODY
                )
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

    def test_empty_inbox_unchanged_by_full_flag(
        self, runner, session_id, agent_id, monkeypatch
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

    def test_list_of_three_tasks_each_truncated(
        self, runner, session_id, agent_id, monkeypatch
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

    def test_non_text_fields_byte_identical_between_default_and_full(
        self, runner, session_id, agent_id, monkeypatch
    ):
        def fresh_payload():
            return [
                _task_payload(
                    "t-1", sender="from-1", recipient=agent_id, text=LONG_BODY
                )
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
            default_task["metadata"]["fromAgentId"]
            == full_task["metadata"]["fromAgentId"]
        )
        assert (
            default_task["metadata"]["toAgentId"] == full_task["metadata"]["toAgentId"]
        )
        assert default_task["metadata"]["type"] == full_task["metadata"]["type"]


class TestMessageShowTruncation:
    def test_default_truncates_text_in_text_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_full_emits_full_text_in_text_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_default_truncates_text_in_json_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_full_emits_full_text_in_json_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_non_text_fields_byte_identical_between_default_and_full(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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
            default_task["metadata"]["fromAgentId"]
            == full_task["metadata"]["fromAgentId"]
        )
        assert (
            default_task["metadata"]["toAgentId"] == full_task["metadata"]["toAgentId"]
        )
        assert default_task["metadata"]["type"] == full_task["metadata"]["type"]


class TestMessageSendTruncation:
    def test_default_truncates_echo_in_text_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_full_emits_full_echo_in_text_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_default_truncates_echo_in_json_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_full_emits_full_echo_in_json_output(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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

    def test_non_text_fields_byte_identical_between_default_and_full(
        self, runner, session_id, agent_id, task_id, other_agent_id, monkeypatch
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
            default_task["metadata"]["fromAgentId"]
            == full_task["metadata"]["fromAgentId"]
        )
        assert (
            default_task["metadata"]["toAgentId"] == full_task["metadata"]["toAgentId"]
        )
        assert default_task["metadata"]["type"] == full_task["metadata"]["type"]


class TestMessageBroadcastTruncation:
    def test_default_truncates_each_in_text_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]
        monkeypatch.setattr(
            broker,
            "broadcast_message",
            lambda *_a, **_k: [
                _task_payload(
                    f"t-{i}",
                    sender=agent_id,
                    recipient=f"recipient-{i}",
                    text=bodies[i],
                    type_="broadcast",
                )
                for i in range(3)
            ],
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
        assert result.output.count(TRUNCATED_BODY) == 3
        assert LONG_BODY not in result.output

    def test_full_emits_full_bodies_in_text_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]
        monkeypatch.setattr(
            broker,
            "broadcast_message",
            lambda *_a, **_k: [
                _task_payload(
                    f"t-{i}",
                    sender=agent_id,
                    recipient=f"recipient-{i}",
                    text=bodies[i],
                    type_="broadcast",
                )
                for i in range(3)
            ],
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
        for body in bodies:
            assert body in result.output

    def test_default_truncates_each_in_json_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]
        monkeypatch.setattr(
            broker,
            "broadcast_message",
            lambda *_a, **_k: [
                _task_payload(
                    f"t-{i}",
                    sender=agent_id,
                    recipient=f"recipient-{i}",
                    text=bodies[i],
                    type_="broadcast",
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
                "broadcast",
                "--agent-id",
                agent_id,
                "--text",
                LONG_BODY,
            ],
        )
        assert result.exit_code == 0, result.output
        payload = json.loads(result.output)
        assert len(payload) == 3
        for item in payload:
            assert item["artifacts"][0]["parts"][0]["text"] == TRUNCATED_BODY

    def test_full_emits_full_bodies_in_json_output(
        self, runner, session_id, agent_id, monkeypatch
    ):
        bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]
        monkeypatch.setattr(
            broker,
            "broadcast_message",
            lambda *_a, **_k: [
                _task_payload(
                    f"t-{i}",
                    sender=agent_id,
                    recipient=f"recipient-{i}",
                    text=bodies[i],
                    type_="broadcast",
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
        assert len(payload) == 3
        for i, item in enumerate(payload):
            assert item["artifacts"][0]["parts"][0]["text"] == bodies[i]

    def test_non_text_fields_byte_identical_between_default_and_full(
        self, runner, session_id, agent_id, monkeypatch
    ):
        bodies = [LONG_BODY + "X", LONG_BODY + "Y", LONG_BODY + "Z"]

        def fresh_payload():
            return [
                _task_payload(
                    f"t-{i}",
                    sender=agent_id,
                    recipient=f"recipient-{i}",
                    text=bodies[i],
                    type_="broadcast",
                )
                for i in range(3)
            ]

        monkeypatch.setattr(
            broker, "broadcast_message", lambda *_a, **_k: fresh_payload()
        )
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

        default_payload = json.loads(default_res.output)
        full_payload = json.loads(full_res.output)
        assert len(default_payload) == len(full_payload) == 3
        for d, f in zip(default_payload, full_payload, strict=True):
            assert d["id"] == f["id"]
            assert d["status"]["state"] == f["status"]["state"]
            assert d["metadata"]["fromAgentId"] == f["metadata"]["fromAgentId"]
            assert d["metadata"]["toAgentId"] == f["metadata"]["toAgentId"]
            assert d["metadata"]["type"] == f["metadata"]["type"]
