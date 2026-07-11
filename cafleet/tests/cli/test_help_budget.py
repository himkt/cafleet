"""Per-subcommand ``--help`` line budgets.

These budgets keep every ``@click.option(help=...)`` in the ``cli/``
package (~80 sites) reduced to single phrases. Narrative explanation lives in
``docs/spec/cli-options.md`` instead. The budgets below are tight enough
that any subcommand carrying a wrapped multi-line option help will fail.

Why per-subcommand line counts (not aggregate token bytes only)? Agents that
mistype a flag and fall into ``--help`` pay the per-subcommand cost, not the
aggregate. The per-subcommand cap is what they actually feel, so it is the
budget that drives the trim where it matters.
"""

import pytest
from click.testing import CliRunner

from cafleet.cli import cli


def _help_lines(*subcommand_path: str) -> list[str]:
    """Run ``cafleet <subcommand_path...> --help`` and return its stdout
    split into lines (trailing newline removed). Always uses CliRunner so
    no real subprocess is spawned."""
    runner = CliRunner()
    result = runner.invoke(cli, [*subcommand_path, "--help"])
    assert result.exit_code == 0, (
        f"--help itself failed for {' '.join(subcommand_path)!r}: "
        f"exit={result.exit_code}, output={result.output!r}"
    )
    return result.output.rstrip("\n").splitlines()


# Per-subcommand line budgets. Each command's --help output must stay at or
# below its budget; option helps are kept to a single concise sentence.
_PER_SUBCOMMAND_BUDGETS: dict[tuple[str, ...], int] = {
    ("message", "send"): 14,
    ("message", "broadcast"): 12,
    ("message", "poll"): 10,
    ("message", "ack"): 12,
    ("message", "cancel"): 11,
    ("message", "show"): 11,
    # member create's wide option column (the --coding-agent
    # [claude|codex|opencode] metavar) wraps the --fleet-id help to two lines,
    # so it gains 2 lines rather than 1. Added --text/--text-file and the
    # now-visible --full increase this.
    ("member", "create"): 20,
    ("member", "list"): 10,
    ("member", "show"): 10,
    ("member", "capture"): 11,
    ("member", "ping"): 10,
    ("member", "exec"): 9,
    ("member", "delete"): 10,
}


@pytest.mark.parametrize(
    ("subcommand_path", "budget"),
    list(_PER_SUBCOMMAND_BUDGETS.items()),
    ids=lambda x: " ".join(x) if isinstance(x, tuple) else str(x),
)
def test_subcommand_help_within_line_budget(
    subcommand_path: tuple[str, ...], budget: int
):
    """Each subcommand's ``--help`` output MUST fit in its line budget.
    A failure means an option's help string still wraps to a second line —
    Phase B trims those to single phrases."""
    lines = _help_lines(*subcommand_path)
    assert len(lines) <= budget, (
        f"`cafleet {' '.join(subcommand_path)} --help` rendered "
        f"{len(lines)} lines (budget {budget}). "
        f"output:\n{chr(10).join(lines)}"
    )


def test_aggregate_help_under_byte_budget():
    """Aggregate ``--help`` byte cost across every subcommand listed in
    ``_PER_SUBCOMMAND_BUDGETS`` stays under a fixed byte budget: every
    multi-line option help in the table above MUST collapse to a single
    line for this to pass.

    The budget accommodates the ``type=int`` id options, whose Click metavar
    renders as ``INTEGER`` rather than ``TEXT`` (a few bytes wider per id
    option across the table)."""
    total_bytes = sum(
        len("\n".join(_help_lines(*subcommand)).encode("utf-8"))
        for subcommand in _PER_SUBCOMMAND_BUDGETS
    )
    assert total_bytes <= 6500, (
        f"aggregate --help bytes = {total_bytes} (budget 6500). "
        f"Trim multi-sentence option helps in the cli/ package to fit."
    )


def test_root_help_does_not_regress_above_baseline():
    """``cafleet --help`` line count is structural (1 line per option +
    1 per leaf command), so Step 17 cannot shrink it materially. This is
    a pure regression guard: a new top-level option or command without a
    corresponding bump to the budget below will fail this test and force
    an explicit decision."""
    lines = _help_lines()
    assert len(lines) <= 20


def test_message_group_help_does_not_regress_above_baseline():
    """``cafleet message --help`` is dominated by the per-leaf command
    summary lines (one per leaf). Regression guard, NOT a trim driver."""
    lines = _help_lines("message")
    assert len(lines) <= 14


def test_member_group_help_does_not_regress_above_baseline():
    """``cafleet member --help`` is dominated by the per-leaf command
    summary lines (one per leaf). Regression guard, NOT a trim driver;
    the budget carries one line for the ``show`` leaf."""
    lines = _help_lines("member")
    assert len(lines) <= 17
