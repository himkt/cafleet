"""``cafleet member`` — tmux-backed member agent commands (Director only)."""

import contextlib
import os
from typing import NoReturn

import click

from cafleet import broker, output
from cafleet.broker import _shared
from cafleet.cli._helpers import (
    director_member_options,
    ensure_tmux_or_die,
    fleet_id_option,
    full_flag,
    quiet_flag,
)
from cafleet.cli._prompt import resolve_prompt
from cafleet.coding_agent import CODING_AGENTS
from cafleet.multiplexer import MULTIPLEXERS, TmuxError


@click.group()
def member():
    """Manage tmux-backed member agents (Director only)."""


_PLACEMENT_MISSING_DEFAULT = (
    "agent {member_id} has no placement row; it was not "
    "spawned via `cafleet member create`."
)


def _require_member_pane(placement: dict, member_id: int, action: str) -> str:
    pane_id = placement["tmux_pane_id"]
    if pane_id is None:
        raise click.ClickException(
            f"member {member_id} has no pane yet (pending placement) "
            f"— nothing to {action}."
        )
    return pane_id


def _load_authorized_member(
    fleet_id: int,
    member_id: int,
    *,
    placement_missing_template: str = _PLACEMENT_MISSING_DEFAULT,
) -> tuple[dict, dict]:
    """Resolve a fleet-scoped member's agent + placement.

    The only boundary is fleet isolation: ``broker.get_agent(member_id,
    fleet_id)`` returns ``None`` for a ``member_id`` that is not an active agent
    of ``fleet_id``, so a cross-fleet / unknown / inactive target raises "not
    found". There is no caller-auth check — any active in-fleet agent with a
    placement row (the root Director included) is a valid target; an in-fleet
    agent without a placement row raises the placement-missing error below.

    ``placement_missing_template`` is a ``{member_id}`` format string for the
    "no placement" path, because each caller points users at a different
    follow-up command (``cafleet member create`` by default; ``cafleet agent
    deregister`` from delete). Pane-id presence is NOT checked here — delete
    tolerates a pending placement while the others reject it.

    Callers MUST use ``target["agent_id"]``, since reassigning the
    ``member_id`` local param does not propagate to the caller.
    """
    try:
        target = broker.get_agent(member_id, fleet_id)
    except Exception as exc:
        raise click.ClickException(f"failed to fetch member: {exc}") from exc
    if target is None:
        raise click.ClickException(f"Agent {member_id} not found")
    placement = target["placement"]
    if placement is None:
        raise click.ClickException(
            placement_missing_template.format(member_id=member_id)
        )
    return target, placement


def _deregister_with_warning(new_agent_id: int, *, fleet_id: int) -> None:
    """Best-effort deregister; emit warning to stderr if it fails."""
    try:
        broker.deregister_agent(new_agent_id)
    except Exception as drop_exc:
        click.echo(
            f"WARNING: rollback deregister failed — agent {new_agent_id} is "
            f"orphaned in the registry. Run `cafleet agent deregister "
            f"--fleet-id {fleet_id} --agent-id {new_agent_id}` manually to clean up. "
            f"Cause: {drop_exc}",
            err=True,
        )


def _rollback_register(new_agent_id: int, *, fleet_id: int, reason: str) -> NoReturn:
    """Best-effort deregister of a just-created agent, then raise ClickException."""
    _deregister_with_warning(new_agent_id, fleet_id=fleet_id)
    raise click.ClickException(f"{reason}. Rolled back registration of {new_agent_id}.")


@member.command("create")
@fleet_id_option
@click.option("--agent-id", type=int, required=True, help="Director's agent ID")
@click.option("--name", required=True, help="Member name")
@click.option("--description", required=True, help="Member description")
@click.option(
    "--coding-agent",
    "coding_agent",
    type=click.Choice(list(CODING_AGENTS.keys())),
    default="claude",
    show_default=True,
    help="Coding-agent binary to spawn / declare for the placement.",
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
@click.option(
    "--prompt-file",
    "prompt_file",
    type=str,
    default=None,
    help="Read spawn prompt from FILE (abs path, UTF-8).",
)
@full_flag
@click.argument("prompt_argv", nargs=-1)
@click.pass_context
def member_create(
    ctx,
    agent_id,
    name,
    description,
    coding_agent,
    model,
    role,
    prompt_file,
    full,
    prompt_argv,
):
    """Register a new member and spawn its pane."""
    if prompt_file is not None and prompt_argv:
        raise click.UsageError(
            "--prompt-file and the positional prompt argument are mutually exclusive."
        )
    fleet_id = ctx.obj["fleet_id"]

    agent = CODING_AGENTS[coding_agent]

    try:
        agent.validate_model(model)
    except ValueError as exc:
        raise click.UsageError(str(exc)) from exc

    try:
        MULTIPLEXERS["tmux"].ensure_available()
        agent.ensure_available()
        director_ctx = MULTIPLEXERS["tmux"].context_discovery()
    except (TmuxError, RuntimeError) as exc:
        raise click.ClickException(str(exc)) from exc

    try:
        result = broker.register_agent(
            fleet_id,
            name,
            description,
            placement={
                "director_agent_id": agent_id,
                "tmux_session": director_ctx.session,
                "tmux_window_id": director_ctx.window_id,
                "tmux_pane_id": None,
                "coding_agent": coding_agent,
            },
            kind=_shared.MONITORING_MEMBER_KIND if role == "monitor" else None,
        )
    except click.ClickException:
        # The one-monitoring-member-per-fleet guard raises ClickException with a
        # user-facing message; surface it verbatim rather than wrapping it.
        raise
    except Exception as exc:
        raise click.ClickException(f"register failed: {exc}") from exc
    new_agent_id = result["agent_id"]

    try:
        prompt = resolve_prompt(ctx, agent_id, new_agent_id, prompt_argv, prompt_file)
    except (click.UsageError, click.ClickException):
        # Re-raise unwrapped so the exact message from docs/spec/cli-options.md
        # § Error Messages reaches the operator. Wrapping via _rollback_register
        # would prepend "prompt resolution failed:" and append "Rolled back
        # registration of <id>." (with a stray ".." when the inner message
        # already ends in a period), and would also downgrade UsageError exit
        # code 2 → ClickException exit 1.
        _deregister_with_warning(new_agent_id, fleet_id=fleet_id)
        raise

    spawn_command = agent.build_spawn_argv(prompt, display_name=name, model=model)

    try:
        db_url = os.environ.get("CAFLEET_DATABASE_URL")
        fwd_env = {"CAFLEET_DATABASE_URL": db_url} if db_url else {}
        pane_id = MULTIPLEXERS["tmux"].split_window(
            target_window_id=director_ctx.window_id,
            env=fwd_env,
            command=spawn_command,
        )
    except TmuxError as exc:
        _rollback_register(
            new_agent_id,
            fleet_id=fleet_id,
            reason=f"tmux split-window failed: {exc}",
        )

    try:
        placement_view = broker.update_placement_pane_id(new_agent_id, pane_id)
    except Exception as exc:
        # Pane is alive but the registration row is dangling; /exit the pane
        # and roll back the agent so the caller can retry cleanly.
        with contextlib.suppress(TmuxError):
            MULTIPLEXERS["tmux"].send_exit(target_pane_id=pane_id, ignore_missing=True)
        _rollback_register(
            new_agent_id,
            fleet_id=fleet_id,
            reason=f"placement update failed: {exc}",
        )
    if placement_view is None:
        with contextlib.suppress(TmuxError):
            MULTIPLEXERS["tmux"].send_exit(target_pane_id=pane_id, ignore_missing=True)
        _rollback_register(
            new_agent_id,
            fleet_id=fleet_id,
            reason="placement row vanished before pane-id patch",
        )

    result["placement"] = placement_view
    if ctx.obj["json_output"]:
        click.echo(output.format_json(result))
    else:
        click.echo(output.format_member(result, full=full))


@member.command("delete")
@fleet_id_option
@director_member_options
@click.option(
    "--force",
    "-f",
    "force",
    is_flag=True,
    default=False,
    help="Skip /exit; kill-pane immediately.",
)
@click.pass_context
def member_delete(ctx, member_id, force):
    """Deregister a member agent and close its tmux pane."""
    fleet_id = ctx.obj["fleet_id"]

    ensure_tmux_or_die()

    fleet = broker.get_fleet(fleet_id)
    if fleet is not None and member_id == fleet["director_agent_id"]:
        raise click.ClickException(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead"
        )

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
        placement_missing_template=(
            "agent {member_id} has no placement; use `cafleet agent deregister` instead"
        ),
    )
    member_id = target["agent_id"]
    pane_id = placement["tmux_pane_id"]

    if pane_id is None:
        try:
            broker.deregister_agent(member_id)
        except Exception as exc:
            raise click.ClickException(f"deregister failed: {exc}") from exc
        pane_status = "(pending — no pane)"
        _emit_member_delete_output(
            ctx, member_id, pane_status, header="Member deleted."
        )
        return

    if force:
        try:
            MULTIPLEXERS["tmux"].kill_pane(target_pane_id=pane_id, ignore_missing=True)
        except TmuxError as exc:
            raise click.ClickException(
                f"kill_pane failed for pane {pane_id}: {exc}. "
                f"The tmux server may be unreachable. Verify with 'cafleet doctor', "
                f"then re-run the command."
            ) from exc
        try:
            broker.deregister_agent(member_id)
        except Exception as exc:
            raise click.ClickException(f"deregister failed: {exc}") from exc
        pane_status = f"{pane_id} (killed)"
        _emit_member_delete_output(
            ctx, member_id, pane_status, header="Member deleted (--force)."
        )
        return

    try:
        MULTIPLEXERS["tmux"].send_exit(target_pane_id=pane_id, ignore_missing=True)
    except TmuxError as exc:
        raise click.ClickException(
            f"send_exit failed for pane {pane_id}: {exc}. "
            f"The tmux server may be unreachable. Verify with 'cafleet doctor', "
            f"then re-run 'cafleet member delete', or use '--force' to kill the "
            f"pane directly."
        ) from exc

    try:
        gone = MULTIPLEXERS["tmux"].wait_for_pane_gone(
            target_pane_id=pane_id, timeout=15.0, interval=0.5
        )
    except TmuxError as exc:
        raise click.ClickException(
            f"tmux call failed while waiting for pane {pane_id} to close: {exc}"
        ) from exc

    if gone:
        try:
            broker.deregister_agent(member_id)
        except Exception as exc:
            raise click.ClickException(f"deregister failed: {exc}") from exc
        pane_status = f"{pane_id} (closed)"
        _emit_member_delete_output(
            ctx, member_id, pane_status, header="Member deleted."
        )
        return

    try:
        tail = MULTIPLEXERS["tmux"].capture_pane(target_pane_id=pane_id, lines=80)
    except TmuxError as exc:
        click.echo(
            f"Warning: capture_pane failed during timeout handling: {exc}. "
            f"The timeout error and recovery hint still print.",
            err=True,
        )
        tail = ""

    click.echo(
        f"Error: pane {pane_id} did not close within 15.0s after /exit.", err=True
    )
    click.echo(f"--- pane {pane_id} tail (last 80 lines) ---", err=True)
    click.echo(tail, err=True)
    click.echo("---", err=True)
    click.echo(
        "Recovery: inspect with `cafleet member capture`, answer any prompt with "
        "`cafleet member send-input`, then re-run `cafleet member delete`. "
        "Or re-run with `--force` to skip the wait and kill the pane.",
        err=True,
    )

    pane_status = f"{pane_id} (timeout)"
    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {"agent_id": member_id, "pane_status": pane_status},
            )
        )
    ctx.exit(2)


def _emit_member_delete_output(
    ctx: click.Context,
    member_id: int,
    pane_status: str,
    *,
    header: str,
) -> None:
    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {"agent_id": member_id, "pane_status": pane_status},
            )
        )
    else:
        click.echo(header)
        click.echo(f"  agent_id:  {member_id}")
        click.echo(f"  pane_id:   {pane_status}")


@member.command("list")
@fleet_id_option
@click.option(
    "--activity",
    "activity",
    is_flag=True,
    default=False,
    hidden=True,
)
@click.pass_context
def member_list(ctx, activity):
    """List every member of the fleet (the root Director is excluded)."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        if activity:
            rows = broker.list_members_with_activity(fleet_id)
        else:
            rows = broker.list_members(fleet_id)
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc
    if ctx.obj["json_output"]:
        click.echo(output.format_json(rows))
    elif activity:
        click.echo(output.format_member_list_activity(rows))
    else:
        click.echo(output.format_member_list(rows))


@member.command("capture")
@fleet_id_option
@director_member_options
@click.option(
    "--lines",
    "--tail",
    "lines",
    type=int,
    default=20,
    show_default=True,
    help="Lines to capture (alias: --tail).",
)
@click.option(
    "--ansi/--no-ansi",
    default=False,
    hidden=True,
)
@click.pass_context
def member_capture(ctx, member_id, lines, ansi):
    """Capture the last N lines of a member pane's terminal buffer."""
    fleet_id = ctx.obj["fleet_id"]

    ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["agent_id"]
    pane_id = _require_member_pane(placement, member_id, "capture")

    try:
        content = MULTIPLEXERS["tmux"].capture_pane(target_pane_id=pane_id, lines=lines)
    except TmuxError as exc:
        raise click.ClickException(f"capture failed: {exc}") from exc

    if not ansi:
        content = output.strip_ansi(content)

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
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


@member.command("send-input")
@fleet_id_option
@director_member_options
@click.option(
    "--choice",
    type=click.IntRange(1, 3),
    default=None,
    help="Choice 1/2/3 (xor --freetext).",
)
@click.option(
    "--freetext",
    type=str,
    default=None,
    hidden=True,
)
@click.pass_context
def member_send_input(ctx, member_id, choice, freetext):
    """Safely forward a restricted keystroke to a member pane."""
    fleet_id = ctx.obj["fleet_id"]

    if freetext is not None and freetext.lstrip().startswith("!"):
        raise click.UsageError(
            "--freetext may not start with '!' — that triggers the coding agent's "
            "shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead."
        )

    supplied = sum(1 for v in (choice, freetext) if v is not None)
    if supplied != 1:
        raise click.UsageError(
            "--choice and --freetext are mutually exclusive; supply exactly one."
        )

    if freetext is not None and ("\n" in freetext or "\r" in freetext):
        raise click.UsageError("free text may not contain newlines.")

    ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["agent_id"]
    pane_id = _require_member_pane(placement, member_id, "send")

    try:
        if choice is not None:
            MULTIPLEXERS["tmux"].send_choice_key(target_pane_id=pane_id, digit=choice)
            action, value = "choice", str(choice)
        else:
            MULTIPLEXERS["tmux"].send_freetext_and_submit(
                target_pane_id=pane_id, text=freetext
            )
            action, value = "freetext", freetext
    except TmuxError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                    "action": action,
                    "value": value,
                },
            )
        )
    else:
        label = f"choice {value}" if action == "choice" else "free text"
        click.echo(f"Sent {label} to member {target['name']} ({pane_id}).")


@member.command("exec")
@fleet_id_option
@director_member_options
@click.argument("command")
@click.pass_context
def member_exec(ctx, member_id, command):
    """Dispatch a shell command via the coding agent's `!` shortcut."""
    fleet_id = ctx.obj["fleet_id"]

    if "\n" in command or "\r" in command:
        raise click.UsageError("command may not contain newlines.")
    if not command.strip():
        raise click.UsageError("command may not be empty.")
    command = command.strip()

    ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["agent_id"]
    pane_id = _require_member_pane(placement, member_id, "exec")

    try:
        MULTIPLEXERS["tmux"].send_bash_command(target_pane_id=pane_id, command=command)
    except TmuxError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
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
@director_member_options
@quiet_flag
@click.pass_context
def member_ping(ctx, member_id, quiet):
    """Inject an inbox-poll keystroke into a member's pane (Director-only)."""
    fleet_id = ctx.obj["fleet_id"]

    ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["agent_id"]
    pane_id = _require_member_pane(placement, member_id, "ping")

    try:
        ok = MULTIPLEXERS["tmux"].send_poll_trigger(
            target_pane_id=pane_id,
            fleet_id=fleet_id,
            agent_id=member_id,
        )
    except TmuxError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc
    if not ok:
        raise click.ClickException(
            f"send failed: tmux send-keys did not deliver the poll-trigger "
            f"keystroke to pane {pane_id}."
        )

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
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


@member.command("nudge")
@fleet_id_option
@click.option(
    "--agent-id",
    type=int,
    required=True,
    help="Sender's agent ID (the acting member, typically the monitoring member).",
)
@director_member_options
@click.option("--text", required=True, help="Re-engage summary")
@click.pass_context
def member_nudge(ctx, agent_id, member_id, text):
    """Re-engage a member (typically the Director) with an ACKable task + preview."""
    fleet_id = ctx.obj["fleet_id"]

    if not text.strip():
        raise click.UsageError("text may not be empty.")

    # Resolve the target FIRST (fleet-isolation only): a cross-fleet / unknown /
    # inactive --member-id raises "Agent <id> not found" here, before the send
    # path runs. This also makes send_message's own destination ValueError
    # unreachable in the nudge path — only its sender check can still fire.
    target, placement = _load_authorized_member(fleet_id, member_id)
    member_id = target["agent_id"]

    try:
        result = broker.send_message(fleet_id, agent_id, to=member_id, text=text)
    except ValueError as exc:
        raise click.ClickException(str(exc)) from exc

    task_id = result["task"]["task_id"]
    notification_sent = result["notification_sent"]
    pane_id = placement["tmux_pane_id"]

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                    "task_id": task_id,
                    "notification_sent": notification_sent,
                },
            )
        )
    elif notification_sent:
        click.echo(
            f"Nudged Director {target['name']} ({pane_id}) — task {task_id} queued, "
            f"Esc-safeguarded preview dispatched."
        )
    else:
        click.echo(
            f"Nudged Director {target['name']} — no pane; task {task_id} queued."
        )
