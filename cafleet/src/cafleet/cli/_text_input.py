"""Shared ``--text`` / ``--text-file`` body reader for the text-body commands.

``read_text_input`` owns all body resolution and validation for the four
text-body commands (``message send`` / ``message broadcast`` / ``member nudge``
/ ``member create``): the mutual-exclusivity of the two flags, path resolution
(absolute or CWD-relative), ``-`` stdin, UTF-8 decoding (byte-for-byte — no
universal-newline translation), and uniform empty-body rejection.

``substitute_spawn_placeholders`` is the ``member create``-only spawn-prompt
``.format`` substitution, applied to the resolved body after ``read_text_input``
returns.

Exception polarity is contractual (it drives the CLI exit code): the
xor / required and empty-inline surfaces raise ``UsageError`` (exit 2); the file
and empty-file / empty-stdin surfaces raise a plain ``ClickException`` (exit 1).
"""

import sys
from pathlib import Path

import click


def read_text_input(text: str | None, text_file: str | None) -> str:
    """Resolve a command body from the ``--text`` / ``--text-file`` pair.

    Enforces exactly-one-of, resolves the file path (absolute, or relative to
    CWD; ``'-'`` reads all of stdin), decodes UTF-8, and rejects an empty or
    whitespace-only body. Returns the body verbatim (no stripping).
    """
    if text is not None and text_file is not None:
        raise click.UsageError("--text and --text-file are mutually exclusive.")

    if text is not None:
        if text == "" or text.isspace():
            raise click.UsageError("text may not be empty.")
        return text

    if text_file is None:
        raise click.UsageError("Provide exactly one of --text or --text-file.")

    if text_file == "-":
        content = sys.stdin.buffer.read().decode("utf-8")
        if content == "" or content.isspace():
            raise click.ClickException("--text-file -: stdin is empty.")
        return content

    # A relative path resolves against the CWD (Path opens relative to CWD); an
    # absolute path is used as-is. read_bytes().decode — NOT read_text — so
    # universal-newline translation never collapses CR / CRLF to LF. The
    # existence / regular-file checks ride the read_bytes() exception surface
    # (no is_file() pre-check), so a permission failure lands in the
    # 'not readable' branch directly rather than the 'does not exist' branch.
    try:
        content = Path(text_file).read_bytes().decode("utf-8")
    except (FileNotFoundError, IsADirectoryError) as exc:
        raise click.ClickException(
            f"--text-file {text_file}: file does not exist or is not a regular file."
        ) from exc
    except PermissionError as exc:
        raise click.ClickException(
            f"--text-file {text_file}: file is not readable."
        ) from exc
    except UnicodeDecodeError as exc:
        raise click.ClickException(
            f"--text-file {text_file}: file is not valid UTF-8."
        ) from exc
    except OSError as exc:
        # True I/O failures (EIO, ENOSPC, …); the more precise OSError
        # subclasses above already have their own § 1 message.
        raise click.ClickException(
            f"--text-file {text_file}: file is not readable."
        ) from exc
    if content == "" or content.isspace():
        raise click.ClickException(f"--text-file {text_file}: file is empty.")
    return content


def substitute_spawn_placeholders(
    body: str,
    *,
    fleet_id: int,
    agent_id: int,
    director_agent_id: int,
    coding_agent: str | None,
) -> str:
    """Run ``str.format`` on a spawn prompt (``member create`` only).

    Substitutes ``{fleet_id}`` / ``{agent_id}`` / ``{director_agent_id}`` /
    ``{coding_agent}``; a custom prompt keeps a literal brace by doubling it
    (``{{`` / ``}}``). An unknown placeholder or a malformed brace expression is
    a ``UsageError`` (exit 2).
    """
    try:
        return body.format(
            fleet_id=fleet_id,
            agent_id=agent_id,
            director_agent_id=director_agent_id,
            coding_agent=coding_agent,
        )
    except KeyError as exc:
        raise click.UsageError(
            f"Unknown placeholder {exc} in custom prompt. "
            "Supported placeholders: {fleet_id}, {agent_id}, "
            "{director_agent_id}, {coding_agent}. "
            "Double literal braces ({{, }}) to keep them as text."
        ) from exc
    except (ValueError, IndexError, AttributeError) as exc:
        raise click.UsageError(
            f"Malformed custom prompt: {exc}. "
            "Double literal braces ({{, }}) to keep them as text."
        ) from exc
