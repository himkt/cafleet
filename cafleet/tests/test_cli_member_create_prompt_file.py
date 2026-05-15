"""Tests for ``--prompt-file`` on ``cafleet member create`` (design doc 0000059)."""

import json
import os
import sys
import uuid

import click
import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import _resolve_prompt, cli
from cafleet.tmux import DirectorContext

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def bootstrapped_session(tmp_path, monkeypatch, _reset_engine_singletons):
    """Fresh DB + session + Director, returning ``(session_id, director_id, runner)``.

    Mirrors the fixture in ``test_cli_member.py`` so the spawn-argv assertions
    here line up with the same plumbing.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _FAKE_DIRECTOR_CTX)

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    create = runner.invoke(cli, ["session", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    return data["session_id"], data["director"]["agent_id"], runner


@pytest.fixture
def split_window_recorder(monkeypatch):
    calls: list[dict] = []

    def fake_split_window(**kwargs):
        calls.append(kwargs)
        return "%42"

    monkeypatch.setattr("cafleet.tmux.split_window", fake_split_window)
    monkeypatch.setattr("cafleet.tmux.select_layout", lambda **_: None)
    monkeypatch.setattr("cafleet.tmux.send_exit", lambda **_: None, raising=False)
    return calls


@pytest.fixture
def stub_coding_agent_binaries(monkeypatch):
    monkeypatch.setattr("cafleet.cli.shutil.which", lambda _: "/usr/bin/stub")


def _invoke_member_create(
    runner: CliRunner,
    session_id: str,
    director_id: str,
    *,
    coding_agent: str = "claude",
    prompt_file: str | None = None,
    inline_prompt: str | None = None,
    name: str = "Member",
):
    args = [
        "--session-id",
        session_id,
        "member",
        "create",
        "--agent-id",
        director_id,
        "--name",
        name,
        "--description",
        f"{name} for prompt-file tests",
    ]
    if coding_agent != "claude":
        args.extend(["--coding-agent", coding_agent])
    if prompt_file is not None:
        args.extend(["--prompt-file", prompt_file])
    if inline_prompt is not None:
        args.extend(["--", inline_prompt])
    return runner.invoke(cli, args)


# --- prompt_file_substitutes_session_id_placeholder: design doc 0000059 §1+§4.
# A file whose body contains ``{session_id}`` is read, substituted via
# ``str.format``, and the resulting text becomes the spawn argv's prompt
# element (last element for both claude and codex backends). Parametrized
# across both backends so neither path regresses. ---


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_prompt_file_substitutes_session_id_placeholder(
    coding_agent,
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("session={session_id}", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        coding_agent=coding_agent,
        prompt_file=str(prompt_path),
        name=f"Member-{coding_agent}",
    )
    assert result.exit_code == 0, result.output
    assert len(split_window_recorder) == 1
    command = split_window_recorder[0]["command"]
    assert command[-1] == f"session={session_id}"
    if coding_agent == "claude":
        assert command[0] == "claude"
    else:
        assert command[0] == "codex"


# --- prompt_file_parity_with_positional_form: design doc 0000059 §4.
# ``--prompt-file <path>`` and the positional ``prompt_argv`` form must
# substitute identically — the file-read path is just a different SOURCE for
# the same template that flows through the same ``_resolve_prompt``. Tested
# at the helper boundary so the new-agent UUID allocation that varies between
# CLI invocations does not muddy a byte-for-byte comparison. ---


def test_prompt_file_parity_with_positional_form(tmp_path):
    session_id = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    new_agent_id = str(uuid.uuid4())
    template = "hello {agent_id} from director {director_agent_id}"

    command = click.Command("member-create")
    ctx = click.Context(command)
    ctx.obj = {"session_id": session_id, "json_output": False}

    inline_result = _resolve_prompt(
        ctx,
        director_agent_id=director_id,
        new_agent_id=new_agent_id,
        prompt_argv=tuple(template.split(" ")),
        prompt_file=None,
    )

    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text(template, encoding="utf-8")
    file_result = _resolve_prompt(
        ctx,
        director_agent_id=director_id,
        new_agent_id=new_agent_id,
        prompt_argv=(),
        prompt_file=str(prompt_path),
    )

    assert inline_result == file_result
    assert new_agent_id in file_result
    assert director_id in file_result


# --- mutually_exclusive: design doc 0000059 §2 + §6. Supplying both
# ``--prompt-file`` AND a positional prompt is a hard ``UsageError`` (exit 2).
# The check fires before any registration work begins so no half-created
# agent is left behind — verified by asserting ``split_window`` was never
# reached. ---


def test_prompt_file_and_positional_are_mutually_exclusive(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("hello", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(prompt_path),
        inline_prompt="hello positional",
    )
    assert result.exit_code == 2, result.output
    assert (
        "--prompt-file and the positional prompt argument are mutually exclusive"
        in result.output
    )
    assert len(split_window_recorder) == 0


# --- relative_path_rejected: design doc 0000059 §3 + §6. The CLI does NOT
# resolve relative paths against any base directory — that is the caller's
# job. A relative input like ``./foo.md`` exits with the absolute-path
# ``UsageError`` BEFORE any existence check fires. The Click option must be
# declared as ``type=str`` (not ``click.Path``) for this ordering to hold. ---


def test_prompt_file_relative_path_rejected(
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file="./foo.md",
    )
    assert result.exit_code == 2, result.output
    assert "--prompt-file requires an absolute path" in result.output
    assert "./foo.md" in result.output
    assert len(split_window_recorder) == 0


# --- file_not_found: design doc 0000059 §5 + §6. A path that does not
# resolve to a regular file exits 1 (``ClickException``) with the file-not-
# found message. Companion test below covers the directory-path case
# (same expected message, since ``Path.is_file()`` returns False for
# directories). ---


def test_prompt_file_not_found(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    missing_path = tmp_path / "does-not-exist.md"

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(missing_path),
    )
    assert result.exit_code == 1, result.output
    assert "file does not exist or is not a regular file" in result.output
    assert str(missing_path) in result.output
    assert len(split_window_recorder) == 0


def test_prompt_file_directory_not_regular_file(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    dir_path = tmp_path / "subdir"
    dir_path.mkdir()

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(dir_path),
    )
    assert result.exit_code == 1, result.output
    assert "file does not exist or is not a regular file" in result.output
    assert len(split_window_recorder) == 0


# --- empty_zero_bytes / empty_whitespace_only: design doc 0000059 §5 + §6.
# A zero-byte file AND a whitespace-only file (``content.isspace()``) both
# exit 1 with the ``file is empty`` message. The whitespace-only branch
# guards against a sneaky "user opened the file in an editor and saved a
# blank line" footgun. ---


def test_prompt_file_empty_zero_bytes(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    empty_file = tmp_path / "empty.md"
    empty_file.write_bytes(b"")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(empty_file),
    )
    assert result.exit_code == 1, result.output
    assert "file is empty" in result.output
    assert len(split_window_recorder) == 0


def test_prompt_file_empty_whitespace_only(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    ws_file = tmp_path / "whitespace.md"
    ws_file.write_text("\n   \t\n", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(ws_file),
    )
    assert result.exit_code == 1, result.output
    assert "file is empty" in result.output
    assert len(split_window_recorder) == 0


# --- invalid_utf8: design doc 0000059 §5 + §6. A file containing bytes that
# cannot be decoded as UTF-8 exits 1 with the ``file is not valid UTF-8``
# message. ``b"\xff\xfe\xfd"`` is the canonical "no valid leading byte"
# fixture used elsewhere in the project. ---


def test_prompt_file_invalid_utf8(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    bad_file = tmp_path / "bad.md"
    bad_file.write_bytes(b"\xff\xfe\xfd")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(bad_file),
    )
    assert result.exit_code == 1, result.output
    assert "file is not valid UTF-8" in result.output
    assert len(split_window_recorder) == 0


# --- not_readable: design doc 0000059 §5 + §6. ``Path.read_text`` raises
# ``PermissionError`` when the OS denies read access; the helper converts it
# to ``ClickException`` (exit 1) with the ``file is not readable`` message.
# Skipped on Windows (no POSIX permission semantics) and when running as
# root (root bypasses the read bit). ---


@pytest.mark.skipif(
    sys.platform == "win32" or (hasattr(os, "geteuid") and os.geteuid() == 0),
    reason="POSIX-only test; root bypasses the read-permission check",
)
def test_prompt_file_not_readable(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    unreadable_file = tmp_path / "unreadable.md"
    unreadable_file.write_text("hello", encoding="utf-8")
    unreadable_file.chmod(0o000)
    try:
        result = _invoke_member_create(
            runner,
            session_id,
            director_id,
            prompt_file=str(unreadable_file),
        )
        assert result.exit_code == 1, result.output
        assert "file is not readable" in result.output
        assert str(unreadable_file) in result.output
        assert len(split_window_recorder) == 0
    finally:
        unreadable_file.chmod(0o644)


# --- preserves_trailing_newline / preserves_surrounding_whitespace: design
# doc 0000059 SC bullets + §4. The helper does NOT strip — every byte of the
# file body, including leading/inner/trailing whitespace and trailing
# newlines, lands verbatim in the spawn argv. The whitespace fixture below
# contains real text in the middle so the emptiness check does not fire. ---


def test_prompt_file_preserves_trailing_newline(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("hello\n", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(prompt_path),
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][-1] == "hello\n"


def test_prompt_file_preserves_surrounding_whitespace(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    content = "   \n  hello world  \n   "
    prompt_path.write_text(content, encoding="utf-8")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(prompt_path),
    )
    assert result.exit_code == 0, result.output
    assert split_window_recorder[0]["command"][-1] == content


# --- unknown_placeholder_raises_usage_error: design doc 0000059 §6 last row.
# An unknown ``{placeholder}`` in file content goes through the SAME
# ``str.format`` substitution as the positional path, so the existing
# ``Unknown placeholder ...`` ``UsageError`` (exit 2) fires unchanged. This
# is a regression guard that the file-read path does not bypass the format
# error-handling branch. ---


def test_prompt_file_unknown_placeholder_raises_usage_error(
    tmp_path,
    bootstrapped_session,
    split_window_recorder,
    stub_coding_agent_binaries,
):
    session_id, director_id, runner = bootstrapped_session
    prompt_path = tmp_path / "prompt.md"
    prompt_path.write_text("hi {unknown}", encoding="utf-8")

    result = _invoke_member_create(
        runner,
        session_id,
        director_id,
        prompt_file=str(prompt_path),
    )
    assert result.exit_code == 2, result.output
    assert "Unknown placeholder" in result.output
    assert "unknown" in result.output
    assert "{session_id}" in result.output
    assert "{agent_id}" in result.output
    assert len(split_window_recorder) == 0
