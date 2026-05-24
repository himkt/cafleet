"""Tests for design-0000069 Step 5: source-code marker rename.

Step 5 changes two source literals so they reference the renamed
``cafleet-base-dir`` skill via the canonical tool-agnostic phrasing
(``the `cafleet-base-dir` skill``) instead of the old Claude-Code-only
``Skill(cafleet:base-dir)`` form:

- ``cafleet.base_dir._BASE_INSERT_MARKER`` — the ``[INSERT ...]`` marker
  the Director substitutes into every spawn-prompt template.
- ``cafleet.cli._read_prompt_file`` — the ``--prompt-file`` relative-path
  rejection error the end user sees from ``cafleet member create``.

The existing ``tests/test_base_dir_spawn_flow.py`` ``BASE_MARKER`` constant
moves in lockstep with ``_BASE_INSERT_MARKER`` (Step 5 task 3); the
consistency check below guards that invariant.
"""

import click
import pytest

from cafleet.base_dir import _BASE_INSERT_MARKER
from cafleet.cli import _read_prompt_file

from tests.test_base_dir_spawn_flow import BASE_MARKER as SPAWN_FLOW_BASE_MARKER


EXPECTED_BASE_MARKER = (
    "[INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]"
)
EXPECTED_CLI_ERROR_FRAGMENT = "see the `cafleet-base-dir` skill."


def test_base_insert_marker_uses_tool_agnostic_phrasing():
    """``_BASE_INSERT_MARKER`` references the renamed skill via canonical phrasing.

    The marker is the source of truth that flows into every consumer skill's
    ``BASE: [INSERT ...]`` line; Director substitution and consumer parsing
    both anchor on this literal. After Step 5 it MUST match the new form.
    """
    assert _BASE_INSERT_MARKER == EXPECTED_BASE_MARKER


def test_base_insert_marker_drops_old_claude_code_slash_form():
    """The legacy ``Skill(cafleet:base-dir)`` substring no longer appears in the marker."""
    assert "Skill(cafleet:base-dir)" not in _BASE_INSERT_MARKER
    assert "Skill(" not in _BASE_INSERT_MARKER


def test_spawn_flow_base_marker_constant_matches_runtime_marker():
    """``tests.test_base_dir_spawn_flow.BASE_MARKER`` stays in lockstep with the runtime.

    Step 5 task 3 explicitly requires the test-file constant on line 42 to
    track ``_BASE_INSERT_MARKER`` verbatim — otherwise the spawn-flow tests
    drift away from the runtime contract.
    """
    assert SPAWN_FLOW_BASE_MARKER == _BASE_INSERT_MARKER
    assert SPAWN_FLOW_BASE_MARKER == EXPECTED_BASE_MARKER


def test_read_prompt_file_relative_path_error_points_at_renamed_skill():
    """``_read_prompt_file`` rejects relative paths with a pointer at the renamed skill.

    The error body keeps its existing ``--prompt-file requires an absolute
    path`` lead-in and gains the new tool-agnostic pointer ``see the
    `cafleet-base-dir` skill.``. The legacy ``see Skill(cafleet:base-dir).``
    form is gone.
    """
    with pytest.raises(click.UsageError) as exc_info:
        _read_prompt_file("./foo.md")
    message = str(exc_info.value)
    assert "--prompt-file requires an absolute path" in message
    assert "./foo.md" in message
    assert EXPECTED_CLI_ERROR_FRAGMENT in message
    assert "Skill(cafleet:base-dir)" not in message
