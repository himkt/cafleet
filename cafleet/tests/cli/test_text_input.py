"""Tests for the shared text-input helper (design 0000112 §1, §2).

``read_text_input`` owns all body resolution and validation for the four
text-body commands (``message send`` / ``message broadcast`` / ``member nudge``
/ ``member create``): the ``--text`` / ``--text-file`` mutual-exclusivity, path
resolution (absolute or CWD-relative), ``-`` stdin, UTF-8 decoding, and uniform
empty-body rejection. ``substitute_spawn_placeholders`` runs the ``member
create``-only spawn-prompt ``.format`` substitution on the resolved body.

Exception polarity is contractual (it drives the CLI exit code): the
xor/required and empty-inline surfaces raise ``UsageError`` (exit 2); the file
and empty-file / empty-stdin surfaces raise a plain ``ClickException`` (exit 1).
"""

import io
import os
import sys
import types

import click
import pytest

from cafleet.cli._text_input import read_text_input, substitute_spawn_placeholders


def _patch_stdin(monkeypatch, data: bytes) -> None:
    """Replace ``sys.stdin`` with a stub whose ``.buffer.read()`` yields ``data``.

    ``read_text_input('-')`` reads ``sys.stdin.buffer.read()`` to EOF, so the
    stub only needs a ``buffer`` attribute backed by a ``BytesIO``.
    """
    monkeypatch.setattr(sys, "stdin", types.SimpleNamespace(buffer=io.BytesIO(data)))


# --- read_text_input: mutual exclusivity + required (§1) -------------------


def test_read_text_input__neither_flag_raises_usage_error():
    with pytest.raises(click.UsageError) as exc_info:
        read_text_input(None, None)
    assert str(exc_info.value) == "Provide exactly one of --text or --text-file."


def test_read_text_input__both_flags_raise_usage_error_before_touching_file():
    # A non-existent path proves the xor check fires before any file access.
    with pytest.raises(click.UsageError) as exc_info:
        read_text_input("inline body", "/nonexistent/path.txt")
    assert str(exc_info.value) == "--text and --text-file are mutually exclusive."


# --- read_text_input: --text inline (§1) -----------------------------------


def test_read_text_input__inline_text_returned_verbatim():
    assert read_text_input("hello world", None) == "hello world"


def test_read_text_input__inline_text_preserves_surrounding_whitespace():
    # The helper never strips — the body reaches the caller verbatim.
    assert read_text_input("  hi \n", None) == "  hi \n"


@pytest.mark.parametrize("text", ["", "   ", "\t", "\n  \n"])
def test_read_text_input__empty_or_whitespace_inline_raises_usage_error(text):
    with pytest.raises(click.UsageError) as exc_info:
        read_text_input(text, None)
    assert str(exc_info.value) == "text may not be empty."


# --- read_text_input: --text-file <path> (§1) ------------------------------


def test_read_text_input__absolute_path_read_and_decoded(tmp_path):
    target = tmp_path / "body.txt"
    target.write_text("file body café", encoding="utf-8")
    assert read_text_input(None, str(target)) == "file body café"


def test_read_text_input__relative_path_resolved_against_cwd(tmp_path, monkeypatch):
    # The old read_prompt_file rejected relative paths; §1 relaxes that — a
    # relative path is now resolved against the current working directory.
    (tmp_path / "rel.txt").write_text("relative body", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    assert read_text_input(None, "rel.txt") == "relative body"


def test_read_text_input__utf8_multibyte_decoded(tmp_path):
    target = tmp_path / "u.txt"
    target.write_text("日本語✓ éàü", encoding="utf-8")
    assert read_text_input(None, str(target)) == "日本語✓ éàü"


@pytest.mark.parametrize(
    ("scenario", "raw"),
    [
        ("crlf", b"line1\r\nline2\r\n"),
        ("bare_cr", b"line1\rline2"),
        ("mixed", b"a\r\nb\rc\n"),
    ],
)
def test_read_text_input__file_preserves_cr_crlf_verbatim(tmp_path, scenario, raw):
    # read_bytes().decode — NOT read_text — so universal-newline translation
    # never collapses CR / CRLF to LF. The body arrives byte-for-byte.
    target = tmp_path / "nl.txt"
    target.write_bytes(raw)
    assert read_text_input(None, str(target)) == raw.decode("utf-8")


@pytest.mark.parametrize(
    ("scenario", "raw"),
    [("zero_bytes", b""), ("whitespace_only", b"  \n\t\n")],
)
def test_read_text_input__empty_file_raises_click_exception(tmp_path, scenario, raw):
    target = tmp_path / "empty.txt"
    target.write_bytes(raw)
    with pytest.raises(click.ClickException) as exc_info:
        read_text_input(None, str(target))
    # ClickException, not UsageError → exit 1, not exit 2.
    assert not isinstance(exc_info.value, click.UsageError)
    assert str(exc_info.value) == f"--text-file {target}: file is empty."


# --- read_text_input: --text-file file-error surfaces (§1) -----------------


def test_read_text_input__missing_file_raises_non_regular_click_exception(tmp_path):
    target = tmp_path / "nope.txt"
    with pytest.raises(click.ClickException) as exc_info:
        read_text_input(None, str(target))
    assert not isinstance(exc_info.value, click.UsageError)
    assert (
        str(exc_info.value)
        == f"--text-file {target}: file does not exist or is not a regular file."
    )


def test_read_text_input__directory_raises_non_regular_click_exception(tmp_path):
    target = tmp_path / "a_dir"
    target.mkdir()
    with pytest.raises(click.ClickException) as exc_info:
        read_text_input(None, str(target))
    assert (
        str(exc_info.value)
        == f"--text-file {target}: file does not exist or is not a regular file."
    )


def test_read_text_input__invalid_utf8_raises_click_exception(tmp_path):
    target = tmp_path / "bad.txt"
    target.write_bytes(b"\xff\xfe\xfd")
    with pytest.raises(click.ClickException) as exc_info:
        read_text_input(None, str(target))
    assert not isinstance(exc_info.value, click.UsageError)
    assert str(exc_info.value) == f"--text-file {target}: file is not valid UTF-8."


@pytest.mark.skipif(
    sys.platform == "win32" or (hasattr(os, "geteuid") and os.geteuid() == 0),
    reason="POSIX-only; root bypasses the read-permission check",
)
def test_read_text_input__unreadable_file_raises_click_exception(tmp_path):
    # The existence / regular-file check rides the read_bytes() exception
    # surface (no is_file() pre-check), so a permission failure lands in the
    # 'not readable' branch directly rather than the 'does not exist' branch.
    target = tmp_path / "locked.txt"
    target.write_text("secret", encoding="utf-8")
    target.chmod(0o000)
    try:
        with pytest.raises(click.ClickException) as exc_info:
            read_text_input(None, str(target))
        assert not isinstance(exc_info.value, click.UsageError)
        assert str(exc_info.value) == f"--text-file {target}: file is not readable."
    finally:
        target.chmod(0o644)


# --- read_text_input: --text-file - stdin (§1) -----------------------------


def test_read_text_input__stdin_dash_reads_all(monkeypatch):
    _patch_stdin(monkeypatch, b"piped body from stdin")
    assert read_text_input(None, "-") == "piped body from stdin"


def test_read_text_input__stdin_preserves_cr_crlf_verbatim(monkeypatch):
    _patch_stdin(monkeypatch, b"a\r\nb\rc")
    assert read_text_input(None, "-") == "a\r\nb\rc"


def test_read_text_input__stdin_utf8_decoded(monkeypatch):
    _patch_stdin(monkeypatch, "café ☕ 日本語".encode())
    assert read_text_input(None, "-") == "café ☕ 日本語"


@pytest.mark.parametrize("raw", [b"", b"  \n\t\n"])
def test_read_text_input__empty_stdin_raises_click_exception(monkeypatch, raw):
    _patch_stdin(monkeypatch, raw)
    with pytest.raises(click.ClickException) as exc_info:
        read_text_input(None, "-")
    assert not isinstance(exc_info.value, click.UsageError)
    assert str(exc_info.value) == "--text-file -: stdin is empty."


# --- substitute_spawn_placeholders (§2) ------------------------------------

_SUB_KWARGS = {
    "fleet_id": 11,
    "agent_id": 22,
    "director_agent_id": 33,
    "coding_agent": "claude",
}


def test_substitute__all_four_placeholders_substituted():
    body = (
        "fleet={fleet_id} agent={agent_id} "
        "director={director_agent_id} ca={coding_agent}"
    )
    result = substitute_spawn_placeholders(body, **_SUB_KWARGS)
    assert result == "fleet=11 agent=22 director=33 ca=claude"
    for raw in ("{fleet_id}", "{agent_id}", "{director_agent_id}", "{coding_agent}"):
        assert raw not in result


def test_substitute__no_placeholders_passthrough():
    assert (
        substitute_spawn_placeholders("plain spawn body", **_SUB_KWARGS)
        == "plain spawn body"
    )


def test_substitute__doubled_brace_collapses_to_single_literal():
    result = substitute_spawn_placeholders("data {{not}} a placeholder", **_SUB_KWARGS)
    assert result == "data {not} a placeholder"


def test_substitute__unknown_placeholder_raises_usage_error_listing_supported():
    with pytest.raises(click.UsageError) as exc_info:
        substitute_spawn_placeholders("hello {foo}", **_SUB_KWARGS)
    message = str(exc_info.value)
    for needle in (
        "{fleet_id}",
        "{agent_id}",
        "{director_agent_id}",
        "{coding_agent}",
    ):
        assert needle in message


@pytest.mark.parametrize(
    ("scenario", "body"),
    [
        ("unmatched_brace", "hello {unclosed"),
        ("attribute_access", "hello {agent_id.foo}"),
    ],
)
def test_substitute__malformed_prompt_raises_usage_error(scenario, body):
    with pytest.raises(click.UsageError) as exc_info:
        substitute_spawn_placeholders(body, **_SUB_KWARGS)
    message = str(exc_info.value)
    # The malformed-prompt guidance tells the author to double literal braces.
    assert "{{" in message
    assert "}}" in message
