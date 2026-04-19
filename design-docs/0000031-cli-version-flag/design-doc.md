# CLI `--version` Flag

**Status**: Complete
**Progress**: 11/11 tasks complete
**Last Updated**: 2026-04-19

## Overview

Add a top-level `cafleet --version` global flag that prints `cafleet <version>` and exits 0, short-circuiting the `--session-id` required-flag check. The version string is sourced from the installed package metadata via `importlib.metadata`, so it stays in lock-step with `pyproject.toml` without any manual bookkeeping.

## Success Criteria

- [x] `cafleet --version` exits 0 and prints exactly `cafleet <version>\n` (e.g. `cafleet 0.1.0`).
- [x] `cafleet --version` works **without** `--session-id`; no "session-id required" error is raised.
- [x] Version string equals `importlib.metadata.version("cafleet")` (i.e., matches `project.version` in `cafleet/pyproject.toml`).
- [x] `mise //cafleet:test` passes with two new test cases covering the above.
- [x] `mise //cafleet:lint`, `mise //cafleet:format` (check mode), and `mise //cafleet:typecheck` all pass.
- [x] README.md, `docs/spec/cli-options.md`, and `skills/cafleet/SKILL.md` list `--version` under their Global Options tables.

---

## Specification

### Flag surface

Exactly one invocation is supported:

| Invocation | Behavior |
|---|---|
| `cafleet --version` | Prints `cafleet <version>` to stdout, exits 0 |

Explicitly **out of scope** (per user answers Q1):

- No short alias `-V`.
- No `cafleet version` subcommand form.
- No `--version` on subgroups (`cafleet session --version`, `cafleet member --version`, etc.).

### Session-id bypass

`cafleet --version` must succeed without `--session-id` (per Q2). This is natural behavior of `click.version_option`: the option is registered as `is_eager=True`, so Click invokes its callback during option parsing and calls `ctx.exit()` **before** the parent `cli` callback runs. The `_require_session_id(ctx)` guard lives inside each subcommand callback, so it is never reached. No custom bypass logic is needed.

### Output format

Text (per Q3):

```
cafleet 0.1.0
```

Implemented via the `message` parameter on `click.version_option`:

```python
click.version_option(
    package_name="cafleet",
    message="cafleet %(version)s",
)
```

Click substitutes `%(version)s` with the result of `importlib.metadata.version("cafleet")`. No trailing content, no `, version ` separator.

### JSON mode

`--version` does **not** honor the global `--json` flag (per Q4). Running `cafleet --json --version` still prints the plain text form:

```
cafleet 0.1.0
```

Rationale: the version string is trivially parseable already, and the `--json` flag is captured by the `cli` group callback, which Click skips when `--version`'s eager callback has already exited. A dedicated JSON branch would require replacing `click.version_option` with a handwritten eager option — not worth the complexity.

### Version source

`click.version_option(package_name="cafleet")` reads the version from the installed distribution's metadata via `importlib.metadata.version("cafleet")`. This means:

- After `mise //cafleet:install` (editable uv-tool install), the metadata is present and the flag works.
- After `uv sync`, metadata is present inside the project's virtualenv and the flag works.
- No `__version__` constant is added to `cafleet/src/cafleet/__init__.py` — the single source of truth stays `project.version` in `cafleet/pyproject.toml` (currently `0.1.0`).

### Error handling (design note)

If `importlib.metadata.version("cafleet")` raises `PackageNotFoundError` (the `cafleet` distribution is not installed in the current Python environment), Click lets it propagate as an unhandled exception, producing a traceback and non-zero exit. This is the desired behavior per Q6 and the project's `code-quality.md` ban on meaningless fallbacks.

This is a structural guarantee of `click.version_option(package_name="cafleet")` combined with the "no fallback" policy — there is no `try/except` around the metadata lookup anywhere in the code. No test is added for this case: monkeypatching `importlib.metadata.version` to raise would exercise Click's internals rather than cafleet's behavior, and the Q7 test scope explicitly limits us to the two CliRunner cases. Listed here as a Specification note rather than a Success Criterion to keep the acceptance bar aligned with the actual test suite.

### Code change

Single-line addition to `cafleet/src/cafleet/cli.py` — a `click.version_option` decorator on the top-level `cli` group. Placement is between the `@click.group()` decorator and the existing `@click.option("--json", ...)` / `@click.option("--session-id", ...)` decorators, matching Click's recommended decorator stacking order (`group` first, then eager options, then regular options):

```python
@click.group()
@click.version_option(package_name="cafleet", message="cafleet %(version)s")
@click.option(
    "--json", "json_output", is_flag=True, default=False, help="Output in JSON format"
)
@click.option(
    "--session-id",
    "session_id",
    default=None,
    help="Session ID (UUID); required for client subcommands.",
)
@click.pass_context
def cli(ctx, json_output, session_id):
    """CAFleet — CLI for the A2A-inspired message broker."""
    ctx.ensure_object(dict)
    ctx.obj["session_id"] = session_id
    ctx.obj["json_output"] = json_output
```

No other code files change. No new imports are needed (`click` is already imported).

### Tests

New file: `cafleet/tests/test_cli_version.py`. Two test cases (per Q7):

```python
from click.testing import CliRunner

from cafleet.cli import cli


def test_version_flag_prints_cafleet_and_version() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert result.output.startswith("cafleet ")
    # sanity: at least one digit in the version, and a trailing newline
    assert any(ch.isdigit() for ch in result.output)
    assert result.output.endswith("\n")


def test_version_flag_does_not_require_session_id() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert "session-id" not in result.output.lower()
```

The second test mirrors the invocation of the first but asserts the session-id bypass invariant — specifically, that the `--version` short-circuit runs before `_require_session_id` can trigger. Both tests use `CliRunner` (already the established pattern — see `test_cli_session_flag.py`, `test_cli_member.py`).

### Documentation updates

Per `.claude/rules/design-doc-numbering.md` Implementation Order section, every doc affected by a CLI-surface change must be updated in this same design-doc cycle. Concrete surface and action for each:

| File | Path | Change |
|---|---|---|
| README.md | `/README.md` | Add a `--version` row to the Global flags table at `README.md:190`. |
| ARCHITECTURE.md | `/ARCHITECTURE.md` | **No change.** The "CLI Option Sources" table (line 227) enumerates configuration parameters (session id, DB URL, agent id, JSON). `--version` is not a configuration parameter — it is a one-shot metadata dump. Excluding it keeps the table's purpose intact. |
| CLI Options spec | `/docs/spec/cli-options.md` | Add a `--version` row to the Global Options table at `docs/spec/cli-options.md:22`. Also mention in the "Subcommands that do NOT require `--session-id`" subsection that `--version` short-circuits before that check is reached. |
| CAFleet skill | `/skills/cafleet/SKILL.md` | Under the `## Global Options` section (starts at `skills/cafleet/SKILL.md:51`), extend the "Only `--json` and `--session-id` are global" sentence to also mention `--version`, and add a short paragraph describing the flag (one-line output, exits 0, works without `--session-id`). Do NOT touch the `## Required Flags` table — `--version` is not required. |
| Plugin copy | *(none)* | No `plugins/*/skills/cafleet/SKILL.md` exists at the project root — the single source of truth is `skills/cafleet/SKILL.md`. Confirmed via `Glob plugins/**/skills/cafleet*/SKILL.md` returning zero hits. |

Concrete row body to use everywhere (adapt column headers to each table's shape):

```
| `--version` | no | Print `cafleet <version>` and exit 0. Bypasses the `--session-id` requirement. Sourced from the installed package metadata via `importlib.metadata`. |
```

### Out of scope

- No changes to `__init__.py`, `pyproject.toml`, or any other package metadata.
- No version reporting in the WebUI / `server.py` (FastAPI `title="CAFleet Admin", version="0.1.0"` on line 30 of `server.py` is unchanged — that hardcoded string is a pre-existing drift issue tracked separately, not a part of this design doc).
- No version command output on other subcommands (e.g., `cafleet register` does not prefix its output with the version).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Per `.claude/rules/design-doc-numbering.md`, documentation updates come FIRST, then code, then tests.

### Step 1: Documentation updates

- [x] Update `README.md` — add a `--version` row to the Global flags table starting at `README.md:190`, using the body specified above. <!-- completed: 2026-04-19T03:45 -->
- [x] Update `docs/spec/cli-options.md` — add a `--version` row to the Global Options table at `docs/spec/cli-options.md:22`, and add a sentence to the "Subcommands that do NOT require `--session-id`" subsection noting that `--version` short-circuits the session-id check. <!-- completed: 2026-04-19T03:45 -->
- [x] Update `skills/cafleet/SKILL.md` — under the `## Global Options` section (starts at `skills/cafleet/SKILL.md:51`), extend the "Only `--json` and `--session-id` are global" sentence to include `--version`, and add a short paragraph describing the flag (one-line output, exits 0, works without `--session-id`). Do not edit the `## Required Flags` table. <!-- completed: 2026-04-19T03:45 -->
- [x] Confirm `ARCHITECTURE.md` needs no change by re-reading the "CLI Option Sources" section and the flag-listing tables; document the no-op decision in the commit message. <!-- completed: 2026-04-19T03:45 -->

### Step 2: Code implementation

- [x] Add `@click.version_option(package_name="cafleet", message="cafleet %(version)s")` to the `cli` group in `cafleet/src/cafleet/cli.py`, placed between `@click.group()` and the first `@click.option` decorator (as shown in the Specification code block). <!-- completed: 2026-04-19T03:50 -->
- [x] Run `mise //cafleet:format` and `mise //cafleet:lint` to confirm the decorator addition does not trigger style issues. <!-- completed: 2026-04-19T03:50 -->

### Step 3: Tests

- [x] Create `cafleet/tests/test_cli_version.py` with the two test cases shown in the Specification. <!-- completed: 2026-04-19T03:50 -->
- [x] Run `mise //cafleet:test` and confirm both tests pass alongside the existing suite. <!-- completed: 2026-04-19T03:50 -->

### Step 4: Verification

- [x] Run `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test` — all must pass. <!-- completed: 2026-04-19T03:55 -->
- [x] After reinstalling via `mise //cafleet:install`, manually verify `cafleet --version` prints `cafleet 0.1.0` and exits 0 (this step is optional for Claude Code, required for the human reviewer). <!-- completed: 2026-04-19T03:55 -->
- [x] Update this design doc's `Status` to `Complete` and `Progress` to `N/N` in the final commit. <!-- completed: 2026-04-19T04:00 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-19 | Initial draft |
| 2026-04-19 | Implementation complete. PR #33 opened. |
