"""Static drift guard for the spawn-prompt / CLI-name reconciliation (design 0000113 S4).

These tests are the executable specification for the checker
(``cafleet.spawn_prompt_guard``), which both this pytest module and the
``mise //cafleet:lint-spawn-guard`` task wrap.

The guard scans the project-local edit surface — the repo ``skills/`` tree,
``docs/``, ``SPEC.md``, ``README.md``, the project-local ``.claude/``
(``rules/`` + ``skills/skill-author/``), and ``CLAUDE.md`` — excluding
``design-docs/`` (the historical record), and returns a violation naming the
file, line, and matched pattern for any occurrence of:

1. ``$CAFLEET_FLEET_ID`` / ``$CAFLEET_AGENT_ID`` / ``$CAFLEET_DIRECTOR_AGENT_ID``
   — dollar-sign identity env refs (the fiction issue #155 removes); the bare
   dollar-less names in the CLI env-var catalog are NOT matched.
2. ``cafleet agent spawn`` — removed command; now ``cafleet member create``.
3. ``cafleet pane `` (with trailing space / subcommand) — removed group; now
   ``cafleet member *``.

The pure scanner (:func:`scan_text`) is exercised with crafted inputs so a
no-op implementation cannot pass; the tree walker
(:func:`check_spawn_prompt_drift`) is exercised against crafted ``tmp_path``
trees for the scan-scope contract, and against the live repo for the "clean"
invariant CI enforces.
"""

from pathlib import Path

from cafleet.spawn_prompt_guard import check_spawn_prompt_drift, scan_text

_DOLLAR_IDENTITY_REFS = (
    "$CAFLEET_FLEET_ID",
    "$CAFLEET_AGENT_ID",
    "$CAFLEET_DIRECTOR_AGENT_ID",
)

_CLEAN_DOC = (
    "Spawn via cafleet member create; identity is substituted by str.format.\n"
    "Lifecycle verbs: cafleet member delete/capture/exec/ping/nudge --member-id.\n"
)

_DIRTY_DOC = "Legacy: cafleet agent spawn --role monitor\n"


def _write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


# --------------------------------------------------------------------------
# scan_text — forbidden patterns (positive)
# --------------------------------------------------------------------------


def test_scan_flags_cafleet_agent_spawn():
    violations = scan_text("docs/x.md", "run cafleet agent spawn --role monitor\n")
    assert len(violations) == 1
    assert "cafleet agent spawn" in violations[0]


def test_scan_flags_cafleet_pane_subcommands():
    for line in (
        "cafleet pane exec --fleet-id 24 --agent-id 88 'ls'",
        "cafleet pane wake --poll-only",
        "cafleet pane capture --fleet-id 24 --agent-id 88",
    ):
        violations = scan_text("docs/x.md", line + "\n")
        assert len(violations) == 1, line
        assert "cafleet pane " in violations[0], line


def test_scan_flags_each_dollar_identity_ref():
    for ref in _DOLLAR_IDENTITY_REFS:
        violations = scan_text(
            "skills/cafleet/SKILL.md", f"poll with --agent-id {ref}\n"
        )
        assert len(violations) == 1, ref
        assert ref in violations[0], ref


def test_scan_matches_inside_inline_code_spans():
    """Substring semantics: a backticked occurrence is still drift."""
    violations = scan_text("docs/x.md", "the old `cafleet agent spawn` command\n")
    assert len(violations) == 1


def test_scan_reports_each_offending_line_with_its_line_number():
    text = "cafleet agent spawn\nclean line\n$CAFLEET_FLEET_ID\n"
    violations = scan_text("docs/x.md", text)
    assert len(violations) == 2
    assert any("docs/x.md:1" in v and "cafleet agent spawn" in v for v in violations), (
        violations
    )
    assert any("docs/x.md:3" in v and "$CAFLEET_FLEET_ID" in v for v in violations), (
        violations
    )


def test_scan_reports_two_patterns_on_one_line_separately():
    """One violation per (line, pattern) occurrence — no silent merging."""
    violations = scan_text("docs/x.md", "cafleet agent spawn reads $CAFLEET_AGENT_ID\n")
    assert len(violations) == 2


def test_scan_violation_names_file_line_and_pattern():
    text = "clean\nalso clean\nlegacy: cafleet pane wake --message hi\n"
    violations = scan_text("docs/how-to/monitor-and-recover.md", text)
    assert len(violations) == 1
    v = violations[0]
    assert "docs/how-to/monitor-and-recover.md:3" in v
    assert "cafleet pane " in v


# --------------------------------------------------------------------------
# scan_text — legitimate text (negative)
# --------------------------------------------------------------------------


def test_scan_passes_member_surface_commands():
    text = (
        "cafleet member create --fleet-id 24 --agent-id 84 --role monitor\n"
        "cafleet member delete --fleet-id 24 --member-id 88\n"
        "cafleet member list --activity\n"
    )
    assert scan_text("docs/x.md", text) == []


def test_scan_passes_dollarless_cafleet_env_names():
    """The bare (dollar-less) names in the CLI env-var catalog stay legal."""
    text = (
        "CAFLEET_FLEET_ID defaults --fleet-id; CAFLEET_DATABASE_URL is forwarded.\n"
        "CAFLEET_AGENT_ID and CAFLEET_DIRECTOR_AGENT_ID as prose names.\n"
    )
    assert scan_text("skills/cafleet/reference/cli.md", text) == []


def test_scan_passes_dollar_database_url():
    """Only the three identity refs are forbidden — not every $CAFLEET_* var."""
    text = "the pane inherits $CAFLEET_DATABASE_URL from the Director\n"
    assert scan_text("docs/x.md", text) == []


def test_scan_passes_registry_agent_commands():
    """agent register/list/show/deregister remain shipped registry commands."""
    text = (
        "cafleet agent register --fleet-id 24 --name tester\n"
        "cafleet agent list --fleet-id 24\n"
        "cafleet agent show --fleet-id 24 --agent-id 88\n"
        "cafleet agent deregister --fleet-id 24 --agent-id 88\n"
    )
    assert scan_text("docs/x.md", text) == []


def test_scan_passes_str_format_identity_placeholders():
    """The four str.format placeholders are the sanctioned mechanism (S1)."""
    text = (
        "FLEET ID: {fleet_id}\n"
        "DIRECTOR AGENT ID: {director_agent_id}\n"
        "YOUR AGENT ID: {agent_id}\n"
        "CODING AGENT: {coding_agent}\n"
    )
    assert scan_text("skills/cafleet/reference/director.md", text) == []


def test_scan_empty_and_plain_text_are_clean():
    assert scan_text("docs/x.md", "") == []
    assert scan_text("docs/x.md", "plain prose about fleet members\n") == []


# --------------------------------------------------------------------------
# check_spawn_prompt_drift — crafted-tree scan scope
# --------------------------------------------------------------------------


def test_check_scans_every_surface_bucket(tmp_path):
    """One dirty file in each in-scope location — every bucket is flagged."""
    dirty = (
        "skills/cafleet/SKILL.md",
        "skills/cafleet-design-doc/execute/roles/tester.md",
        "docs/concepts/member-lifecycle.md",
        "SPEC.md",
        "README.md",
        "CLAUDE.md",
        ".claude/rules/bash-tool.md",
        ".claude/skills/skill-author/SKILL.md",
    )
    for rel in dirty:
        _write(tmp_path, rel, _DIRTY_DOC)
    violations = check_spawn_prompt_drift(tmp_path)
    assert len(violations) == len(dirty), violations
    for rel in dirty:
        assert any(rel in v for v in violations), rel


def test_check_excludes_design_docs_and_out_of_scope_files(tmp_path):
    """design-docs/ is the historical record; the source tree, researches/,
    .claude/settings.json, and non-skill-author .claude/skills/ entries are
    outside the S3/S4 edit surface."""
    _write(tmp_path, "docs/x.md", _CLEAN_DOC)
    for rel in (
        "design-docs/0000113-spawn-prompt-simplification/design-doc.md",
        "researches/topic/report.md",
        ".claude/settings.json",
        ".claude/skills/other-skill/SKILL.md",
        "cafleet/src/cafleet/cli/member.py",
    ):
        _write(tmp_path, rel, _DIRTY_DOC)
    assert check_spawn_prompt_drift(tmp_path) == []


def test_check_clean_tree_returns_no_violations(tmp_path):
    for rel in (
        "skills/cafleet/SKILL.md",
        "docs/concepts/member-lifecycle.md",
        "SPEC.md",
        "README.md",
        "CLAUDE.md",
        ".claude/rules/commands.md",
        ".claude/skills/skill-author/SKILL.md",
    ):
        _write(tmp_path, rel, _CLEAN_DOC)
    assert check_spawn_prompt_drift(tmp_path) == []


def test_check_tolerates_missing_surface_entries(tmp_path):
    """A tree with only part of the surface scans what exists (no crash on a
    missing .claude/ or CLAUDE.md)."""
    _write(tmp_path, "docs/x.md", _DIRTY_DOC)
    violations = check_spawn_prompt_drift(tmp_path)
    assert len(violations) == 1


def test_check_violation_carries_file_line_and_pattern(tmp_path):
    _write(tmp_path, "docs/x.md", "clean\nreads $CAFLEET_DIRECTOR_AGENT_ID\n")
    violations = check_spawn_prompt_drift(tmp_path)
    assert len(violations) == 1
    v = violations[0]
    assert "docs/x.md:2" in v
    assert "$CAFLEET_DIRECTOR_AGENT_ID" in v


# --------------------------------------------------------------------------
# Real-repo aggregate — the live invariant CI enforces
# --------------------------------------------------------------------------


def test_live_tree_is_clean():
    """The aggregate checker — what ``mise //cafleet:lint-spawn-guard`` wraps —
    reports no drift against the live project-local edit surface."""
    violations = check_spawn_prompt_drift()
    assert violations == [], "spawn-prompt drift violations:\n" + "\n".join(violations)
