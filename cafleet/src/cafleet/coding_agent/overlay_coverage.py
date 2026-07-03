"""Static coverage guard for the coding-agent overlay token contract (design 0000107 Part 5).

Keeps the overlay / base / template token set coherent so the resolve step
(``cafleet/SKILL.md`` § *Resolve your overlay*) always has complete data. Three
checks run against the live skill tree:

1. **Token coverage** — every token in the base token universe is canonical, is
   defined in all three overlay value tables, and has a documented-default row.
2. **No orphan tokens** — every overlay/default token appears in a base file.
3. **Note-binding integrity** — every overlay note's *applies-at* anchor is a
   canonical token, the note-anchor token set is identical across overlays, and
   every cited ``*.md`` note path resolves to an existing file.

The pure-set functions (:func:`token_coverage_violations` etc.) take crafted
inputs; the live-tree aggregate (:func:`check_overlay_coverage`) wires the
parsers together and is what ``mise //cafleet:lint-overlay`` runs.
"""

import re
import sys
from pathlib import Path

# The 9 resolvable tokens an overlay materializes (design 0000107 Part 5).
CANONICAL_TOKENS = frozenset(
    {
        "{decision_surface}",
        "{monitor_model}",
        "{reviewer_model}",
        "{permission_flags}",
        "{bg_run}",
        "{bg_stop}",
        "{task_coord}",
        "{pane_title}",
        "{skill_loader}",
    }
)

# Documentation meta-tokens: they name the token *mechanism* in prose (e.g.
# "a literal `{token}` is a defect"), not a resolvable value. Pinned to exactly
# these two by the contract; excluded from the token universe.
META_TOKENS = frozenset({"{placeholder}", "{token}"})

# Non-resolvable brace spans that legitimately appear in base files but are NOT
# overlay-resolved: spawn-prompt ``str.format()`` template fields, fenced-code
# f-strings, LaTeX, and a git ref. Each is a deliberate human classification
# (design Part 5: "a new meta-token that must be added to the ignore-set — a
# deliberate human decision, not a silent pass"), so the universe stays exactly
# the canonical set while the guard still flags any *new* resolvable prose token.
NON_RESOLVABLE_TOKENS = frozenset(
    {
        "{fleet_id}",  # spawn-prompt skeleton field, filled by `member create` str.format()
        "{agent_id}",  # spawn-prompt skeleton field
        "{director_agent_id}",  # spawn-prompt skeleton field
        "{coding_agent}",  # monitor spawn field, resolved by --coding-agent, not the overlay
        "{slug}",  # fleet-label / design-doc path template
        "{dir_path}",  # interview question.md directory template
        "{topic}",  # web-researcher search-query template
        "{current_year}",  # web-researcher search-query template
        "{current_month}",  # web-researcher search-query template
        "{output_path}",  # visualization.md code-block f-string
        "{script_stem}",  # visualization.md code-block f-string
        "{n}",  # math-formulas.md LaTeX summation bound
        "{upstream}",  # git `rev-parse @{upstream}` example
    }
)

# Everything subtracted from the raw base-file brace matches.
_IGNORE_TOKENS = META_TOKENS | NON_RESOLVABLE_TOKENS

_TOKEN_RE = re.compile(r"\{[a-z_]+\}")

# Backticked skill-root-relative ``*.md`` path cited in a note's applies-at cell.
_MD_PATH_RE = re.compile(r"`([^`]+\.md)`")

_BACKENDS = ("claude", "codex", "opencode")

# Skill families whose markdown forms the backend-neutral "base".
_BASE_FAMILIES = ("cafleet", "cafleet-design-doc", "cafleet-research")

_NOTE_HEADING = "## Note → applies at"
_DEFAULTS_HEADING = "### Documented defaults"


def extract_tokens(text: str) -> set[str]:
    """Return every ``{lowercase_underscore}`` brace span in ``text`` (verbatim,
    braces included). Pure: meta-tokens are NOT subtracted here — the universe
    builder does that."""
    return set(_TOKEN_RE.findall(text))


def _skill_root() -> Path:
    """Locate the repo's ``skills/`` directory relative to this module."""
    for parent in Path(__file__).resolve().parents:
        candidate = parent / "skills"
        if (candidate / "cafleet" / "SKILL.md").is_file():
            return candidate
    raise FileNotFoundError(
        "could not locate the skills/ root from overlay_coverage.py"
    )


def _coding_agent_dir() -> Path:
    return _skill_root() / "cafleet" / "reference" / "coding-agent"


def _base_files() -> list[Path]:
    """Every base markdown file: all ``*.md`` under the three skill families,
    excluding the backend overlays under ``cafleet/reference/coding-agent/``."""
    root = _skill_root()
    overlay_dir = _coding_agent_dir()
    files: list[Path] = []
    for family in _BASE_FAMILIES:
        for path in sorted((root / family).rglob("*.md")):
            if overlay_dir in path.parents:
                continue
            files.append(path)
    return files


def base_token_universe() -> set[str]:
    """All brace tokens across the base files, minus the ignore-set (meta +
    documented non-resolvable spans). Coherent base files yield exactly the
    canonical set; any leftover is a new resolvable token lacking coverage."""
    universe: set[str] = set()
    for path in _base_files():
        universe |= extract_tokens(path.read_text(encoding="utf-8"))
    return universe - _IGNORE_TOKENS


def _table_rows(lines: list[str]) -> list[str]:
    return [ln for ln in lines if ln.lstrip().startswith("|")]


def _cells(row: str) -> list[str]:
    # Neutralize escaped pipes (``\|``) so they don't split a cell in two.
    return [c.strip() for c in row.replace("\\|", " ").split("|")]


def _section(lines: list[str], heading: str) -> list[str]:
    """Lines from ``heading`` (exclusive) up to the next markdown heading."""
    out: list[str] = []
    in_section = False
    for ln in lines:
        if ln.strip() == heading:
            in_section = True
            continue
        if in_section and ln.startswith("#"):
            break
        if in_section:
            out.append(ln)
    return out


def _overlay_path(backend: str) -> Path:
    return _coding_agent_dir() / f"{backend}.md"


def _value_table_tokens(path: Path) -> set[str]:
    """First-column ``{token}`` cells of a value table — the rows before the
    ``## Note → applies at`` heading."""
    lines = path.read_text(encoding="utf-8").splitlines()
    value_lines: list[str] = []
    for ln in lines:
        if ln.strip() == _NOTE_HEADING:
            break
        value_lines.append(ln)
    tokens: set[str] = set()
    for row in _table_rows(value_lines):
        cells = _cells(row)
        if len(cells) > 1:
            tokens |= extract_tokens(cells[1])
    return tokens


def _overlay_value_tokens(backend: str) -> set[str]:
    """Tokens defined in an overlay's value table."""
    return _value_table_tokens(_overlay_path(backend))


def _template_value_tokens() -> set[str]:
    """Tokens defined in ``_template.md``'s value table (the new-backend skeleton)."""
    return _value_table_tokens(_coding_agent_dir() / "_template.md")


def _overlay_note_anchors(backend: str) -> set[str]:
    """Tokens an overlay's notes bind to — the ``{token}`` in each note row's
    *applies-at* (second) column."""
    lines = _overlay_path(backend).read_text(encoding="utf-8").splitlines()
    anchors: set[str] = set()
    for row in _table_rows(_section(lines, _NOTE_HEADING)):
        cells = _cells(row)
        if len(cells) > 2:
            anchors |= extract_tokens(cells[2])
    return anchors


def _overlay_note_paths(backend: str) -> set[str]:
    """Skill-root-relative ``*.md`` paths cited in an overlay's note applies-at
    cells (the backticked file references each note binds to)."""
    lines = _overlay_path(backend).read_text(encoding="utf-8").splitlines()
    paths: set[str] = set()
    for row in _table_rows(_section(lines, _NOTE_HEADING)):
        cells = _cells(row)
        if len(cells) > 2:
            paths |= set(_MD_PATH_RE.findall(cells[2]))
    return paths


def _default_table_tokens() -> set[str]:
    """Tokens with a row in the ``### Documented defaults`` table in
    ``cafleet/SKILL.md`` (first-column cell)."""
    lines = (
        (_skill_root() / "cafleet" / "SKILL.md")
        .read_text(encoding="utf-8")
        .splitlines()
    )
    tokens: set[str] = set()
    for row in _table_rows(_section(lines, _DEFAULTS_HEADING)):
        cells = _cells(row)
        if len(cells) > 1:
            tokens |= extract_tokens(cells[1])
    return tokens


def token_coverage_violations(
    base_universe: set[str],
    overlays: dict[str, set[str]],
    default_tokens: set[str],
) -> list[str]:
    """Every base token must be canonical, defined in all three overlay value
    tables, and present in the documented-default table."""
    violations: list[str] = []
    for token in sorted(base_universe):
        if token not in CANONICAL_TOKENS:
            violations.append(
                f"token coverage: base token {token} is not in the canonical set "
                f"(add overlay + default coverage, or add it to the ignore-set)"
            )
            continue
        violations.extend(
            f"token coverage: {token} missing from the {backend} overlay value table"
            for backend in _BACKENDS
            if token not in overlays[backend]
        )
        if token not in default_tokens:
            violations.append(
                f"token coverage: {token} missing from the documented-default table"
            )
    return violations


def orphan_token_violations(
    base_universe: set[str],
    overlays: dict[str, set[str]],
    default_tokens: set[str],
) -> list[str]:
    """Every token defined in an overlay value table or the default table must
    appear in at least one base file."""
    violations: list[str] = []
    for backend in _BACKENDS:
        violations.extend(
            f"orphan token: {token} defined in the {backend} overlay "
            f"but absent from every base file"
            for token in sorted(overlays[backend])
            if token not in base_universe
        )
    violations.extend(
        f"orphan token: {token} in the default table but absent from every base file"
        for token in sorted(default_tokens)
        if token not in base_universe
    )
    return violations


def note_anchor_violations(anchors: dict[str, set[str]]) -> list[str]:
    """Every note anchor must be a canonical token, and the note-anchor token
    set must be identical across the three overlays."""
    violations: list[str] = []
    for backend in _BACKENDS:
        violations.extend(
            f"note anchor: {token} in the {backend} overlay is not a canonical token"
            for token in sorted(anchors[backend])
            if token not in CANONICAL_TOKENS
        )
    union: set[str] = set()
    for backend in _BACKENDS:
        union |= anchors[backend]
    for backend in _BACKENDS:
        violations.extend(
            f"note anchor: {token} carries a note in another overlay but not in {backend}"
            for token in sorted(union - anchors[backend])
        )
    return violations


def note_path_violations(
    cited_paths: dict[str, set[str]],
    skill_root: Path,
) -> list[str]:
    """Every ``*.md`` path cited in a note's applies-at cell must resolve to an
    existing file under ``skill_root`` (design Part 5 check 3: "where it cites a
    file/section, that file exists")."""
    violations: list[str] = []
    for backend, paths in cited_paths.items():
        violations.extend(
            f"note reference: {backend} overlay cites `{rel_path}` in a note "
            f"applies-at cell, but no such file exists under the skills root"
            for rel_path in sorted(paths)
            if not (skill_root / rel_path).is_file()
        )
    return violations


def template_token_violations(template_tokens: set[str]) -> list[str]:
    """``_template.md``'s value table must define exactly the canonical token set
    — no missing token (a new backend would lack it) and no extra (drift)."""
    violations: list[str] = []
    violations.extend(
        f"template token: {token} is in the canonical set "
        f"but missing from the _template.md value table"
        for token in sorted(CANONICAL_TOKENS - template_tokens)
    )
    violations.extend(
        f"template token: {token} is in the _template.md value table "
        f"but not in the canonical set"
        for token in sorted(template_tokens - CANONICAL_TOKENS)
    )
    return violations


def check_overlay_coverage() -> list[str]:
    """Run all checks against the live skill tree; return all violations."""
    root = _skill_root()
    base_universe = base_token_universe()
    overlays = {backend: _overlay_value_tokens(backend) for backend in _BACKENDS}
    default_tokens = _default_table_tokens()
    anchors = {backend: _overlay_note_anchors(backend) for backend in _BACKENDS}
    cited_paths = {backend: _overlay_note_paths(backend) for backend in _BACKENDS}
    template_tokens = _template_value_tokens()
    return (
        token_coverage_violations(base_universe, overlays, default_tokens)
        + orphan_token_violations(base_universe, overlays, default_tokens)
        + note_anchor_violations(anchors)
        + note_path_violations(cited_paths, root)
        + template_token_violations(template_tokens)
    )


def main() -> int:
    """Entry point for ``mise //cafleet:lint-overlay``."""
    violations = check_overlay_coverage()
    if violations:
        print("overlay coverage: FAIL")
        for violation in violations:
            print(f"  - {violation}")
        return 1
    print(
        "overlay coverage: OK — 9 canonical tokens covered across overlays + defaults"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
