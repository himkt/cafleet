"""Per-subcommand ``--help`` line budgets (design doc 0000049, Surface 19,
Step 17).

Step 17 audits every ``@click.option(help=...)`` in ``cli.py`` (~80 sites)
and reduces multi-sentence helps to single phrases. Narrative explanation
lives in ``docs/spec/cli-options.md`` instead. The budgets below are tight
enough that any subcommand still carrying a wrapped multi-line option help
will fail until the trim lands.

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


# Per-subcommand line budgets. These are intentionally below the current
# (pre-Step-17) line counts; Phase B's trim of multi-sentence option helps
# is what brings each below its budget.
_PER_SUBCOMMAND_BUDGETS: dict[tuple[str, ...], int] = {
    ("message", "send"): 9,
    ("message", "broadcast"): 8,
    ("message", "poll"): 8,
    ("message", "ack"): 9,
    ("message", "cancel"): 9,
    ("message", "show"): 8,
    ("member", "create"): 11,
    ("member", "list"): 7,
    ("member", "capture"): 9,
    ("member", "send-input"): 9,
    ("member", "ping"): 8,
    ("member", "exec"): 8,
    ("member", "delete"): 9,
    ("agent", "register"): 9,
    ("agent", "list"): 8,
    ("agent", "show"): 8,
    ("agent", "deregister"): 7,
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
    ``_PER_SUBCOMMAND_BUDGETS`` MUST cut by ≥ 40 % from the pre-Step-17
    baseline. The 4500-byte budget below is sized accordingly: every
    multi-line option help in the table above MUST collapse to a single
    line for this to pass."""
    total_bytes = sum(
        len("\n".join(_help_lines(*subcommand)).encode("utf-8"))
        for subcommand in _PER_SUBCOMMAND_BUDGETS
    )
    assert total_bytes <= 4500, (
        f"aggregate --help bytes = {total_bytes} (budget 4500). "
        f"Trim multi-sentence option helps in cli.py to fit."
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
    summary lines (one per leaf). Regression guard, NOT a trim driver."""
    lines = _help_lines("member")
    assert len(lines) <= 16
