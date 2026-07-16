# Ship Coding-Agent Presets via `cafleet setup`

**Status**: Approved
**Progress**: 25/25 tasks complete
**Last Updated**: 2026-07-17

## Overview

The OpenCode agent preset (`~/.opencode/agents/cafleet.md`) and the Codex rules file (`~/.codex/rules/cafleet.rules`) become static assets shipped inside the skills release archive and installed by `cafleet setup`, exactly like the skills: downloaded, overwritten on every install, and version-recorded. The code-generated lazy materialization on first `member create --coding-agent opencode` is removed, and the `setup skill` subcommand is replaced by per-agent subcommands (`cafleet setup claude|codex|opencode`).

## Success Criteria

- [x] `presets/opencode/cafleet.md` (byte-identical to the current rendered preset) and `presets/codex/cafleet.rules` (the rules block currently documented as a manual step in quickstart) are checked into the repo and packaged into `cafleet-skills-v<version>.zip` alongside `skills/`
- [x] Bare `cafleet setup` installs each preset to its target whenever its agent is a resolved target, overwriting any existing copy
- [x] `cafleet setup claude` / `cafleet setup codex` / `cafleet setup opencode` replace `cafleet setup skill --agent <name>`; no `setup skill` surface remains
- [x] `member create --coding-agent opencode` writes no file; a missing preset fails the spawn with guidance to run `cafleet setup opencode`
- [x] `opencode_preset.py`, its tests, every "materialized on first spawn" / skip-if-exists mention, and quickstart's manual rules-file step are removed repo-wide (per `removal.md`)
- [x] SPEC.md, docs/, README.md, and skills references reflect the new contract; `mise //cafleet:lint`, `//cafleet:typecheck`, and `//cafleet:test` pass

---

## Background

Today the opencode preset is generated in Python (`cafleet/src/cafleet/coding_agent/opencode_preset.py`: `PermissionRuleset` + `OpencodeAgentDefinition` + `CAFLEET_AGENT` + `materialize_cafleet_agent`) and written lazily by `OpencodeAgent.ensure_available()` on the first opencode spawn, with skip-if-exists semantics and a refuse-to-overwrite branch for non-regular-file targets. The Codex rules file `~/.codex/rules/cafleet.rules` is documented only as a manual operator step in `docs/quickstart.md`; no cafleet code writes it. `cafleet setup` (`cafleet/src/cafleet/cli/setup.py`) runs two independent halves — the db half and the skills half — and touches neither file. The skills half downloads `cafleet-skills-v<version>.zip` (built by `.github/workflows/publish.yml` as `zip -r … skills`), validates the layout, delete-and-reinstalls each skill dir per agent home, and upserts one `skill_installs` row per home.

The user decided: treat both presets the same way as the skills, and ship them in the same archive.

---

## Specification

### Decisions

| # | Decision | Choice |
|---|----------|--------|
| 1 | Preset sources of truth | Static repo files: `presets/opencode/cafleet.md`, checked in byte-identical to the current rendered output (the SPEC.md verbatim block remains the content contract), and `presets/codex/cafleet.rules`, checked in with the two `prefix_rule` blocks quickstart documents today (verbatim below). The Python generator is deleted. |
| 2 | Distribution | The existing release asset `cafleet-skills-v<version>.zip` gains a top-level `presets/` directory next to `skills/`. Asset name unchanged. |
| 3 | Setup surface | The preset installs fold into the (renamed) **assets half**. `setup skill` is replaced by per-agent subcommands `cafleet setup claude` / `codex` / `opencode`. |
| 4 | Detection gating | Unchanged: auto-detect via the known agent-home parents (`~/.claude`, `~/.codex`, `~/.config/opencode`). Each preset installs whenever its agent is a resolved target. An explicit `cafleet setup <agent>` skips detection, like `--agent` today. |
| 5 | Overwrite semantics | Same as skills: remove whatever exists at the target (file, symlink, or directory), then install the bundled copy. The skip-if-exists and refuse-to-overwrite branches are removed. The refresh recipe becomes "re-run `cafleet setup <agent>`". |
| 6 | Spawn preconditions | `OpencodeAgent.ensure_available()` no longer writes; it PATH-checks `opencode`, then verifies the preset file exists (a spawn precondition — the spawn argv references `--agent cafleet`) and raises with guidance if missing. `CodexAgent.ensure_available()` stays PATH-check-only: the rules file is a permission posture, not an argv dependency, and operators may keep equivalent rules in other files under `~/.codex/rules/`. |
| 7 | Version recording | Unchanged schema. An agent's `skill_installs` row is upserted only after both its skill dirs and its preset (where one exists) install successfully, so the row attests skills + preset. The stale-skills guard needs no logic change. |

**Subcommand shape rationale** (decision 3): the user floated `cafleet setup claude|codex|opencode`. Per-agent subcommands are adopted because the half now installs heterogeneous per-agent assets (skills for all three, plus a preset for codex and opencode), which "skill" no longer names accurately, and they remove the repeatable `--agent` flag. The alternative — keeping `setup skill --agent` — was rejected as a misnomer after the fold. Installing two of three agents takes two invocations; bare `cafleet setup` still covers "all detected".

### Preset asset catalog

| Agent | Archive source | Install target | Spawn precondition check |
|-------|----------------|----------------|--------------------------|
| claude | — (no preset) | — | — |
| codex | `presets/codex/cafleet.rules` | `~/.codex/rules/cafleet.rules` | none (PATH check only) |
| opencode | `presets/opencode/cafleet.md` | `~/.opencode/agents/cafleet.md` | `ensure_available` existence check |

Exact contents of `presets/codex/cafleet.rules` (verbatim, as quickstart documents today):

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

**Ownership of `cafleet.rules`.** Because quickstart has told operators to hand-create this file, existing installs may carry customizations that the overwrite-every-run install would silently clobber. The design therefore declares `~/.codex/rules/cafleet.rules` **owned by `cafleet setup`**: it is overwritten on every install, and operator customizations belong in a separate rules file under `~/.codex/rules/` (Codex loads every rules file in that directory, which is the escape hatch decision 6's rationale relies on). The docs task in Step 1 verifies this multi-file loading behavior against the Codex CLI documentation and states both the ownership and the customization recipe in the new codex rules-file section.

### Repo layout and release archive

| Artifact | Path |
|----------|------|
| Checked-in preset sources | `presets/opencode/cafleet.md`, `presets/codex/cafleet.rules` (repo root) |
| Archive layout | `skills/{cafleet,cafleet-design-doc,cafleet-research}/…` + `presets/opencode/cafleet.md` + `presets/codex/cafleet.rules` |
| Publish workflow change | `.github/workflows/publish.yml`: `zip -r "cafleet-skills-v${tag}.zip" skills presets` |

The two opencode base directories keep their distinct purposes: the preset lives under `~/.opencode/` (opencode's `--agent cafleet` discovery path); skills install + home auto-detection use `~/.config/opencode/`.

Version skew is a non-issue: the CLI always downloads the archive tagged with its own version (`importlib.metadata.version("cafleet")`), so a CLI with the new extractor always receives an archive with `presets/`.

### `cafleet setup` CLI surface

`setup` remains a Click group with `invoke_without_command=True`.

| Command | Behavior |
|---------|----------|
| `cafleet setup` (bare) | db half, then **assets half** over auto-detected agent homes. Half-failure labels: `db half failed: <msg>` / `assets half failed: <msg>`; combined exit-1 error `db and assets half failed`. |
| `cafleet setup db` | Unchanged. |
| `cafleet setup claude` / `codex` / `opencode` | Runs the assets half for exactly that agent (no auto-detection). Same schema pre-flight and error string as `setup skill` today (`the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first`). Registered from the coding-agent names so all three share one implementation. |

`setup skill` and its `--agent` option are removed entirely (hard break, no alias).

### Assets half (per resolved target)

1. Resolve targets (auto-detect for bare `setup`; the literal agent for a per-agent subcommand). The no-homes-detected error message drops the `--agent` hint: `no coding-agent homes detected (looked for ~/.claude, ~/.codex, ~/.config/opencode); install a coding agent first, or run 'cafleet setup <agent>'`.
2. Resolve the download URL and download/extract once (unchanged mechanics, timeout, and unsafe-path rejection). Layout validation now also requires both catalog archive sources (`presets/opencode/cafleet.md`, `presets/codex/cafleet.rules`) to be regular files in the extracted root, else the existing `release asset is malformed` error.
3. Per target, delete-and-reinstall the three skill dirs (unchanged; error string `failed to install skills into <skills_dir>: <error>` unchanged).
4. **Targets with a catalog entry** (codex, opencode): install the preset — create the target's parent directory chain recursively, remove any existing target in this explicit check order: `is_symlink()` → unlink; else `is_dir()` → rmtree; else if it exists → unlink (the symlink check must come first — `is_dir()` follows symlinks and `shutil.rmtree` refuses them), then copy the archive source in. A filesystem error aborts with `failed to install preset into <target>: <error>` (exit 1, same abort-the-loop behavior as a skills failure; that agent's `skill_installs` row is not recorded).
5. Upsert the target's `skill_installs` row, then echo:

```
<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>   # codex/opencode targets only, second line
```

### `OpencodeAgent.ensure_available()`

```python
def ensure_available(self) -> None:
    ensure_binary_on_path(self.binary_name)
    preset = Path("~/.opencode/agents/cafleet.md").expanduser()
    if not preset.is_file():
        raise RuntimeError(
            f"opencode agent preset not found at {preset}; "
            "run 'cafleet setup opencode' first"
        )
```

`member create` keeps the `validate_model → ensure_available → build_spawn_argv` ordering. The `CodingAgent` protocol docstring in `base.py` is updated in both places it mentions materialization: the side-effect clause ("Impls MAY materialize required config files here as a side effect…", base.py:36-37) is removed — no backend materializes config anymore — and the preconditions sentence ("(for backends with bundled presets) the preset has been materialized to disk", base.py:32-34) is reworded to "(for backends with bundled presets) the preset file exists on disk".

### Contract string changes (SPEC.md surfaces)

| String | Change |
|--------|--------|
| `skills half failed: <msg>` | → `assets half failed: <msg>` |
| `db and skills half failed` | → `db and assets half failed` |
| `run 'cafleet setup skill' to reinstall` (stale-skills guard) | → `run 'cafleet setup' to reinstall` |
| `…install a coding agent first, or pass --agent` | → `…install a coding agent first, or run 'cafleet setup <agent>'` |
| `cannot materialize CAFleet opencode agent preset: … refusing to overwrite` | Removed (branch deleted). |
| `cannot materialize CAFleet opencode agent preset at <target>: <error>` | Removed; superseded by `failed to install preset into <target>: <error>`. |
| (new) `opencode agent preset not found at <preset>; run 'cafleet setup opencode' first` | Raised by `ensure_available` on a missing preset. |

### Removals (per `removal.md`, total in this change)

- `cafleet/src/cafleet/coding_agent/opencode_preset.py` — entire module (dataclasses, `CAFLEET_AGENT`, `materialize_cafleet_agent`).
- `cafleet/tests/coding_agent/test_opencode_preset.py` — entire file (generation + materialization tests; the static file is the artifact, setup tests cover installation).
- The materialization spy/idempotency tests in `cafleet/tests/coding_agent/test_opencode.py` — replaced by PATH-check + preset-existence tests.
- All "materialized on first spawn" / skip-if-exists / refuse-to-overwrite prose in `docs/spec/coding-agent-backends.md`, `docs/quickstart.md`, SPEC.md §6.7, and `skills/cafleet/reference/director.md:30` ("The `opencode` backend materializes its agent preset on first spawn."); SPEC's "opencode preset materialization" and "rendering rules" sections are rewritten to describe the shipped-file contract (the verbatim contents block stays, re-anchored to `presets/opencode/cafleet.md`).
- Quickstart's manual rules-file step (the `~/.codex/rules/cafleet.rules` tip + snippet) — replaced by "installed by `cafleet setup`" coverage; the precedence explanation moves to `docs/spec/coding-agent-backends.md`.
- Every `setup skill` mention repo-wide (docs, SPEC, skills, tests) — known doc hits beyond the setup pages: `docs/spec/data-model.md:140` and `docs/concepts/storage.md:49`.

### Test impact

| Test surface | Change |
|--------------|--------|
| `tests/cli/test_setup.py` | Fake-archive fixtures gain `presets/opencode/cafleet.md` + `presets/codex/cafleet.rules`; new assertions: both presets installed on bare setup + their per-agent subcommands, overwrite of an existing file / directory / symlink / **symlink-to-directory** target, preset failure aborts and skips the row, layout validation rejects an archive missing either preset; `setup skill` tests replaced by per-agent subcommand tests (absence of `setup skill` asserted via Click's default no-such-command error). |
| `tests/coding_agent/test_opencode.py` | `ensure_available` asserts PATH check, missing-preset `RuntimeError` with the new message, and success when the preset file exists. |
| `tests/coding_agent/test_protocol.py` | The HOME-redirect fixture now pre-creates `~/.opencode/agents/cafleet.md` (opencode's `ensure_available` reads rather than writes; codex needs no fixture — it stays PATH-check-only). |
| `tests/cli/test_member.py` | Opencode `member create` tests pre-create the preset file in the redirected HOME. |
| `tests/cli/test_skills_guard.py` | The `STALE_REPAIR` constant (test_skills_guard.py:25) and the full-message assertions (lines 113, 128-129) update to the new guard string `run 'cafleet setup' to reinstall`. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/spec/cli-options.md` § `cafleet setup`: assets half, per-agent subcommands, new/changed error strings, preset installs + output lines <!-- completed: 2026-07-17T02:18 -->
- [x] Rewrite `docs/spec/coding-agent-backends.md`: the cafleet agent preset section (installed/overwritten by `cafleet setup`, refresh recipe = re-run, missing-preset spawn error) and a codex rules-file section absorbing quickstart's precedence explanation, declaring `cafleet.rules` as owned/overwritten by `cafleet setup`, and directing customizations to a separate rules file under `~/.codex/rules/` — verifying Codex's multi-file rules-loading behavior against the Codex CLI documentation before stating it (per § Preset asset catalog, *Ownership of `cafleet.rules`*) <!-- completed: 2026-07-17T02:20 -->
- [x] Update `docs/quickstart.md`: setup line covers skills + presets; drop the first-spawn materialization note and the manual `cafleet.rules` step <!-- completed: 2026-07-17T02:25 -->
- [x] Update `README.md` setup comment if its thin surface drifts (via the `/update-readme` skill) <!-- completed: 2026-07-17T02:42 -->
- [x] Update SPEC.md: §setup (assets half + per-agent subcommands + shared helpers + archive validation), §stale-skills guard message, §6.7 coding-agent scope + preset sections (add the codex rules-file contract) + contract error strings, §leaf-module list entry for coding-agent, plus the five outlying surfaces: the fail-loud inventory ("The opencode preset refuses to overwrite a non-regular-file target", SPEC.md:2603), the `skill_installs` schema note ("rows written by the skills half of `setup` and by `setup skill`", SPEC.md:2678 — also gains decision 7's skills+preset attestation), the test-strategy coding-agent bullet ("the preset markdown structure; the materializer's skip/refuse/write branches", SPEC.md:2724-2726), the CLI-surface checklist rows for bare `setup` / `setup skill` (SPEC.md:2763-2765), and the upgrade-path prose ("instructs the operator to run `cafleet setup skill` to reinstall", SPEC.md:2841) <!-- completed: 2026-07-17T02:33 -->
- [x] Sweep skills/ and remaining docs for `setup skill` and first-spawn-materialization mentions, including the named hits in § Removals (`skills/cafleet/reference/director.md:30`, `docs/spec/data-model.md:140`, `docs/concepts/storage.md:49`); update each <!-- completed: 2026-07-17T02:36 -->

### Step 2: Preset assets + release archive

- [x] Add `presets/opencode/cafleet.md`, byte-identical to the current `CAFLEET_AGENT.to_markdown()` output (verify against the SPEC verbatim block) <!-- completed: 2026-07-17T02:52 -->
- [x] Add `presets/codex/cafleet.rules` with the verbatim rules block from § Preset asset catalog <!-- completed: 2026-07-17T02:53 -->
- [x] Update `.github/workflows/publish.yml` to zip `skills presets` <!-- completed: 2026-07-17T02:53 -->

### Step 3: CLI implementation

- [x] `cli/setup.py`: rename the skills half to the assets half; update half-failure labels and the no-homes-detected message <!-- completed: 2026-07-17T03:05 -->
- [x] `cli/setup.py`: replace `setup skill` with per-agent subcommands `claude` / `codex` / `opencode` sharing one implementation <!-- completed: 2026-07-17T03:05 -->
- [x] `cli/setup.py`: add the preset asset catalog; extend archive validation to require both archive sources; add the per-agent preset install (mkdir parents; remove existing target in the order `is_symlink()` → unlink, else `is_dir()` → rmtree, else unlink if present; copy) with the new error string; record the row only after skills + preset succeed; add the preset success line <!-- completed: 2026-07-17T03:05 -->
- [x] Update the stale-skills guard message to `run 'cafleet setup' to reinstall` <!-- completed: 2026-07-17T03:05 -->
- [x] `coding_agent/opencode.py`: `ensure_available` = PATH check + preset-existence check with the new error; drop the `materialize_cafleet_agent` import <!-- completed: 2026-07-17T03:05 -->
- [x] Delete `coding_agent/opencode_preset.py`; update the `CodingAgent` protocol docstring in `base.py` in both places (remove the side-effect clause; reword the precondition to "the preset file exists on disk") <!-- completed: 2026-07-17T03:12 -->

### Step 4: Tests

- [x] `tests/cli/test_setup.py`: extend fake-archive fixtures with both presets; add install/overwrite (file, directory, symlink, symlink-to-directory)/failure/validation assertions for both agents; replace `setup skill` tests with per-agent subcommand tests + a no-such-command regression guard <!-- completed: 2026-07-17T02:28 -->
- [x] Delete `tests/coding_agent/test_opencode_preset.py` <!-- completed: 2026-07-17T02:28 -->
- [x] `tests/coding_agent/test_opencode.py`: rewrite `ensure_available` tests (PATH check, missing-preset error, success with preset present) <!-- completed: 2026-07-17T02:28 -->
- [x] `tests/coding_agent/test_protocol.py`: fixture pre-creates the opencode preset file in the redirected HOME <!-- completed: 2026-07-17T02:28 -->
- [x] `tests/cli/test_member.py`: opencode spawn tests pre-create the preset file <!-- completed: 2026-07-17T02:28 -->
- [x] `tests/cli/test_skills_guard.py`: update `STALE_REPAIR` and the full-message assertions to the new guard string <!-- completed: 2026-07-17T02:28 -->

### Step 5: Verification

- [x] `mise //cafleet:lint` passes <!-- completed: 2026-07-17T02:51 -->
- [x] `mise //cafleet:typecheck` passes <!-- completed: 2026-07-17T02:51 -->
- [x] `mise //cafleet:test` passes <!-- completed: 2026-07-17T02:51 -->
- [x] Repo-wide sweep confirms no `setup skill`, `materialize_cafleet_agent`, or first-spawn-materialization mention remains outside design-docs/ and git history <!-- completed: 2026-07-17T02:51 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-17 | Initial draft |
| 2026-07-17 | Review round 1: Codex `cafleet.rules` added to scope (shipped + installed like the opencode preset); symlink-to-directory removal order fixed; second `base.py` docstring mention, `test_skills_guard.py`, and the outlying SPEC/doc `setup skill` surfaces enumerated |
| 2026-07-17 | Review round 2: `cafleet.rules` declared owned/overwritten by `cafleet setup`, with customizations directed to a separate rules file and the multi-file loading behavior to be verified in the docs task |
