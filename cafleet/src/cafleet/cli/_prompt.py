"""Member spawn-prompt resolution."""

from pathlib import Path

import click

MEMBER_PROMPT_TEMPLATE = (
    "Member of cafleet fleet {fleet_id} "
    "(agent={agent_id}, director={director_agent_id}).\n"
    "Load skill 'cafleet'. Bash auto-approves. Poll: "
    "cafleet message poll --fleet-id {fleet_id} --agent-id {agent_id}"
)


def read_prompt_file(path: str) -> str:
    """Read the spawn prompt from a file, validating absolute path / readability / UTF-8 / non-empty.

    Owns the five error surfaces: relative path →
    ``UsageError``; missing / non-regular file → ``ClickException``;
    permission / generic-I/O failures → ``ClickException``; invalid UTF-8 →
    ``ClickException``; zero-byte or whitespace-only contents →
    ``ClickException``. Returns the file body verbatim — no stripping,
    trailing newlines preserved.

    The existence and regular-file checks ride entirely on the
    ``read_bytes()`` exception surface (no ``is_file()`` pre-check), so a
    path on which we lack traverse permission cannot leak through a stat
    gate and surface as the wrong message — ``PermissionError`` from
    ``read_bytes`` lands in the ``file is not readable`` branch directly.
    """
    if not Path(path).is_absolute():
        raise click.UsageError(
            f"--prompt-file requires an absolute path (got '{path}'). "
            "Resolve relative paths against your BASE first — "
            "see the `cafleet-base-dir` skill."
        )
    file_path = Path(path)
    try:
        # read_bytes() + decode() instead of read_text() so universal-newline
        # translation does NOT collapse CRLF / CR to LF — the success-criterion
        # promises the file body lands in the spawn argv byte-for-byte verbatim.
        content = file_path.read_bytes().decode("utf-8")
    except (FileNotFoundError, IsADirectoryError) as exc:
        # FileNotFoundError → the path does not name an existing file.
        # IsADirectoryError → the path names a directory, not a regular file.
        # Both map to the § 6 "missing or non-regular file" surface.
        raise click.ClickException(
            f"--prompt-file {path}: file does not exist or is not a regular file."
        ) from exc
    except PermissionError as exc:
        raise click.ClickException(
            f"--prompt-file {path}: file is not readable."
        ) from exc
    except UnicodeDecodeError as exc:
        raise click.ClickException(
            f"--prompt-file {path}: file is not valid UTF-8."
        ) from exc
    except OSError as exc:
        # Reserved for true I/O failures (EIO, ENOSPC, EBUSY, …). The earlier
        # FileNotFoundError / IsADirectoryError / PermissionError branches
        # handle the OSError subclasses that have a more precise § 6 message.
        raise click.ClickException(
            f"--prompt-file {path}: file is not readable."
        ) from exc
    if content == "" or content.isspace():
        raise click.ClickException(f"--prompt-file {path}: file is empty.")
    return content


def resolve_prompt(
    ctx: click.Context,
    director_agent_id: int,
    new_agent_id: int,
    prompt_argv: tuple[str, ...],
    prompt_file: str | None = None,
    coding_agent: str | None = None,
) -> str:
    """Substitute ``fleet_id`` / ``agent_id`` / ``director_agent_id`` / ``coding_agent`` into the spawn prompt.

    Runs ``str.format`` on the chosen template (file > positional > default)
    so custom prompts must double literal braces (``{{`` / ``}}``) to survive
    the substitution. ``coding_agent`` is the resolved backend, used by the
    monitor prompt's ``CODING AGENT:`` line.
    """
    fleet_id = ctx.obj["fleet_id"]
    if prompt_file is not None:
        template = read_prompt_file(prompt_file)
    elif prompt_argv:
        template = " ".join(prompt_argv)
    else:
        template = MEMBER_PROMPT_TEMPLATE
    try:
        return template.format(
            fleet_id=fleet_id,
            agent_id=new_agent_id,
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
