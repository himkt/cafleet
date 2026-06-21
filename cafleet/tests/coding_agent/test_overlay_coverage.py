"""Static coverage guard for the coding-agent overlay token contract.

Design 0000107 Part 5 specifies a static data-contract checker that keeps
the overlay / base / template token set coherent so the resolve step always
has complete data. These tests are the executable specification for that
checker (``cafleet.coding_agent.overlay_coverage``), which both this pytest
module and the ``mise //cafleet:lint-overlay`` task wrap.

Three checks (Part 5):

1. **Token coverage** — every token in the base token universe (matches of
   ``\\{[a-z_]+\\}`` in base files, minus the meta-token ignore-set) is in the
   canonical 8-token set, is defined in all three overlay value tables, and
   has a row in the base default table.
2. **No orphan tokens** — every token defined in an overlay value table or in
   the default table appears in at least one base file.
3. **Note-binding integrity** — every overlay note's *applies-at* anchor names
   a canonical token, every cited ``*.md`` note path resolves to an existing
   file under the skill root, and the note-anchor *token set* (tokens carrying
   ≥ 1 note) is identical across the three overlays. Per-token note count may
   differ; the check compares the set, not the multiset.

The pure-set check functions (``token_coverage_violations`` etc.) are exercised
with crafted inputs so a no-op implementation cannot pass; the real-repo
aggregate (``check_overlay_coverage()``) asserts the live skill tree is
coherent and that legitimate base ``{placeholder}`` / ``{token}`` meta-tokens
are never flagged (SC5).
"""

from cafleet.coding_agent.overlay_coverage import (
    CANONICAL_TOKENS,
    META_TOKENS,
    base_token_universe,
    check_overlay_coverage,
    extract_tokens,
    note_anchor_violations,
    note_path_violations,
    orphan_token_violations,
    token_coverage_violations,
)

# The canonical 8 resolvable tokens (Part 5), written with braces to match
# the design's notation. These are the only tokens an overlay resolves.
_EXPECTED_CANONICAL = frozenset(
    {
        "{decision_surface}",
        "{monitor_model}",
        "{permission_flags}",
        "{bg_run}",
        "{bg_stop}",
        "{task_coord}",
        "{pane_title}",
        "{skill_loader}",
    }
)

# Documentation meta-tokens: they name the token *mechanism* in prose, not a
# resolvable value. The checker excludes them from every check via this set.
_EXPECTED_META = frozenset({"{placeholder}", "{token}"})

# The note-anchor token set every overlay must share (Part 3 / Part 5): the
# two tokens that carry caveats on every backend.
_EXPECTED_NOTE_ANCHORS = frozenset({"{decision_surface}", "{task_coord}"})

_BACKENDS = ("claude", "codex", "opencode")


def _all_eight() -> set[str]:
    return set(_EXPECTED_CANONICAL)


def _overlays_all_defining_eight() -> dict[str, set[str]]:
    return {backend: _all_eight() for backend in _BACKENDS}


# --------------------------------------------------------------------------
# Canonical constants
# --------------------------------------------------------------------------


def test_canonical_token_set_is_the_eight_resolvable_tokens():
    """The checker's canonical set is exactly the 8 resolvable tokens."""
    assert set(CANONICAL_TOKENS) == set(_EXPECTED_CANONICAL)


def test_meta_token_ignore_set_is_placeholder_and_token():
    """``{placeholder}`` and ``{token}`` are the documentation meta-tokens
    excluded from every check."""
    assert set(META_TOKENS) == set(_EXPECTED_META)


def test_canonical_and_meta_sets_are_disjoint():
    assert not (set(CANONICAL_TOKENS) & set(META_TOKENS))


# --------------------------------------------------------------------------
# Token extraction (regex ``\{[a-z_]+\}``)
# --------------------------------------------------------------------------


def test_extract_tokens_matches_brace_lowercase_underscore():
    text = "use {monitor_model} and {task_coord} here"
    assert extract_tokens(text) == {"{monitor_model}", "{task_coord}"}


def test_extract_tokens_includes_meta_tokens_verbatim():
    """Extraction is pure: it does not subtract meta-tokens (the universe
    builder does that). A literal ``{placeholder}`` IS a brace match."""
    assert extract_tokens("a {placeholder} and a {token}") == {
        "{placeholder}",
        "{token}",
    }


def test_extract_tokens_ignores_uppercase_and_hyphen_and_camelcase():
    """The regex is ``\\{[a-z_]+\\}`` — uppercase, hyphens, digits, and
    CamelCase brace spans are NOT tokens."""
    text = "{MonitorModel} {monitor-model} {model2} {Foo_Bar}"
    assert extract_tokens(text) == set()


def test_extract_tokens_empty_when_no_braces():
    assert extract_tokens("plain prose, no tokens") == set()


# --------------------------------------------------------------------------
# Check 1 — token coverage (pure logic)
# --------------------------------------------------------------------------


def test_token_coverage_passes_when_every_token_defined_everywhere():
    violations = token_coverage_violations(
        _all_eight(), _overlays_all_defining_eight(), _all_eight()
    )
    assert violations == []


def test_token_coverage_flags_token_missing_from_one_overlay():
    overlays = _overlays_all_defining_eight()
    overlays["codex"].discard("{monitor_model}")
    violations = token_coverage_violations(_all_eight(), overlays, _all_eight())
    assert violations
    assert any("{monitor_model}" in v and "codex" in v for v in violations), violations


def test_token_coverage_flags_token_missing_from_default_table():
    default_tokens = _all_eight()
    default_tokens.discard("{pane_title}")
    violations = token_coverage_violations(
        _all_eight(), _overlays_all_defining_eight(), default_tokens
    )
    assert violations
    assert any("{pane_title}" in v for v in violations), violations


def test_token_coverage_flags_unknown_resolvable_base_token():
    """A brace match in a base file outside both the canonical set and the
    ignore-set fails the check (a new resolvable token lacking coverage)."""
    base_universe = _all_eight() | {"{rogue_token}"}
    violations = token_coverage_violations(
        base_universe, _overlays_all_defining_eight(), _all_eight()
    )
    assert violations
    assert any("{rogue_token}" in v for v in violations), violations


# --------------------------------------------------------------------------
# Check 2 — no orphan tokens (pure logic)
# --------------------------------------------------------------------------


def test_no_orphan_when_overlay_and_default_tokens_all_in_base():
    violations = orphan_token_violations(
        _all_eight(), _overlays_all_defining_eight(), _all_eight()
    )
    assert violations == []


def test_orphan_flagged_when_overlay_defines_token_absent_from_base():
    """A token left in an overlay after removal from the base is an orphan."""
    overlays = _overlays_all_defining_eight()
    overlays["claude"].add("{ghost_token}")
    violations = orphan_token_violations(_all_eight(), overlays, _all_eight())
    assert violations
    assert any("{ghost_token}" in v for v in violations), violations


def test_orphan_flagged_when_default_table_defines_token_absent_from_base():
    default_tokens = _all_eight() | {"{ghost_token}"}
    violations = orphan_token_violations(
        _all_eight(), _overlays_all_defining_eight(), default_tokens
    )
    assert violations
    assert any("{ghost_token}" in v for v in violations), violations


# --------------------------------------------------------------------------
# Check 3 — note-binding integrity (pure logic)
# --------------------------------------------------------------------------


def test_note_anchors_pass_when_identical_across_overlays():
    anchors = {backend: set(_EXPECTED_NOTE_ANCHORS) for backend in _BACKENDS}
    assert note_anchor_violations(anchors) == []


def test_note_anchors_pass_despite_differing_per_token_note_counts():
    """The check compares the token *set*, not the multiset — claude carries
    extra ``{decision_surface}`` rows, which collapse to the same set."""
    anchors = {
        "claude": {"{decision_surface}", "{task_coord}"},
        "codex": {"{decision_surface}", "{task_coord}"},
        "opencode": {"{decision_surface}", "{task_coord}"},
    }
    assert note_anchor_violations(anchors) == []


def test_note_anchors_flagged_when_a_backend_drops_an_anchor():
    anchors = {
        "claude": {"{decision_surface}", "{task_coord}"},
        "codex": {"{decision_surface}"},
        "opencode": {"{decision_surface}", "{task_coord}"},
    }
    violations = note_anchor_violations(anchors)
    assert violations
    assert any("{task_coord}" in v for v in violations), violations


def test_note_anchors_flagged_when_anchor_is_not_a_canonical_token():
    anchors = {backend: {"{decision_surface}"} for backend in _BACKENDS}
    anchors["claude"] = {"{decision_surface}", "{not_a_token}"}
    violations = note_anchor_violations(anchors)
    assert violations
    assert any("{not_a_token}" in v for v in violations), violations


# --------------------------------------------------------------------------
# Check 3 — note-binding file existence (pure logic)
# --------------------------------------------------------------------------


def test_note_path_flags_cited_md_path_missing_under_skill_root(tmp_path):
    """A note that cites a ``*.md`` path with no file under the skill root is
    a violation that names the missing path; an existing path is not."""
    existing = tmp_path / "cafleet" / "SKILL.md"
    existing.parent.mkdir(parents=True)
    existing.write_text("# stub\n")
    cited_paths = {
        "claude": {"cafleet/SKILL.md", "cafleet/reference/gone.md"},
    }
    violations = note_path_violations(cited_paths, tmp_path)
    assert violations
    assert any("cafleet/reference/gone.md" in v for v in violations), violations
    assert not any("cafleet/SKILL.md" in v for v in violations), violations


def test_note_path_passes_when_every_cited_path_exists(tmp_path):
    for rel in ("cafleet/SKILL.md", "cafleet-research/report/report.md"):
        path = tmp_path / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("# stub\n")
    cited_paths = {
        "claude": {"cafleet/SKILL.md"},
        "codex": {"cafleet-research/report/report.md"},
        "opencode": {"cafleet-research/report/report.md"},
    }
    assert note_path_violations(cited_paths, tmp_path) == []


# --------------------------------------------------------------------------
# Real-repo aggregate — the live invariant CI enforces
# --------------------------------------------------------------------------


def test_meta_tokens_excluded_from_base_token_universe():
    """SC5: legitimate base ``{placeholder}`` / ``{token}`` meta-tokens are
    never treated as resolvable tokens."""
    universe = base_token_universe()
    assert not (universe & set(META_TOKENS)), (
        f"meta-tokens leaked into the token universe: {universe & set(META_TOKENS)}"
    )


def test_all_canonical_tokens_appear_in_base_token_universe():
    """Every canonical token is actually used in a base file (so coverage is
    not vacuous)."""
    universe = base_token_universe()
    missing = set(CANONICAL_TOKENS) - universe
    assert not missing, f"canonical tokens absent from base files: {missing}"


def test_base_token_universe_has_no_unknown_resolvable_tokens():
    """The live base files introduce no resolvable token outside the canonical
    set (a new one would need overlay + default coverage first)."""
    universe = base_token_universe()
    unknown = universe - set(CANONICAL_TOKENS)
    assert not unknown, f"unknown resolvable tokens in base files: {unknown}"


def test_real_repo_overlay_coverage_is_clean():
    """The aggregate checker — what ``mise //cafleet:lint-overlay`` wraps —
    reports no violations against the live skill tree."""
    violations = check_overlay_coverage()
    assert violations == [], "overlay coverage violations:\n" + "\n".join(violations)
