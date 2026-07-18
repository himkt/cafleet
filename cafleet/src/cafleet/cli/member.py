"""``cafleet member`` — multiplexer-backed member commands (Director only)."""

import contextlib
import os
from typing import Literal, NoReturn, overload

import click

from cafleet import broker, output
from cafleet.broker import _shared
from cafleet.cli._helpers import (
    ensure_assets_current,
    ensure_multiplexer_or_die,
    fleet_id_option,
    full_flag,
    json_flag,
    member_id_option,
    quiet_flag,
    text_body_options,
)
from cafleet.cli._text_input import read_text_input, substitute_spawn_placeholders
from cafleet.coding_agent import CODING_AGENTS
from cafleet.multiplexer import MultiplexerError, resolve_multiplexer


@click.group()
def member():
    """Manage multiplexer-backed members (Director only)."""
    ensure_assets_current()


def _require_member_pane(placement: dict, member_id: int, action: str) -> str:
    pane_id = placement["mux_pane_id"]
    if pane_id is None:
        raise click.ClickException(
            f"member {member_id} has no pane yet (pending placement) "
            f"— nothing to {action}."
        )
    return pane_id


@overload
def _load_authorized_member(
    fleet_id: int,
    member_id: int,
    *,
    allow_missing_placement: Literal[True],
) -> tuple[dict, dict | None]: ...


@overload
def _load_authorized_member(
    fleet_id: int,
    member_id: int,
) -> tuple[dict, dict]: ...


def _load_authorized_member(
    fleet_id: int,
    member_id: int,
    *,
    allow_missing_placement: bool = False,
) -> tuple[dict, dict | None]:
    """Resolve a fleet-scoped member's registry row + placement.

    The only boundary is fleet isolation: ``broker.get_member(member_id,
    fleet_id)`` returns ``None`` for a ``member_id`` that is not an active
    member of ``fleet_id``, so a cross-fleet / unknown / inactive target raises
    "not found". There is no caller-auth check — any active in-fleet member is
    a valid target. An in-fleet member without a placement row raises the
    placement-missing error, unless the caller opts in via
    ``allow_missing_placement`` (``member show`` and ``member delete`` do —
    both resolve a placementless target successfully and receive
    ``placement=None``). Pane-id presence is NOT checked here — delete
    tolerates a pending placement while the pane verbs reject it.

    Callers MUST use ``target["member_id"]``, since reassigning the
    ``member_id`` local param does not propagate to the caller.
    """
    try:
        target = broker.get_member(member_id, fleet_id)
    except Exception as exc:
        raise click.ClickException(f"failed to fetch member: {exc}") from exc
    if target is None:
        raise click.ClickException(f"Member {member_id} not found")
    placement = target["placement"]
    if placement is None and not allow_missing_placement:
        raise click.ClickException(
            f"member {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`."
        )
    return target, placement


def _deregister_with_warning(new_member_id: int, *, fleet_id: int) -> None:
    """Best-effort deregister; emit warning to stderr if it fails."""
    try:
        broker.deregister_member(new_member_id)
    except Exception as drop_exc:
        click.echo(
            f"WARNING: rollback deregister failed — member {new_member_id} is "
            f"orphaned in the registry. Run `cafleet member delete "
            f"--fleet-id {fleet_id} --member-id {new_member_id}` manually to "
            f"clean up. Cause: {drop_exc}",
            err=True,
        )


def _rollback_register(new_member_id: int, *, fleet_id: int, reason: str) -> NoReturn:
    """Best-effort deregister of a just-created member, then raise ClickException."""
    _deregister_with_warning(new_member_id, fleet_id=fleet_id)
    raise click.ClickException(
        f"{reason}. Rolled back registration of {new_member_id}."
    )


def _resolve_director_or_die(fleet_id: int) -> int:
    """Auto-resolve the fleet's root Director id, first thing in ``member create``.

    A fleet has exactly one root Director, back-filled by ``create_fleet``, so
    no override flag exists. The error polarity follows the §185 table: an
    unknown fleet is a usage error (exit 2); a soft-deleted fleet or a
    mid-bootstrap-corrupted fleet row is an application error (exit 1).
    """
    fleet = broker.get_fleet(fleet_id)
    if fleet is None:
        raise click.UsageError(f"Fleet '{fleet_id}' not found.")
    if fleet["deleted_at"] is not None:
        raise click.ClickException(f"fleet {fleet_id} is deleted")
    director_member_id = fleet["director_member_id"]
    if director_member_id is None:
        raise click.ClickException(
            f"fleet {fleet_id} has no root Director recorded; "
            f"re-create the fleet with 'cafleet fleet create'."
        )
    return director_member_id


def _resolve_coding_agent(
    coding_agent: str | None,
    director_member_id: int,
    fleet_id: int,
) -> str:
    """Resolve the backend for a new member.

    An explicit ``--coding-agent`` always wins. When the flag is omitted,
    the member inherits the spawning Director's backend from its placement row.
    """
    if coding_agent is not None:
        return coding_agent
    try:
        director = broker.get_member(director_member_id, fleet_id)
    except Exception as exc:  # broker/DB failure — surface, do not mask
        raise click.ClickException(
            f"cannot resolve the member's coding agent: failed to fetch "
            f"Director {director_member_id}: {exc}. "
            f"Re-run with an explicit --coding-agent."
        ) from exc
    if director is None:
        raise click.ClickException(
            f"cannot resolve the member's coding agent: Director "
            f"{director_member_id} not found in fleet {fleet_id}. "
            f"Re-run with an explicit --coding-agent."
        )
    placement = director["placement"]
    if placement is None:
        raise click.ClickException(
            f"cannot resolve the member's coding agent: Director "
            f"{director_member_id} has no placement row recording its backend. "
            f"Re-run with an explicit --coding-agent."
        )
    return placement["coding_agent"]


@member.command("create")
@fleet_id_option
@click.option("--name", required=True, help="Member name")
@click.option("--description", required=True, help="Member description")
@click.option(
    "--coding-agent",
    "coding_agent",
    type=click.Choice(list(CODING_AGENTS.keys())),
    default=None,
    show_default="inherits the Director's backend",
    help="Backend binary to spawn / record.",
)
@click.option(
    "--model",
    "model",
    type=str,
    default=None,
    help="Model passed to the backend binary.",
)
@click.option(
    "--role",
    "role",
    type=click.Choice(["member", "monitor"]),
    default="member",
    show_default=True,
    help="Member role. 'monitor' spawns the dedicated monitoring member.",
)
@text_body_options("Inline spawn prompt.")
@full_flag
@json_flag
@click.pass_context
def member_create(
    ctx,
    name,
    description,
    coding_agent,
    model,
    role,
    text,
    text_file,
    full,
    json_output,
):
    """Register a new member and spawn its pane (Director auto-resolved)."""
    fleet_id = ctx.obj["fleet_id"]

    # Director auto-discovery runs first thing: the resolved id feeds the
    # member's backend inheritance and the spawn-prompt substitution.
    director_member_id = _resolve_director_or_die(fleet_id)

    coding_agent = _resolve_coding_agent(coding_agent, director_member_id, fleet_id)

    agent = CODING_AGENTS[coding_agent]

    try:
        agent.validate_model(model)
    except ValueError as exc:
        raise click.UsageError(str(exc)) from exc

    # Resolve the body up front (xor / required / empty / file surfaces raise
    # before any registration or multiplexer side effect). Placeholder
    # substitution is deferred until after register_member, since {member_id}
    # needs the new id.
    body = read_text_input(text, text_file)

    try:
        mux = resolve_multiplexer()
        mux.ensure_available()
        agent.ensure_available()
        director_ctx = mux.context_discovery()
    except (MultiplexerError, RuntimeError) as exc:
        raise click.ClickException(str(exc)) from exc

    try:
        result = broker.register_member(
            fleet_id,
            name,
            description,
            placement={
                "backend": mux.name,
                "mux_session": director_ctx.session,
                "mux_window_id": director_ctx.window_id,
                "mux_pane_id": None,
                "coding_agent": coding_agent,
            },
            kind=_shared.MONITORING_MEMBER_KIND if role == "monitor" else None,
        )
    except click.ClickException:
        # The one-monitoring-member-per-fleet and root-Director invariant guards
        # raise ClickException with a user-facing message; surface it verbatim
        # rather than wrapping it.
        raise
    except Exception as exc:
        raise click.ClickException(f"register failed: {exc}") from exc
    new_member_id = result["member_id"]

    try:
        prompt = substitute_spawn_placeholders(
            body,
            fleet_id=fleet_id,
            member_id=new_member_id,
            director_member_id=director_member_id,
            coding_agent=coding_agent,
        )
    except (click.UsageError, click.ClickException):
        # Re-raise unwrapped so the exact substitution error (e.g. the
        # unknown-placeholder UsageError) reaches the operator. Wrapping via
        # _rollback_register would prepend "prompt resolution failed:" and
        # downgrade UsageError exit 2 → ClickException exit 1.
        _deregister_with_warning(new_member_id, fleet_id=fleet_id)
        raise

    spawn_command = agent.build_spawn_argv(prompt, display_name=name, model=model)

    try:
        db_url = os.environ.get("CAFLEET_DATABASE_URL")
        fwd_env = {"CAFLEET_DATABASE_URL": db_url} if db_url else {}
        pane_id = mux.split_window(
            reference=director_ctx,
            env=fwd_env,
            command=spawn_command,
        )
    except MultiplexerError as exc:
        _rollback_register(
            new_member_id,
            fleet_id=fleet_id,
            reason=f"split_window failed: {exc}",
        )

    try:
        placement_view = broker.update_placement_pane_id(new_member_id, pane_id)
    except Exception as exc:
        # Pane is alive but the registration row is dangling; /exit the pane
        # and roll back the member so the caller can retry cleanly.
        with contextlib.suppress(MultiplexerError):
            mux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        _rollback_register(
            new_member_id,
            fleet_id=fleet_id,
            reason=f"placement update failed: {exc}",
        )
    if placement_view is None:
        with contextlib.suppress(MultiplexerError):
            mux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        _rollback_register(
            new_member_id,
            fleet_id=fleet_id,
            reason="placement row vanished before pane-id patch",
        )

    result["placement"] = placement_view
    if json_output:
        click.echo(output.format_json(result))
    else:
        click.echo(output.format_member(result, full=full))


@member.command("delete")
@fleet_id_option
@member_id_option
@json_flag
@click.pass_context
def member_delete(ctx, member_id, json_output):
    """Deregister a member and kill its tmux pane."""
    fleet_id = ctx.obj["fleet_id"]

    fleet = broker.get_fleet(fleet_id)
    if fleet is not None and member_id == fleet["director_member_id"]:
        raise click.ClickException(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead"
        )

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
        allow_missing_placement=True,
    )
    member_id = target["member_id"]
    pane_id = placement["mux_pane_id"] if placement is not None else None

    if pane_id is None:
        # Pure registry soft-delete — no multiplexer requirement off the pane paths.
        _deregister_or_die(member_id)
        pane_status = "(no placement)" if placement is None else "(pending — no pane)"
        _emit_member_delete_output(
            json_output, member_id, pane_status, header="Member deleted."
        )
        return

    mux = ensure_multiplexer_or_die()
    try:
        mux.kill_pane(target_pane_id=pane_id, ignore_missing=True)
    except MultiplexerError as exc:
        raise click.ClickException(
            f"kill_pane failed for pane {pane_id}: {exc}. "
            f"The {mux.name} server may be unreachable. Verify with "
            f"'cafleet doctor', then re-run the command."
        ) from exc
    _deregister_or_die(member_id)
    pane_status = f"{pane_id} (killed)"
    _emit_member_delete_output(
        json_output, member_id, pane_status, header="Member deleted."
    )


def _deregister_or_die(member_id: int) -> None:
    """Deregister, surfacing the broker's own guards verbatim.

    The root-Director guard raises a ``ClickException``
    subclass with a user-facing message; only unexpected failures get the
    ``deregister failed:`` wrapping.
    """
    try:
        broker.deregister_member(member_id)
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(f"deregister failed: {exc}") from exc


def _emit_member_delete_output(
    json_output: bool,
    member_id: int,
    pane_status: str,
    *,
    header: str | None,
) -> None:
    if json_output:
        click.echo(
            output.format_json(
                {"member_id": member_id, "pane_status": pane_status},
            )
        )
    elif header is not None:
        click.echo(header)
        click.echo(f"  member_id:  {member_id}")
        click.echo(f"  pane_id:    {pane_status}")


@member.command("show")
@fleet_id_option
@member_id_option
@click.option(
    "--full",
    "full",
    is_flag=True,
    default=False,
    help="Verbose labeled block (text mode only).",
)
@json_flag
@click.pass_context
def member_show(ctx, member_id, full, json_output):
    """Show one member's detail (registry read; no tmux required)."""
    fleet_id = ctx.obj["fleet_id"]
    target, _placement = _load_authorized_member(
        fleet_id,
        member_id,
        allow_missing_placement=True,
    )
    if json_output:
        click.echo(output.format_json(target))
    else:
        click.echo(output.format_member_detail(target, full=full))


@member.command("list")
@fleet_id_option
@json_flag
@click.pass_context
def member_list(ctx, json_output):
    """List every active registry entry of the fleet."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        rows = broker.list_members(fleet_id)
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc
    if json_output:
        click.echo(output.format_json(rows))
    else:
        click.echo(output.format_member_list(rows))


@member.command("capture")
@fleet_id_option
@member_id_option
@click.option(
    "--lines",
    "lines",
    type=int,
    default=20,
    show_default=True,
    help="Lines to capture.",
)
@click.option(
    "--ansi/--no-ansi",
    default=False,
    help="Emit raw ANSI instead of stripping it.",
)
@json_flag
@click.pass_context
def member_capture(ctx, member_id, lines, ansi, json_output):
    """Capture the last N lines of a member pane's terminal buffer."""
    fleet_id = ctx.obj["fleet_id"]

    mux = ensure_multiplexer_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["member_id"]
    pane_id = _require_member_pane(placement, member_id, "capture")

    try:
        content = mux.capture_pane(target_pane_id=pane_id, lines=lines)
    except MultiplexerError as exc:
        raise click.ClickException(f"capture failed: {exc}") from exc

    if not ansi:
        content = output.strip_ansi(content)

    if json_output:
        click.echo(
            output.format_json(
                {
                    "member_id": member_id,
                    "pane_id": pane_id,
                    "lines": lines,
                    "content": content,
                },
            )
        )
    else:
        # color=True preserves ANSI escape sequences on non-TTY sinks (e.g.
        # CliRunner-captured stdout). Without it, click.echo would re-strip
        # the escapes the operator just opted into via --ansi.
        click.echo(content, nl=False, color=True if ansi else None)


@member.command("exec")
@fleet_id_option
@member_id_option
@click.argument("command")
@json_flag
@click.pass_context
def member_exec(ctx, member_id, command, json_output):
    """Dispatch a shell command via the coding agent's `!` shortcut."""
    fleet_id = ctx.obj["fleet_id"]

    if "\n" in command or "\r" in command:
        raise click.UsageError("command may not contain newlines.")
    if not command.strip():
        raise click.UsageError("command may not be empty.")
    command = command.strip()

    mux = ensure_multiplexer_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["member_id"]
    pane_id = _require_member_pane(placement, member_id, "exec")

    try:
        mux.send_bash_command(target_pane_id=pane_id, command=command)
    except MultiplexerError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc

    if json_output:
        click.echo(
            output.format_json(
                {
                    "member_id": member_id,
                    "pane_id": pane_id,
                    "command": command,
                },
            )
        )
    else:
        click.echo(
            f"Sent bash command {command!r} to member {target['name']} ({pane_id})."
        )


@member.command("ping")
@fleet_id_option
@member_id_option
@quiet_flag
@json_flag
@click.pass_context
def member_ping(ctx, member_id, quiet, json_output):
    """Inject an inbox-poll keystroke into a member's pane (Director-only)."""
    fleet_id = ctx.obj["fleet_id"]

    mux = ensure_multiplexer_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["member_id"]
    pane_id = _require_member_pane(placement, member_id, "ping")

    try:
        ok = mux.send_poll_trigger(
            target_pane_id=pane_id,
            fleet_id=fleet_id,
            member_id=member_id,
        )
    except MultiplexerError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc
    if not ok:
        raise click.ClickException(
            f"send failed: tmux send-keys did not deliver the poll-trigger "
            f"keystroke to pane {pane_id}."
        )

    if json_output:
        click.echo(
            output.format_json(
                {
                    "member_id": member_id,
                    "pane_id": pane_id,
                },
            )
        )
    elif quiet:
        click.echo(str(member_id))
    else:
        click.echo(
            f"Pinged member {target['name']} ({pane_id}) — poll keystroke dispatched."
        )
