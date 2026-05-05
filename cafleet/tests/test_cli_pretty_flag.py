"""CLI integration tests for the Surface 1 ``--pretty`` flag and the
``_client_command`` envelope-slim + body-truncation wiring (design 0000049 Step 3).

Exercises:

- New global ``--pretty`` flag at the ``cafleet`` root group.
- Default JSON mode is **compact** (no whitespace separators / newlines).
- ``--pretty --json`` switches to indented JSON.
- Text mode is the new 2-line-per-task render unless ``--full`` is passed.
- ``--full`` disables BOTH the envelope-slim (so verbose layout returns) AND
  the body-truncation (so the full message body emits) — one flag, both
  toggles, per Surface 1 Step 3 success criterion.
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


# ---------------------------------------------------------------------------
# Global --pretty flag
# ---------------------------------------------------------------------------


def test_pretty_flag_exists__cli_help_lists_pretty_at_root(runner):
    result = runner.invoke(cli, ["--help"])
    assert result.exit_code == 0
    assert "--pretty" in result.output


def test_pretty_flag_default_off__json_default_is_compact_no_whitespace(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(text="short")],
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
    payload = result.output.strip()
    parsed = json.loads(payload)
    assert isinstance(parsed, list)
    assert "\n" not in payload
    assert ", " not in payload
    assert ": " not in payload


def test_pretty_flag_on__json_with_pretty_uses_indented_form(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(text="short")],
    )
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "--pretty",
            "message",
            "poll",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 0, result.output
    payload = result.output.strip()
    json.loads(payload)  # still valid JSON
    assert "\n" in payload
    # ``json.dumps(indent=2)`` emits two-space indentation
    assert re.search(r"\n  ", payload)


def test_pretty_flag_position__pretty_can_precede_session_id(
    runner, session_id, agent_id, monkeypatch
):
    """``--pretty`` is a global option; it must work in either order at the root."""
    monkeypatch.setattr(broker, "poll_tasks", lambda *_a, **_k: [_typed_task()])
    result = runner.invoke(
        cli,
        [
            "--pretty",
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
    assert "\n" in result.output


# ---------------------------------------------------------------------------
# Compact rendered envelope in JSON output
# ---------------------------------------------------------------------------


def test_compact_json__poll_output_has_only_compact_keys(
    runner, session_id, agent_id, monkeypatch
):
    """Default JSON poll output must contain only the compact-render keys."""
    monkeypatch.setattr(broker, "poll_tasks", lambda *_a, **_k: [_typed_task()])
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
    parsed = json.loads(result.output)
    assert isinstance(parsed, list)
    assert parsed
    rendered = parsed[0]
    assert set(rendered.keys()).issubset({"id", "from", "ts", "text", "kind", "origin"})
    # Compact must NOT carry the long-form typed-column keys.
    for forbidden in (
        "task_id",
        "context_id",
        "from_agent_id",
        "to_agent_id",
        "status_state",
        "created_at",
        "origin_task_id",
        "type",
    ):
        assert forbidden not in rendered, (
            f"compact JSON should not include {forbidden!r}; "
            f"got keys: {sorted(rendered.keys())}"
        )


def test_compact_json__poll_output_uses_8_char_id_prefixes(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [
            _typed_task(
                task_id="abcdef0123456789-tail-stuff-here",
                sender="zyxwvutsrqponmlk-tail-stuff",
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
    [rendered] = json.loads(result.output)
    assert rendered["id"] == "abcdef01"
    assert rendered["from"] == "zyxwvuts"


# ---------------------------------------------------------------------------
# --full restores the verbose envelope in JSON mode
# ---------------------------------------------------------------------------


def test_full_flag_in_json_mode__restores_typed_column_long_form(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(task_id="abcdef0123456789-tail")],
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
    [rendered] = json.loads(result.output)
    assert rendered["task_id"] == "abcdef0123456789-tail"
    assert "context_id" in rendered
    assert "to_agent_id" in rendered
    assert "status_state" in rendered


def test_full_flag_in_json_mode__also_disables_body_truncation(
    runner, session_id, agent_id, monkeypatch
):
    """One ``--full`` toggle drives BOTH envelope-slim AND body-truncation off."""
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(text=LONG_BODY)],
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
    [rendered] = json.loads(result.output)
    assert rendered["text"] == LONG_BODY


def test_default_mode__body_truncation_still_applies_under_compact_envelope(
    runner, session_id, agent_id, monkeypatch
):
    """Without ``--full``: compact envelope AND truncated body."""
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(text=LONG_BODY)],
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
    [rendered] = json.loads(result.output)
    assert rendered["text"] == TRUNCATED_BODY


# ---------------------------------------------------------------------------
# Text mode 2-line render
# ---------------------------------------------------------------------------


def test_text_mode_default__poll_output_is_two_lines_per_task(
    runner, session_id, agent_id, monkeypatch
):
    """Text mode renders 2 lines per task (line 1 envelope, line 2 body)."""
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(text="hello")],
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
    # Surface 17 dropped the legacy "[N]" indexed-list prefix; items are
    # now blank-line separated. Assert the body line "hello" and the
    # per-task envelope line ("[<id8> | from:<id8> | <ts>]") both appear.
    assert "hello" in result.output
    assert re.search(r"\[abcdef01 \| from:[a-zA-Z0-9]{8} \| 2026-", result.output), (
        f"compact envelope line missing; output was:\n{result.output}"
    )


def test_text_mode_full__poll_output_uses_legacy_verbose_layout(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "poll_tasks",
        lambda *_a, **_k: [_typed_task(text="hello world body")],
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
    # Legacy verbose layout has dedicated lines for id/state/from/to/type/text.
    for needle in ("id:", "state:", "from:", "to:", "type:", "text:"):
        assert needle in result.output, (
            f"legacy field label {needle!r} missing under --full; "
            f"output was:\n{result.output}"
        )


# ---------------------------------------------------------------------------
# Cross-subcommand sanity: message show + send obey the same toggle
# ---------------------------------------------------------------------------


def test_compact_json__message_show_default_returns_compact_envelope(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_task",
        lambda *_a, **_k: {
            "task": _typed_task(task_id="aaaaaaaa11112222-tail", text=LONG_BODY)
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
            "aaaaaaaa11112222-tail",
        ],
    )
    assert result.exit_code == 0, result.output
    parsed = json.loads(result.output)
    inner = parsed["task"] if isinstance(parsed, dict) and "task" in parsed else parsed
    assert "id" in inner
    assert "from" in inner
    assert "task_id" not in inner


def test_compact_json__message_send_default_returns_compact_envelope(
    runner, session_id, agent_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "send_message",
        lambda *_a, **_k: {
            "task": _typed_task(task_id="bbbbbbbb33334444-tail", text=LONG_BODY),
            "notification_sent": True,
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
            str(uuid.uuid4()),
            "--text",
            "ignored under stub",
        ],
    )
    assert result.exit_code == 0, result.output
    parsed = json.loads(result.output)
    assert isinstance(parsed, dict)
    inner = parsed.get("task", parsed)
    assert "id" in inner
    assert "task_id" not in inner
