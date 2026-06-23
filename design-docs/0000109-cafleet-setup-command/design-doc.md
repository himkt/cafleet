# cafleet setup command

**Status**: Approved
**Progress**: 12/13 tasks complete
**Last Updated**: 2026-06-22

## Overview

Add a `cafleet setup` CLI command that onboards a new machine in one step: it downloads the `cafleet-skills-v<cli_version>.zip` asset from the GitHub Release whose tag matches the **installed `cafleet` CLI version**, extracts the three skill directories into every detected coding-agent home, and runs the same Alembic migrations as `cafleet db init`. This replaces the multi-step manual onboarding (`gh skill install` per agent, then `cafleet db init`) with a single command and zero new runtime dependencies.

## Success Criteria

- [ ] `cafleet setup` runs **both** halves: installs skills into every detected agent home **and** migrates the database to head.
- [ ] The skills version is the **installed `cafleet` CLI version** (`importlib.metadata.version("cafleet")`); the skills half downloads `cafleet-skills-v<cli_version>.zip` from the Release tagged `<cli_version>` of `himkt/cafleet` over the public REST API using only the Python standard library (no new dependency, `gh` not required at runtime).
- [ ] Auto-detect installs skills only into agent homes that exist: `claude` → `~/.claude/skills/`, `codex` → `~/.codex/skills/`, `opencode` → `~/.config/opencode/skills/`.
- [ ] `--agent claude|codex|opencode` (repeatable, duplicate values deduped silently) scopes the **skills** targets to the named agents; the database half still runs.
- [ ] Each of the three skill directories is **replaced** (removed then re-extracted) at every target, so re-running `cafleet setup` against the same installed CLI version yields the same tree (per-resolved-version idempotency).
- [ ] Archive members with path-traversal / zip-slip paths (`..` or absolute) are rejected before extraction.
- [ ] A `skills/` directory whose entries are not exactly the three `SKILL_DIRS` directories — an extra directory, a missing one, or any stray non-directory file — is treated as malformed.
- [ ] The database half reuses the exact `cafleet db init` migration code path (including the refuse-on-unknown-revision guard); no migration logic is duplicated.
- [ ] The two halves run independently — a failure in one does not abort the other — each prints its own status, and the command exits non-zero if **either** half fails.
- [ ] `docs/get-started/install.md` and `README.md` present `cafleet setup` as the recommended end-user onboarding path; `gh skill install` / `mise //:skill-install` remain documented only as the contributor/local-dev path.

---

## Background

Today a new user must run several commands to become operational:

1. `uv tool install cafleet` (or `pip install cafleet`) — install the CLI.
2. `gh skill install himkt/cafleet --agent <claude-code|codex|opencode>` — once per coding agent, to place the skills.
3. `cafleet db init` — create/migrate the SQLite registry (`cli/db.py`).

The `gh skill install` path requires the GitHub CLI and is run once per agent. Design doc `0000108-upload-skills-artifacts-on-release` now attaches a self-contained `cafleet-skills-v<tag>.zip` asset to every published Release; the archive unpacks to a single top-level `skills/` folder containing exactly the three skill directories (`cafleet`, `cafleet-design-doc`, `cafleet-research`). `cafleet setup` consumes that asset, removing the need for `gh` and collapsing onboarding into one command.

The three coding-agent skill homes were verified empirically on a configured machine:

| Coding agent | Home directory | Skills directory |
|---|---|---|
| `claude` (Claude Code) | `~/.claude` | `~/.claude/skills/<name>/` |
| `codex` (OpenAI Codex CLI) | `~/.codex` | `~/.codex/skills/<name>/` |
| `opencode` | `~/.config/opencode` | `~/.config/opencode/skills/<name>/` |

`opencode` follows the XDG layout (`~/.config/opencode`), **not** `~/.opencode` (which holds only `agents/cafleet.md`, a separate cafleet artifact). `<name>` is one of `cafleet`, `cafleet-design-doc`, `cafleet-research`.

**Skills version = installed CLI version.** `cafleet setup` derives the version to install from the running CLI (`importlib.metadata.version("cafleet")`), so the skills always match the installed binary — there is no CLI/skills version skew. It resolves the Release tagged with that exact version and downloads its skills asset. Release tags in this repo are bare semantic versions (e.g. `0.12.2`), so the asset filename embeds an explicit `v`: for CLI version `0.12.2` the asset is `cafleet-skills-v0.12.2.zip` (per `0000108`). `0000108`'s `upload-skills` job ships in the same release cycle as this feature, so every published `cafleet` version carries its matching skills asset.

---

## Specification

### Command surface

`cafleet setup` is a new top-level command registered in `cafleet/src/cafleet/cli/__init__.py` alongside the existing groups. It is a single `click.command` (not a group) and always runs both halves.

| Flag | Type | Default | Effect |
|---|---|---|---|
| `--agent` | choice `claude\|codex\|opencode`, repeatable (`multiple=True`) | empty → auto-detect | Scope the **skills** targets to exactly the named agents (repeated values deduped silently). The database half runs regardless. |

`--agent` is the only flag; for a database-only run, use the existing `cafleet db init`. The skills version tracks the installed CLI version, resolved at runtime. Argument validation is just Click's built-in choice check — an unknown `--agent` value fails with exit code 2.

### Source constants

Defined as module-level constants in the new `cli/setup.py` (not settings — all are fixed with no env/flag override: `himkt/cafleet` is the single hardcoded source, the 30 s timeout is fixed, and the three target paths are final):

```python
GITHUB_REPO = "himkt/cafleet"
SKILL_DIRS = ("cafleet", "cafleet-design-doc", "cafleet-research")
HTTP_TIMEOUT = 30  # seconds; applied to every urlopen (release lookup + asset download)

# agent -> skills target directory (parent of the per-skill dirs)
AGENT_SKILLS_DIRS = {
    "claude": Path("~/.claude/skills"),
    "codex": Path("~/.codex/skills"),
    "opencode": Path("~/.config/opencode/skills"),
}
```

Paths are expanded with `Path.expanduser()` at use time. The release tag is **not** a constant — it is resolved at runtime from the installed CLI version (`importlib.metadata.version("cafleet")`).

### Skills half

The skills half runs these steps in order; any failure raises `click.ClickException` with a specific message (see *Error handling*). The download, extraction, and validation (steps 2–4) complete **fully before** any target directory is removed in step 5, so a network or archive failure never leaves a target with its skills removed-but-not-replaced.

1. **Resolve target agents.**
   - If `--agent` is supplied, the target set is exactly those agents (repeated values deduped). For an explicitly named agent whose home does not yet exist, the home/skills tree is created (`mkdir(parents=True)`) during install.
   - Otherwise auto-detect: an agent is a target when its **home directory** exists — the home is the parent of its skills dir (`~/.claude`, `~/.codex`, `~/.config/opencode`), checked via `AGENT_SKILLS_DIRS[agent].expanduser().parent.exists()`. Install only where the home exists; when the home exists but its `skills/` subdir does not, the skills dir is created during install.
   - If the resolved target set is empty → `click.ClickException` "no coding-agent homes detected (looked for ~/.claude, ~/.codex, ~/.config/opencode); install a coding agent first, or pass --agent".

2. **Resolve the release.** Read the installed CLI version with `importlib.metadata.version("cafleet")` (call it `cli_version`), then query the public GitHub REST API (`api.github.com`) over `urllib.request`:
   - `GET /repos/himkt/cafleet/releases/tags/<cli_version>` — the Release whose tag matches the installed CLI version.
   - Request headers: `Accept: application/vnd.github+json`, `User-Agent: cafleet` (GitHub rejects requests without a User-Agent). Strictly unauthenticated (no token); the 60-requests/hour anonymous limit is ample for downloading a public asset.
   - Every `urllib.request.urlopen` call (this lookup and the asset download in step 4) passes `timeout=HTTP_TIMEOUT` (30 s) so the command cannot hang indefinitely; a timeout surfaces via the network-error path.
   - The response JSON yields the `assets` array.

3. **Locate the asset.** The expected asset name is `cafleet-skills-v{cli_version}.zip` (tags are bare semver). Find the matching entry in `assets` and read its `browser_download_url` (a public URL needing no auth). Missing → `click.ClickException` "asset cafleet-skills-v<cli_version>.zip not found in release <cli_version>".

4. **Download, validate, and extract.** Stream the asset to a temporary file via a `urllib.request.Request` that carries the same `User-Agent: cafleet` header as the API lookup (GitHub's asset redirect target can reject UA-less requests; urllib follows the 302 to the presigned URL automatically), then open it with `zipfile.ZipFile` (both under a single `tempfile.TemporaryDirectory()` cleaned up on exit). Before extracting:
   - **Reject zip-slip / path-traversal members.** If any archive member name is absolute or contains a `..` component, raise `click.ClickException` (the archive is rejected, nothing is extracted) — even though it is the project's own asset.
   - Extract, then validate the layout: the archive must unpack to a single top-level `skills/` directory whose entries are **exactly** the three `SKILL_DIRS`, each a directory, and **nothing else**. A missing `skills/`, a missing skill dir, an extra directory, **or any stray non-directory entry directly under `skills/`** (e.g. a `skills/README.md`) → malformed → `click.ClickException`. This strict "nothing else" check is a **deliberate integrity check** (per the interview directive, Q15): the asset is the project's own and `0000108` guarantees it carries only the three skill directories, so any deviation signals a corrupt or wrong asset and is rejected rather than silently tolerated.
   - No symlink-specific handling — rely on `shutil`/`zipfile` defaults. No Content-Length or checksum verification; a `BadZipFile` or truncated download is reported as the "release asset is malformed" error.
   - The download is silent (no progress line).

5. **Install per target (replace semantics).** For each resolved agent and each `name` in `SKILL_DIRS`:
   - `dest = AGENT_SKILLS_DIRS[agent].expanduser() / name`
   - Ensure `dest.parent` exists (`mkdir(parents=True, exist_ok=True)`).
   - If `dest` exists, remove it (`shutil.rmtree`).
   - Copy the extracted `skills/<name>` to `dest` (`shutil.copytree`).
   This makes each run idempotent: the three skill dirs are fully replaced from the release, never merged.

   **Partial-state on mid-loop failure (intended).** The per-agent / per-skill loop is *not* aggregated the way the two top-level halves are. If a target fails partway (e.g. `PermissionError` on the second of three agents), the agents already installed keep their fresh skills and the remaining targets are untouched; the skills half then raises, and the error names the failed target. This partial outcome is acceptable and fully recoverable: because each run replaces every skill dir from the release, re-running `cafleet setup` after fixing the condition converges the whole set.

6. **Report.** Echo one line per agent summarizing the installed skills and the resolved tag, e.g. `claude: installed cafleet, cafleet-design-doc, cafleet-research (v0.12.2) -> ~/.claude/skills`.

### Database half

Reuse the exact `cafleet db init` migration code path. `cli/db.py` is refactored so its body becomes a callable helper; both `db init` and `cafleet setup` invoke it.

- Extract the current `init()` body (lines that build the sync URL, create the parent dir, materialize `alembic.ini`, run the upgrade with the unknown-revision and orphan-tables guards) into a module-level function `run_db_init() -> None` in `cli/db.py`.
- `db.init()` becomes a thin wrapper that calls `run_db_init()`.
- `cafleet setup`'s database half calls `run_db_init()`. No migration logic, guard, or message is duplicated.

The half preserves all existing behavior: idempotent re-runs ("Already at head"), refusal on an unknown Alembic revision, and the orphan-tables-without-`alembic_version` error.

### Independence and exit status

`cafleet setup` always runs both halves, and they are independent: each is wrapped so a failure in one does not prevent the other from running. The two halves are order-independent.

```text
failures = []
try skills half   except ClickException as e: failures.append(("skills", e)); echo error
try run_db_init() except ClickException as e: failures.append(("db", e)); echo error
if failures:      raise click.ClickException summarizing which halves failed   # exit code 1
```

Each half emits its own output, and the orchestration adds **no** redundant per-half success banner on top:

- **Skills half, success:** the per-agent report lines from step 6 (e.g. `claude: installed … -> ~/.claude/skills`).
- **Database half, success:** `run_db_init()`'s own echo verbatim (`Already at head …`, `Created …`, or `Upgraded …`) — the orchestration does **not** print an additional db-success line, so the DB result is reported exactly once.
- **Either half, failure:** the half's `click.ClickException` message is echoed at the point of failure; the orchestration then raises a single final `click.ClickException` naming which half/halves failed (exit code 1).

So on a partial outcome the user sees the successful half's normal output **and** the failed half's error, with a closing summary — the explicit "a failure in one does not abort the other" behavior. Within each half, individual operations still fail loudly; the aggregation is only across the two top-level halves. The per-agent loop inside the skills half is not aggregated (see *Install per target → Partial-state on mid-loop failure*).

### Error handling

| Condition | Surfaced as |
|---|---|
| Network / API unreachable, timeout, DNS failure, **403 rate-limit, and any non-404 HTTP error (5xx)** | `click.ClickException` "could not reach the GitHub API (<reason>)" — from `urllib.error.URLError` / `socket.timeout` / `urllib.error.HTTPError` (non-404). Folded into one path; no token support, so no `GITHUB_TOKEN` mention |
| No release found (`/tags/<cli_version>` returns 404) | `click.ClickException` "no release found for version <cli_version>" — from `urllib.error.HTTPError` 404 |
| Asset `cafleet-skills-v<cli_version>.zip` absent from the release | `click.ClickException` "asset cafleet-skills-v<cli_version>.zip not found in release <cli_version>" |
| Archive member rejected for path-traversal / zip-slip (`..` or absolute) | `click.ClickException` naming the offending member; nothing is extracted |
| Archive malformed: not a zip / `BadZipFile` / truncated download / missing `skills/` / missing a skill dir / **any extra entry under `skills/` beyond the three skill directories (extra dir or stray file)** | `click.ClickException` "release asset is malformed" |
| Target skills dir not writable (`PermissionError`/`OSError` during mkdir/rmtree/copytree) | `click.ClickException` naming the path and the OS error |
| Zero agent homes detected (auto-detect) | `click.ClickException` listing the three searched paths and suggesting `--agent` |
| Unknown / orphan DB revision | inherited verbatim from the reused `run_db_init()` guards |
| `importlib.metadata.version("cafleet")` raises `PackageNotFoundError` | **Intentionally unhandled** — a can't-happen invariant for the installed CLI (the same metadata already backs `@click.version_option(package_name="cafleet")`), so it fails loud per `affirmative-writing.md` rather than being masked |

All network and filesystem calls use `urllib`/`zipfile`/`shutil`/`pathlib` from the standard library; no new dependency is added to `cafleet/pyproject.toml`.

### Documentation surface

Per the project rule, documentation is updated **first** (see Implementation Step 1):

- **`docs/get-started/install.md`** — restructure the end-user flow to: `uv tool install cafleet` → `cafleet setup`. `cafleet setup` both installs the skills (matching the installed CLI version) and runs the migrations, so the separate `gh skill install` block and the standalone `cafleet db init` step are removed from the end-user path. Clarify that `cafleet setup` is an **end-user (installed-CLI) command**; contributors working from the repo use `mise //:skill-install` instead. The `gh skill install` / `mise //:skill-install` commands remain documented as the **contributor / local-dev** path (installing skills from the working tree rather than a Release). The integer-PK upgrade warning stays (it is migration behavior `cafleet setup` now triggers).
- **`README.md`** — reflect `cafleet setup` as the onboarding entry point, consistent with the install page.
- **`docs/spec/cli-options.md`** — add a `cafleet setup` row to the subcommand summary table and a `setup` detail section documenting its **only** flag (`--agent`) and stating that setup installs the skills matching the installed CLI version. Do not add an explicit exit-codes table — runtime error messages are the surface.
- **Affected `SKILL.md`** — update any skill body that documents onboarding/install or the CLI surface (notably `skills/cafleet/SKILL.md` and its CLI reference pages) so no example claims `gh skill install` is the user path; mirror the install-page change.

This change adds a CLI command but no new config/env surface (`config.py` is unchanged) and no API/architectural surface. `cafleet setup` installs only from a published Release; the contributor local-source install (`mise //:skill-install`) stays entirely separate.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Rewrite `docs/get-started/install.md` end-user flow around `uv tool install cafleet` → `cafleet setup`; move `gh skill install` / `mise //:skill-install` to a contributor/local-dev note; keep the integer-PK upgrade warning. <!-- completed: 2026-06-23T08:22 -->
- [x] Update `README.md` so onboarding points to `cafleet setup`, consistent with the install page. <!-- completed: 2026-06-23T08:22 -->
- [x] Add the `cafleet setup` row + detail section (the single `--agent` flag; note skills track the installed CLI version; no exit-codes table) to `docs/spec/cli-options.md`. <!-- completed: 2026-06-23T08:22 -->
- [x] Update affected `SKILL.md` / skill CLI-reference pages (notably `skills/cafleet/SKILL.md`) so the documented onboarding matches `cafleet setup`. <!-- completed: 2026-06-23T08:22 -->

### Step 2: Refactor the database init code path for reuse

- [x] Extract the body of `db.init()` in `cafleet/src/cafleet/cli/db.py` into a module-level `run_db_init() -> None`, preserving the unknown-revision and orphan-tables guards and all echo messages; make `db.init()` a thin wrapper calling it. <!-- completed: 2026-06-23T08:27 -->

### Step 3: Implement the `cafleet setup` command

- [x] Create `cafleet/src/cafleet/cli/setup.py` with the `setup` command: constants (`GITHUB_REPO`, `SKILL_DIRS`, `HTTP_TIMEOUT`, `AGENT_SKILLS_DIRS`) and the sole `--agent` flag (repeatable choice, deduped; Click's choice check is the only validation). <!-- completed: 2026-06-23T08:40 -->
- [x] Implement the skills half: resolve target agents (auto-detect or `--agent`), read the installed version via `importlib.metadata.version("cafleet")`, resolve the Release via `GET /repos/himkt/cafleet/releases/tags/<cli_version>`, locate `cafleet-skills-v<cli_version>.zip`, download to a temp file (timeout `HTTP_TIMEOUT`), reject zip-slip members, extract + validate the `skills/` layout is exactly `SKILL_DIRS` (download/extract/validate before any rmtree), then install each skill dir with replace semantics. <!-- completed: 2026-06-23T08:40 -->
- [x] Implement the database half by calling `run_db_init()`, and the independent-halves orchestration (always run both, collect per-half failures, print per-half status, exit non-zero if either failed). <!-- completed: 2026-06-23T08:40 -->
- [x] Register `setup` in `cafleet/src/cafleet/cli/__init__.py` via `cli.add_command(setup)`. <!-- completed: 2026-06-23T08:40 -->

### Step 4: Tests

- [x] Add `cafleet/tests/cli/test_setup.py` using `click.testing.CliRunner`, monkeypatching `importlib.metadata.version("cafleet")`, the `GET /releases/tags/<cli_version>` response, and the asset download (a synthetic in-memory zip; redirect-following is out of scope — monkeypatch the URL directly), with `AGENT_SKILLS_DIRS` pointed at `tmp_path` homes. Cover: auto-detect installs only into present homes; `--agent` scoping including silent dedupe of repeated values; `--agent` valid while the DB half still runs; replace/idempotency (a second run against the same version leaves a clean tree); zip-slip member rejected (nothing extracted); an extra entry under `skills/` (extra dir or stray file) → malformed; **a missing skill dir (`skills/` holds only two of the three `SKILL_DIRS`) → malformed**; missing-asset generic message; **404 no-release-for-version**; the network-error path including folded 403/5xx; `BadZipFile` → malformed; **unwritable target** (`PermissionError` during install); and **zero homes detected** (auto-detect resolves an empty set). Every *Error handling* row has a corresponding case. <!-- completed: 2026-06-23T08:31 -->
- [x] Add the two **independence** cases under full `cafleet setup`: (a) force the skills half to fail (malformed asset) and assert the DB half still ran (schema created at head), exit code == 1, and both per-half status lines printed; (b) force the DB half to fail (orphan-tables DB) while skills succeed, and assert skills were still installed, exit code == 1, and both status lines printed. <!-- completed: 2026-06-23T08:31 -->
- [x] Add a database-half test that invokes `run_db_init()` **directly** as a unit test (no skills mocking) against a temp SQLite — creates the schema at head; a re-run reports "Already at head"; confirm `cafleet db init` is otherwise unchanged. (The DB half under full `cafleet setup` is already exercised by the main test above, where the skills half is mocked.) <!-- completed: 2026-06-23T08:31 -->

### Step 5: Validate

- [ ] Run `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test`; fix any findings. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-22 | Initial draft |
| 2026-06-22 | Interview pivot: skills version derived from the installed CLI version (no latest-stable); flag surface reduced to `--agent` only (dropped `--skills-only`/`--db-only`/`--release-tag`); added zip-slip rejection and strict `skills/` layout validation; removed the asset-availability-precondition guard. |
