"""Tests for ``cafleet.bash_routing.match_allow`` (Step 4 task 4).

Per design doc §4: the matcher applies an allow×deny truth table against
``Bash(...)`` patterns from the resolved ``permissions.allow`` /
``permissions.deny`` lists.

Truth table (§4):
    allow=yes deny=no  → ``auto-run``
    allow=yes deny=yes → ``ask`` (deny match overrides)
    allow=no  deny=yes → ``ask``
    allow=no  deny=no  → ``ask``

There is no ``auto-deny`` outcome — ``permissions.deny`` only downgrades
``auto-run`` to ``ask``; the operator can still approve via AskUserQuestion.

Pattern syntax: fnmatch-style globs INSIDE the ``Bash(...)`` wrapper. The
matcher only considers patterns that START with ``Bash(...)`` — other tool
patterns (``Edit``, ``Skill(...)``, etc.) are ignored.
"""

from cafleet.bash_routing import match_allow


class TestMatchAllowTruthTable:
    """All four cells of the §4 truth table, one row each."""

    def test_allow_yes_deny_no_returns_auto_run(self):
        result = match_allow(
            cmd="git log -1",
            allow_patterns=["Bash(git *)"],
            deny_patterns=[],
        )
        assert result == "auto-run"

    def test_allow_yes_deny_yes_returns_ask(self):
        result = match_allow(
            cmd="git push origin main",
            allow_patterns=["Bash(git *)"],
            deny_patterns=["Bash(git push *)"],
        )
        assert result == "ask"

    def test_allow_no_deny_yes_returns_ask(self):
        result = match_allow(
            cmd="rm -rf /tmp/x",
            allow_patterns=["Bash(git *)"],
            deny_patterns=["Bash(rm *)"],
        )
        assert result == "ask"

    def test_allow_no_deny_no_returns_ask(self):
        result = match_allow(
            cmd="whoami",
            allow_patterns=["Bash(git *)"],
            deny_patterns=["Bash(rm *)"],
        )
        assert result == "ask"


class TestMatchAllowEmpty:
    def test_empty_allow_and_deny_returns_ask(self):
        result = match_allow(cmd="git log -1", allow_patterns=[], deny_patterns=[])
        assert result == "ask"

    def test_empty_allow_only_returns_ask_even_if_deny_misses(self):
        result = match_allow(
            cmd="echo hi",
            allow_patterns=[],
            deny_patterns=["Bash(rm *)"],
        )
        assert result == "ask"


class TestMatchAllowGlobPatterns:
    """Per §4 pattern-syntax table."""

    def test_bash_git_star_matches_git_log(self):
        assert match_allow(
            cmd="git log -1",
            allow_patterns=["Bash(git *)"],
            deny_patterns=[],
        ) == "auto-run"

    def test_bash_git_star_matches_git_push(self):
        assert match_allow(
            cmd="git push origin main",
            allow_patterns=["Bash(git *)"],
            deny_patterns=[],
        ) == "auto-run"

    def test_bash_git_star_does_not_match_gh_pr_view(self):
        # token-0 differs (``gh`` not ``git``), so the glob ``git *`` does
        # not match ``gh pr view``.
        assert match_allow(
            cmd="gh pr view",
            allow_patterns=["Bash(git *)"],
            deny_patterns=[],
        ) == "ask"

    def test_bash_cafleet_star_matches_cafleet_message_poll(self):
        assert match_allow(
            cmd="cafleet --session-id abc message poll --agent-id def",
            allow_patterns=["Bash(cafleet *)"],
            deny_patterns=[],
        ) == "auto-run"

    def test_bash_mise_cafleet_prefix_matches_test(self):
        assert match_allow(
            cmd="mise //cafleet:test",
            allow_patterns=["Bash(mise //cafleet*)"],
            deny_patterns=[],
        ) == "auto-run"

    def test_bash_mise_cafleet_prefix_matches_lint(self):
        assert match_allow(
            cmd="mise //cafleet:lint",
            allow_patterns=["Bash(mise //cafleet*)"],
            deny_patterns=[],
        ) == "auto-run"

    def test_bash_star_matches_everything(self):
        assert match_allow(
            cmd="any random command here",
            allow_patterns=["Bash(*)"],
            deny_patterns=[],
        ) == "auto-run"


class TestMatchAllowIgnoresNonBashPatterns:
    """The matcher only considers patterns that start with ``Bash(...)``.
    Other tool patterns (``Edit``, ``Write``, ``Skill(...)``, etc.) are
    ignored for both allow and deny.
    """

    def test_edit_pattern_in_allow_ignored(self):
        result = match_allow(
            cmd="git log -1",
            allow_patterns=["Edit", "Write"],
            deny_patterns=[],
        )
        assert result == "ask"

    def test_skill_pattern_in_allow_ignored(self):
        result = match_allow(
            cmd="git log -1",
            allow_patterns=["Skill(cafleet)"],
            deny_patterns=[],
        )
        assert result == "ask"

    def test_edit_pattern_in_deny_ignored(self):
        # An ``Edit`` deny pattern must not affect a ``Bash`` allow match.
        result = match_allow(
            cmd="git log -1",
            allow_patterns=["Bash(git *)"],
            deny_patterns=["Edit"],
        )
        assert result == "auto-run"

    def test_mixed_patterns_only_bash_considered(self):
        result = match_allow(
            cmd="cafleet --session-id abc message poll",
            allow_patterns=["Edit", "Write", "Bash(cafleet *)", "Skill(cafleet)"],
            deny_patterns=["Edit"],
        )
        assert result == "auto-run"


class TestMatchAllowDenyPrecedence:
    """Per §4: a deny match downgrades a would-be auto-run to ask, even
    when a broad allow pattern (e.g. ``Bash(*)``) would otherwise match.
    """

    def test_broad_allow_with_specific_deny_returns_ask(self):
        result = match_allow(
            cmd="rm -rf /tmp/x",
            allow_patterns=["Bash(*)"],
            deny_patterns=["Bash(rm *)"],
        )
        assert result == "ask"

    def test_specific_allow_overlapping_with_specific_deny_returns_ask(self):
        result = match_allow(
            cmd="git push origin main",
            allow_patterns=["Bash(git push *)"],
            deny_patterns=["Bash(git push *)"],
        )
        assert result == "ask"

    def test_deny_only_matches_unrelated_command_does_not_block(self):
        # Deny pattern does NOT match the command, so it cannot downgrade.
        result = match_allow(
            cmd="git log -1",
            allow_patterns=["Bash(git *)"],
            deny_patterns=["Bash(rm *)"],
        )
        assert result == "auto-run"


class TestMatchAllowMultiplePatterns:
    def test_first_matching_allow_pattern_wins(self):
        result = match_allow(
            cmd="git log -1",
            allow_patterns=["Bash(rm *)", "Bash(git *)", "Bash(echo *)"],
            deny_patterns=[],
        )
        assert result == "auto-run"

    def test_no_matching_allow_pattern_in_list_returns_ask(self):
        result = match_allow(
            cmd="whoami",
            allow_patterns=["Bash(git *)", "Bash(rm *)", "Bash(echo *)"],
            deny_patterns=[],
        )
        assert result == "ask"


class TestMatchAllowProjectPatterns:
    """Sanity check against the project's existing ``permissions.allow``
    patterns from `.claude/rules/commands.md`. These shapes drive the
    real-world auto-allow path.
    """

    def test_cafleet_session_id_message_poll_auto_runs(self):
        result = match_allow(
            cmd="cafleet --session-id 550e8400-e29b-41d4-a716-446655440000 message poll --agent-id abc",
            allow_patterns=["Bash(cafleet *)"],
            deny_patterns=[],
        )
        assert result == "auto-run"

    def test_mise_cafleet_test_auto_runs(self):
        result = match_allow(
            cmd="mise //cafleet:test",
            allow_patterns=["Bash(mise //cafleet*)"],
            deny_patterns=[],
        )
        assert result == "auto-run"

    def test_git_mv_auto_runs(self):
        result = match_allow(
            cmd="git mv old new",
            allow_patterns=["Bash(git mv *)"],
            deny_patterns=[],
        )
        assert result == "auto-run"
