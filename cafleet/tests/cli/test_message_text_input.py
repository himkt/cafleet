"""``--text`` / ``--text-file`` input coverage for ``message send`` and
``message broadcast`` (design 0000112 §1, §3).

Both commands drop ``required=True`` on ``--text``, gain ``--text-file``, and
resolve the body through the shared ``read_text_input`` helper (no placeholder
substitution — that is ``member create``-only). The body is passed verbatim to
``broker.send_message`` / ``broker.broadcast_message``. Empty-body rejection is
uniform across inline / file / stdin.
"""

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli

FLEET_ID = 100
MEMBER_ID = 200
TO_ID = 300
TASK_ID = 400


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def _stub_verify(monkeypatch):
    # ``message send`` gates on verify_member_fleet; stub it True so the only
    # error surface exercised here is the shared text-input helper.
    monkeypatch.setattr(broker, "verify_member_fleet", lambda *_a, **_k: True)


def _task_payload(text):
    return {
        "task_id": TASK_ID,
        "context_id": TO_ID,
        "from_member_id": MEMBER_ID,
        "to_member_id": TO_ID,
        "type": "unicast",
        "created_at": "2026-05-01T00:00:00+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-01T00:00:00+00:00",
        "origin_task_id": None,
        "text": text,
    }


@pytest.fixture
def send_recorder(monkeypatch):
    calls: list[tuple] = []

    def fake_send(*args, **kwargs):
        calls.append((args, kwargs))
        return {"task": _task_payload("stored")}

    monkeypatch.setattr(broker, "send_message", fake_send)
    return calls


@pytest.fixture
def broadcast_recorder(monkeypatch):
    calls: list[tuple] = []

    def fake_broadcast(*args, **kwargs):
        calls.append((args, kwargs))
        return [{"task": _task_payload("stored"), "recipients": 1, "delivered": 1}]

    monkeypatch.setattr(broker, "broadcast_message", fake_broadcast)
    return calls


def _body_of(call) -> str:
    """Extract the body passed to the broker (last positional or ``text=`` kw)."""
    args, kwargs = call
    return kwargs["text"] if "text" in kwargs else args[-1]


def _send_args(*extra):
    return [
        "message",
        "send",
        "--fleet-id",
        str(FLEET_ID),
        "--from-member-id",
        str(MEMBER_ID),
        "--to-member-id",
        str(TO_ID),
        *extra,
    ]


def _broadcast_args(*extra):
    return [
        "message",
        "broadcast",
        "--fleet-id",
        str(FLEET_ID),
        "--from-member-id",
        str(MEMBER_ID),
        *extra,
    ]


# --- message send ----------------------------------------------------------


def test_send__text_file_body_reaches_broker(runner, send_recorder, tmp_path):
    target = tmp_path / "body.txt"
    target.write_text("a very long body from a file", encoding="utf-8")
    result = runner.invoke(cli, _send_args("--text-file", str(target)))
    assert result.exit_code == 0, result.output
    assert len(send_recorder) == 1
    assert _body_of(send_recorder[0]) == "a very long body from a file"


def test_send__text_file_preserves_crlf_verbatim(runner, send_recorder, tmp_path):
    target = tmp_path / "body.txt"
    target.write_bytes(b"line1\r\nline2\r\n")
    result = runner.invoke(cli, _send_args("--text-file", str(target)))
    assert result.exit_code == 0, result.output
    assert _body_of(send_recorder[0]) == "line1\r\nline2\r\n"


def test_send__stdin_dash_body_reaches_broker(runner, send_recorder):
    result = runner.invoke(
        cli, _send_args("--text-file", "-"), input=b"piped body via stdin"
    )
    assert result.exit_code == 0, result.output
    assert _body_of(send_recorder[0]) == "piped body via stdin"


def test_send__inline_text_still_reaches_broker(runner, send_recorder):
    result = runner.invoke(cli, _send_args("--text", "inline body"))
    assert result.exit_code == 0, result.output
    assert _body_of(send_recorder[0]) == "inline body"


def test_send__body_not_placeholder_substituted(runner, send_recorder):
    # Only member create substitutes; message send passes the body verbatim.
    result = runner.invoke(cli, _send_args("--text", "hi {member_id}"))
    assert result.exit_code == 0, result.output
    assert _body_of(send_recorder[0]) == "hi {member_id}"


@pytest.mark.parametrize("empty", ["", "   ", "\t"])
def test_send__empty_inline_text_rejected(runner, send_recorder, empty):
    result = runner.invoke(cli, _send_args("--text", empty))
    assert result.exit_code == 2, result.output
    assert "text may not be empty." in result.output
    assert send_recorder == []


def test_send__empty_text_file_rejected(runner, send_recorder, tmp_path):
    target = tmp_path / "empty.txt"
    target.write_bytes(b"")
    result = runner.invoke(cli, _send_args("--text-file", str(target)))
    assert result.exit_code == 1, result.output
    assert f"--text-file {target}: file is empty." in result.output
    assert send_recorder == []


def test_send__empty_stdin_rejected(runner, send_recorder):
    result = runner.invoke(cli, _send_args("--text-file", "-"), input=b"")
    assert result.exit_code == 1, result.output
    assert "--text-file -: stdin is empty." in result.output
    assert send_recorder == []


def test_send__neither_flag_rejected(runner, send_recorder):
    result = runner.invoke(cli, _send_args())
    assert result.exit_code == 2, result.output
    assert "Provide exactly one of --text or --text-file." in result.output
    assert send_recorder == []


def test_send__both_flags_rejected(runner, send_recorder, tmp_path):
    target = tmp_path / "body.txt"
    target.write_text("file body", encoding="utf-8")
    result = runner.invoke(
        cli, _send_args("--text", "inline", "--text-file", str(target))
    )
    assert result.exit_code == 2, result.output
    assert "--text and --text-file are mutually exclusive." in result.output
    assert send_recorder == []


def test_send__missing_text_file_rejected(runner, send_recorder, tmp_path):
    target = tmp_path / "nope.txt"
    result = runner.invoke(cli, _send_args("--text-file", str(target)))
    assert result.exit_code == 1, result.output
    assert (
        f"--text-file {target}: file does not exist or is not a regular file."
        in result.output
    )
    assert send_recorder == []


# --- message broadcast -----------------------------------------------------


def test_broadcast__text_file_body_reaches_broker(runner, broadcast_recorder, tmp_path):
    target = tmp_path / "body.txt"
    target.write_text("broadcast body from file", encoding="utf-8")
    result = runner.invoke(cli, _broadcast_args("--text-file", str(target)))
    assert result.exit_code == 0, result.output
    assert len(broadcast_recorder) == 1
    assert _body_of(broadcast_recorder[0]) == "broadcast body from file"


def test_broadcast__stdin_dash_body_reaches_broker(runner, broadcast_recorder):
    result = runner.invoke(
        cli, _broadcast_args("--text-file", "-"), input=b"broadcast via stdin"
    )
    assert result.exit_code == 0, result.output
    assert _body_of(broadcast_recorder[0]) == "broadcast via stdin"


def test_broadcast__inline_text_still_reaches_broker(runner, broadcast_recorder):
    result = runner.invoke(cli, _broadcast_args("--text", "inline broadcast"))
    assert result.exit_code == 0, result.output
    assert _body_of(broadcast_recorder[0]) == "inline broadcast"


@pytest.mark.parametrize("empty", ["", "   ", "\t"])
def test_broadcast__empty_inline_text_rejected(runner, broadcast_recorder, empty):
    result = runner.invoke(cli, _broadcast_args("--text", empty))
    assert result.exit_code == 2, result.output
    assert "text may not be empty." in result.output
    assert broadcast_recorder == []


def test_broadcast__empty_text_file_rejected(runner, broadcast_recorder, tmp_path):
    target = tmp_path / "empty.txt"
    target.write_bytes(b"  \n\t\n")
    result = runner.invoke(cli, _broadcast_args("--text-file", str(target)))
    assert result.exit_code == 1, result.output
    assert f"--text-file {target}: file is empty." in result.output
    assert broadcast_recorder == []


def test_broadcast__neither_flag_rejected(runner, broadcast_recorder):
    result = runner.invoke(cli, _broadcast_args())
    assert result.exit_code == 2, result.output
    assert "Provide exactly one of --text or --text-file." in result.output
    assert broadcast_recorder == []


def test_broadcast__both_flags_rejected(runner, broadcast_recorder, tmp_path):
    target = tmp_path / "body.txt"
    target.write_text("file body", encoding="utf-8")
    result = runner.invoke(
        cli, _broadcast_args("--text", "inline", "--text-file", str(target))
    )
    assert result.exit_code == 2, result.output
    assert "--text and --text-file are mutually exclusive." in result.output
    assert broadcast_recorder == []


def test_broadcast__missing_text_file_rejected(runner, broadcast_recorder, tmp_path):
    target = tmp_path / "nope.txt"
    result = runner.invoke(cli, _broadcast_args("--text-file", str(target)))
    assert result.exit_code == 1, result.output
    assert (
        f"--text-file {target}: file does not exist or is not a regular file."
        in result.output
    )
    assert broadcast_recorder == []
