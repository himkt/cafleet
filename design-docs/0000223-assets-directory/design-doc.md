# Group embedded data under `assets/`

**Status**: Complete
**Progress**: 29/29 tasks complete
**Last Updated**: 2026-11-08

## Overview

Move every data file the binary embeds — the root `migrations/`, `presets/`, `extensions/` and `skills/` trees plus the runner-profile catalog and native screen fixtures under `src/coding_agent/` — into one top-level `assets/` directory. The move makes the role of these files explicit, so they no longer sit scattered beside `src/` and `docs/` or inside a Rust module directory. It is a pure refactor: embedded table keys, installed bytes, install targets and every user-visible behavior stay identical.

## Success Criteria

- [x] `assets/` holds exactly the subdirectories `migrations/`, `presets/`, `extensions/`, `skills/` and `profiles/`, and the old roots `migrations/`, `presets/`, `extensions/`, `skills/` and `src/coding_agent/native/` plus `src/coding_agent/bundled.json` no longer exist in the working tree.
- [x] Every moved file is a 100%-similarity rename (`R100` in `git diff -M --name-status main`); no moved file's content changes.
- [x] `build.rs` emits one `cargo:rerun-if-changed=assets` line and builds the `SKILLS`, `PRESETS` and `EXTENSIONS` tables from `assets/skills`, `assets/presets` and `assets/extensions` with unchanged relative keys.
- [x] `src/db.rs` embeds `assets/migrations/V1__baseline.sql`, and `src/coding_agent/catalog.rs` embeds `assets/profiles/bundled.json` and `assets/profiles/native/<backend>.json` with the `BUNDLED_FIXTURES` keys still `native/<backend>.json`.
- [x] `SPEC.md` has one owning table for the embedded asset tree, and every repository-source path in `SPEC.md`, `docs/`, root `CLAUDE.md`, `.claude/rules/` and `.claude/skills/` names its `assets/` location; install-target and user-machine paths are unchanged.
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.
- [x] Touching a file under `assets/` reruns the build script, and the local `gh skill install ./ --from-local` listing still discovers both skills from `assets/skills/`.

---

## Background

The binary embeds data from two mechanisms today:

| Data | Current location | Embedding mechanism |
|---|---|---|
| Skills | `skills/` | `build.rs` `emit_table("SKILLS", …)`, `rerun-if-changed=skills` |
| Presets | `presets/` | `build.rs` `emit_table("PRESETS", …)`, `rerun-if-changed=presets` |
| Extensions | `extensions/` | `build.rs` `emit_table("EXTENSIONS", …)`, `rerun-if-changed=extensions` |
| Schema baseline | `migrations/V1__baseline.sql` | `include_str!("../migrations/V1__baseline.sql")` in `src/db.rs` (plus a `rerun-if-changed=migrations` line the build script never reads) |
| Runner-profile catalog | `src/coding_agent/bundled.json` | `include_str!("bundled.json")` in `src/coding_agent/catalog.rs` |
| Native screen fixtures | `src/coding_agent/native/<backend>.json` | `include_bytes!("native/<backend>.json")` in `catalog.rs` `test_support`, keyed `native/<backend>.json`, matching each `bundled.json` fixture entry's `source_ref` |

Install targets on the user's machine — `~/.claude/skills/`, `<pi base>/extensions/cafleet.ts`, `<preset base>/plugins/cafleet.ts`, `<codex base>/rules/cafleet.rules` and the like — are a separate namespace that shares directory names with the source trees. They do not move.

---

## Specification

### Target layout

```
assets/
├── extensions/   # was extensions/            (hermes/, opencode/, pi/)
├── migrations/   # was migrations/            (V1__baseline.sql)
├── presets/      # was presets/               (codex/, opencode/)
├── profiles/     # was src/coding_agent/      (bundled.json, native/<backend>.json)
└── skills/       # was skills/                (cafleet/, cafleet-design-doc/)
```

Move every file with `git mv`, keeping its content and relative position inside its tree. `src/coding_agent/bundled.json` becomes `assets/profiles/bundled.json` and `src/coding_agent/native/` becomes `assets/profiles/native/`, so each fixture entry's `source_ref` (`native/<backend>.json`) still resolves relative to the catalog file.

**Why `profiles/`.** `bundled.json` is the bundled runner-profile catalog (`ProfileCatalog`), and the native screen fixtures exist only as the evidence that backs those profiles' capabilities; `SPEC.md` already groups them under "Bundled profiles and native screen fixtures". `catalog/` would be too generic beside the other kinds, and `coding-agents/` would be wrong because presets and extensions are also per backend.

`git mv` of a directory renames the whole directory, so the ignored interpreter cache `extensions/hermes/cafleet/__pycache__/` moves with its tree to `assets/extensions/hermes/cafleet/__pycache__/` and leaves no `extensions/` behind. `build.rs` `is_excluded` keeps excluding it from the embedded table.

`git mv` needs the destination's parent directory to exist, so create `assets/profiles/` (which also creates `assets/`) before the moves.

### Build and embedding

`build.rs` reads one asset root:

```rust
fn main() {
    println!("cargo:rerun-if-changed=assets");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let assets = Path::new(&manifest_dir).join("assets");

    let mut generated = String::new();
    emit_table(&mut generated, "SKILLS", &assets.join("skills"));
    emit_table(&mut generated, "PRESETS", &assets.join("presets"));
    emit_table(&mut generated, "EXTENSIONS", &assets.join("extensions"));
    // unchanged OUT_DIR write
}
```

`walk`, `is_excluded` and `emit_table` stay as they are. Keys stay relative to each subtree root (`pi/cafleet.ts`, `codex/cafleet.rules`, `cafleet/SKILL.md`), so `lookup`, `embedded_extension` and every install path see identical keys. Cargo scans a `rerun-if-changed` directory recursively, so the single line covers all five subtrees. Rustc's dependency tracking covers the `include_str!`/`include_bytes!` files independently of the build script.

| File | Old literal | New literal |
|---|---|---|
| `src/db.rs` | `include_str!("../migrations/V1__baseline.sql")` | `include_str!("../assets/migrations/V1__baseline.sql")` |
| `src/coding_agent/catalog.rs` | `include_str!("bundled.json")` | `include_str!("../../assets/profiles/bundled.json")` |
| `src/coding_agent/catalog.rs` (`test_support`) | `include_bytes!("native/<backend>.json")`, one per backend | `include_bytes!("../../assets/profiles/native/<backend>.json")` |

The `BUNDLED_FIXTURES` key strings (`"native/<backend>.json"`), the `bundled.json is valid JSON` expect message and every test-local `"native/…"` source constant stay unchanged. They name catalog-relative references, not repository paths.

### Path classification

Only repository-source paths change. Every occurrence falls into one of these classes:

| Class | Examples | Action |
|---|---|---|
| Repository source path | `presets/codex/cafleet.rules`, `skills/cafleet/reference/cli.md`, `migrations/V1__baseline.sql`, `coding_agent/native/<backend>.json` | Rewrite to `assets/…` |
| Relative Markdown link into a source tree | `../../skills/…` in `.claude/rules/`, `../../../skills/…` in `.claude/skills/<name>/` | Insert `assets/` after the `../` prefix |
| GitHub `blob/main` link into a source tree | `https://github.com/himkt/cafleet/blob/main/skills/cafleet/reference/coding-agents.md#…` | Rewrite to `blob/main/assets/skills/…`; links pinned to older refs keep resolving at their own ref |
| Install target or user-machine path | `~/.claude/skills/**`, `<pi base>/extensions/cafleet.ts`, `base.join("extensions/cafleet.ts")` in tests, a directory's `skills/**` | Unchanged |
| Generic prose for kinds of asset | "embedded skills/presets", `Install skills` task descriptions | Unchanged |
| Catalog-relative reference | `source_ref` `native/<backend>.json`, `BUNDLED_FIXTURES` keys | Unchanged |
| Point-in-time records | `design-docs/*/design-doc.md` other than this one | Unchanged |

### SPEC.md

`SPEC.md` § Asset installation (§6.3 `cafleet setup`) becomes the single owner of the tree layout. Open the section with this paragraph and table, before the existing "Each agent's presets and extensions…" paragraph:

```markdown
The binary embeds every data file from the repository's `assets/` tree at build time:

| Source tree | Contents | Consumer |
|---|---|---|
| `assets/skills/` | One directory per skill, embedded file by file | Skill installation (this section) |
| `assets/presets/` | One directory per backend with presets | Preset installation (table below) |
| `assets/extensions/` | One directory per backend with an extension | Extension installation (table below) |
| `assets/migrations/` | The schema baseline `V1__baseline.sql` | Baseline initialization (§8 Database schema) |
| `assets/profiles/` | The bundled catalog `bundled.json` and the native screen fixtures `native/<backend>.json` | The profile catalog (§6.9 Bundled profiles and native screen fixtures) |
```

Every other `SPEC.md` section keeps its own interaction with the fact and names the full `assets/` path:

| Location | Change |
|---|---|
| § Asset installation install table | Source column becomes `assets/presets/codex/cafleet.rules`, `assets/presets/opencode/cafleet.md`, `assets/presets/opencode/cafleet-ask.md`, `assets/extensions/opencode/cafleet.ts`, `assets/extensions/hermes/cafleet/` (every file) and `assets/extensions/pi/cafleet.ts`; targets unchanged |
| §6.7 codex rules file | Both `presets/codex/cafleet.rules` mentions become `assets/presets/codex/cafleet.rules` |
| §6.7 Preset file format | `presets/opencode/cafleet.md` and `presets/opencode/cafleet-ask.md` become their `assets/presets/opencode/` forms, re-wrapping the paragraph's existing hard line breaks |
| §6.7 Host evidence and native-input authorization (the **claude**, **codex** and **pi** paragraphs) | The three `blob/main/skills/cafleet/reference/coding-agents.md#…` links become `blob/main/assets/skills/cafleet/reference/coding-agents.md#…`, anchors unchanged |
| §6.9 Bundled profiles and native screen fixtures | The fixture-registry sentence names `assets/profiles/native/<backend>.json` instead of `coding_agent/native/<backend>.json`, adding `(§6.3 Asset installation)` |
| §6.9 capture-to-fixture conversion | `coding_agent/native/<backend>.json` becomes `assets/profiles/native/<backend>.json` |
| §7.6 Packaging & distribution | "Embed the schema baseline, skills and presets offline." becomes "Embed the `assets/` tree (§6.3 Asset installation) offline." |
| §8 Database schema | "one complete schema baseline, `V1__baseline.sql`" becomes "one complete schema baseline, `assets/migrations/V1__baseline.sql` (§6.3 Asset installation)" |

The §1 product sentence ("embedded skills/presets") and the §6.7 claude settings sentence about a relocated directory's `skills/**` stay as written (generic prose and install path).

### docs/

`docs/docs/contributing.md` is the only docs page with repository paths (its project-structure table is an exempt contributor surface).

| Location | New content |
|---|---|
| Project structure, `src/` row | See the row text below |
| Project structure, rows `migrations/`, `skills/`, `presets/`, `extensions/` | Replaced by one `assets/` row placed directly after `src/`; see the row text below |
| Extend a coding-agent backend, step 5 | `skills/cafleet/reference/coding-agents.md` becomes `assets/skills/cafleet/reference/coding-agents.md` |

The two project-structure rows read:

```markdown
| `src/` | Rust CLI, typed synchronous SQLite broker, multiplexer/process adapters; built by the root `Cargo.toml` and `build.rs` together with the embedded `assets/` tree |
| `assets/` | Data the binary embeds at build time: `assets/skills/` installed agent-facing skills, roles and references; `assets/presets/` backend permission/configuration assets; `assets/extensions/` one directory per backend whose plugin or extension `cafleet setup` installs, for Director reception, goals and, where the backend has no native per-tool approval, the member gate; `assets/migrations/` the SQL schema baseline; `assets/profiles/` the bundled runner-profile catalog and one native screen fixture file per backend with catalogued screens |
```

`docs/docs/concepts/coding-agents.md`, `docs/docs/concepts/overview.md`, `docs/docs/quickstart.md` and `docs/docs/mixed-backend-team.md` mention only install paths or the generic "embedded skills/presets" phrase and stay unchanged; user pages describe behavior, not repository layout. `README.md` names no source path and stays unchanged.

### Project guidance

| File | Change |
|---|---|
| `CLAUDE.md` | The Project sentence "Skills and presets are embedded assets; the documentation site retains its own Node/pnpm toolchain." becomes "The binary embeds its data files from `assets/`; the documentation site retains its own Node/pnpm toolchain." |
| `.claude/rules/code-quality.md` | § Database schema: `migrations/V1__baseline.sql` → `assets/migrations/V1__baseline.sql` and "the only file in `migrations/`" → "the only file in `assets/migrations/`". § Test placement: "the data files that production code embeds (for example the backend fixtures, `bundled.json`, skills, presets and the schema baseline)" → "the data files that production code embeds (every file under `assets/`)". |
| `.claude/rules/commands.md` | Both `skills/cafleet/roles/member.md` mentions → `assets/skills/cafleet/roles/member.md` |
| `.claude/rules/documentation-maintenance.md` | Every repository `skills/` path becomes `assets/skills/` (the Agents audience row, Implementation Order, both first-class target rows, the Coding-agent overlay paragraphs), both `../../skills/…` links become `../../assets/skills/…`, and the contract-paths sentence reads "Paths that are part of a specified contract (the embedded `assets/` tree, embedded preset paths and their install targets, the schema baseline) are `SPEC.md` content." |

### Maintainer skills

Every `.claude/skills/<name>/` file that references the skill sources is updated. Relative links `../../../skills/…` become `../../../assets/skills/…`, and backticked or plain repository paths `skills/…` become `assets/skills/…`:

| Skill | References |
|---|---|
| `cafleet-model-list-refresh/SKILL.md` | Frontmatter `description`, the opening paragraph, the model-selection links, the apply step, and the release step "the repository `skills/` tree embedded, including `skills/cafleet/reference/coding-agents.md`" |
| `clean-docs/SKILL.md`, `clean-docs/roles.md` | Runtime-binding, coordination, Director wake, task-scope, spawn-skeleton and member links, the coordination path, and the KEEP guardrail `skills/cafleet/reference/cli.md` |
| `skill-author/SKILL.md` | Every link in the reading table and both Director wake links |

`update-readme/SKILL.md` names no source path and stays unchanged. The `clean-docs` partition phrase "skills+`.claude`" names a kind of surface, not a path, and stays.

### Compatibility basis

- **Skill discovery.** gh discovers `skills/*/SKILL.md` "including when the `skills/` directory is nested under a prefix". `gh skill install <dir> --from-local` on a probe tree holding only `assets/skills/probe/SKILL.md` listed `probe` (gh 2.103.3, checked 2026-11-08). So `mise //:skill-install` and remote `gh skill install himkt/cafleet` keep working, and no other consumer requires a root-level `skills/`. No redirect stub stays at `skills/`.
- **Runtime.** Embedded keys and bytes are unchanged, so `cafleet setup` installs the same files at the same targets, `doctor` byte comparisons match the previous release, and no version bump or schema change is involved.
- **Links.** `blob/main/assets/skills/…` links resolve once this change merges; links pinned to an older tag keep resolving at that tag.

### Verification

| Check | Command | Pass condition |
|---|---|---|
| Unit tests | `mise //:test` | Exit 0 |
| Lint | `mise //:lint` | Exit 0 |
| Docs and links | `mise //docs:build` | Exit 0 |
| Pure renames | `git diff -M --name-status main` | Every moved file shows `R100`; the only modified files are the edited docs, rules, skills, `build.rs`, `src/db.rs` and `src/coding_agent/catalog.rs` |
| Old roots gone | `ls -a assets migrations presets extensions skills src/coding_agent` | `assets` lists the five subtrees; the four old roots report "No such file or directory"; `src/coding_agent` holds no `bundled.json` or `native/` |
| Rerun trigger | `mise //:typecheck`, then `ls -lt --full-time target/debug/build/cafleet-*/out/embedded_data.rs`, then `touch assets/presets/codex/cafleet.rules`, then `mise //:typecheck` and the same `ls` | The newest `embedded_data.rs` modification time advances after the touch |
| Skill discovery | `gh skill install ./ --from-local \| cut -f1` | Prints one skill name per line, one per directory under `assets/skills/`: `cafleet` and `cafleet-design-doc` |
| Residue | The residue command below | Every remaining hit is an install-target path or generic prose from § Path classification: install-target `.join("extensions/…")` / `.join("skills/…")` calls in `src/assets.rs`, `src/cli/setup.rs` and `src/coding_agent/mod.rs`; the relocated-directory `skills/**` in `SPEC.md` and `docs/docs/concepts/coding-agents.md`; "skills/presets" prose in `SPEC.md`, `docs/`, and `src/cli/mod.rs` |

Residue command:

```bash
git grep -nE "(^|[^/[:alnum:]_.~-])(\.\./)*(skills|presets|extensions|migrations)/|blob/[^/ ]+/(skills|presets|extensions|migrations)/|coding_agent/native|coding_agent/bundled" -- ':!design-docs'
```

The first alternation catches bare and `../`-relative tree paths, and the second catches GitHub `blob/<ref>/` links into an old tree; `assets/<kind>/` paths match neither, because a `/` precedes the tree name.

The skill-discovery check pipes gh's output, so gh runs non-interactively and prints its tab-separated listing without prompting or installing; it replaces `mise //:skill-install`. That keeps the active fleet's installed skills intact, as `.claude/rules/commands.md` requires. No new Rust test is added: the existing embed/install tests already compare installed bytes with the embedded tables, and the catalog tests load every fixture through `BUNDLED_FIXTURES`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] Add the embedded-asset paragraph and `Source tree | Contents | Consumer` table at the start of § Asset installation, and rewrite the install table's source column to `assets/` paths <!-- completed: 2026-11-08T09:50 -->
- [x] Rewrite both `presets/codex/cafleet.rules` mentions in §6.7 codex rules file <!-- completed: 2026-11-08T09:50 -->
- [x] Rewrite both opencode preset paths in §6.7 Preset file format, re-wrapping its hard line breaks <!-- completed: 2026-11-08T09:50 -->
- [x] Rewrite the three `blob/main/skills/…` links in the **claude**, **codex** and **pi** paragraphs of §6.7 Host evidence and native-input authorization to `blob/main/assets/skills/…` <!-- completed: 2026-11-08T09:50 -->
- [x] Rewrite the §6.9 fixture-registry and capture-to-fixture paths to `assets/profiles/native/<backend>.json`, adding the §6.3 pointer to the registry sentence <!-- completed: 2026-11-08T09:50 -->
- [x] Rewrite the embedded-set sentence in §7.6 Packaging & distribution to "Embed the `assets/` tree (§6.3 Asset installation) offline." <!-- completed: 2026-11-08T09:50 -->
- [x] Name `assets/migrations/V1__baseline.sql` with its §6.3 pointer in §8 Database schema <!-- completed: 2026-11-08T09:50 -->

### Step 2: docs/

- [x] Rewrite the `src/` row and replace the four asset rows with one `assets/` row in `docs/docs/contributing.md` § Project structure <!-- completed: 2026-11-08T09:51 -->
- [x] Rewrite the `coding-agents.md` path in contributing.md § Extend a coding-agent backend step 5 <!-- completed: 2026-11-08T09:51 -->

### Step 3: Project guidance

- [x] Rewrite the Project sentence in root `CLAUDE.md` <!-- completed: 2026-11-08T09:53 -->
- [x] Update `.claude/rules/code-quality.md` § Database schema and § Test placement <!-- completed: 2026-11-08T09:53 -->
- [x] Update both member-role paths in `.claude/rules/commands.md` <!-- completed: 2026-11-08T09:53 -->
- [x] Update every `skills/` path, both `../../skills/` links and the contract-paths sentence in `.claude/rules/documentation-maintenance.md` <!-- completed: 2026-11-08T09:53 -->

### Step 4: Maintainer skills

- [x] Update every source path and link in `.claude/skills/cafleet-model-list-refresh/SKILL.md`, frontmatter included <!-- completed: 2026-11-08T09:55 -->
- [x] Update every source path and link in `.claude/skills/clean-docs/SKILL.md` and `.claude/skills/clean-docs/roles.md` <!-- completed: 2026-11-08T09:55 -->
- [x] Update every link in `.claude/skills/skill-author/SKILL.md` <!-- completed: 2026-11-08T09:55 -->

### Step 5: Move the trees and update embedding

- [x] Create the move destinations with `mkdir -p assets/profiles` <!-- completed: 2026-11-08T09:51 -->
- [x] `git mv` `migrations`, `presets`, `extensions` and `skills` to `assets/migrations`, `assets/presets`, `assets/extensions` and `assets/skills` <!-- completed: 2026-11-08T09:51 -->
- [x] `git mv` `src/coding_agent/bundled.json` to `assets/profiles/bundled.json` and `src/coding_agent/native` to `assets/profiles/native` <!-- completed: 2026-11-08T09:51 -->
- [x] Replace the four `rerun-if-changed` lines with `assets` and read the three tables from `assets/<kind>` in `build.rs` <!-- completed: 2026-11-08T09:51 -->
- [x] Point `src/db.rs` `BASELINE_SQL` at `../assets/migrations/V1__baseline.sql` <!-- completed: 2026-11-08T09:51 -->
- [x] Point `src/coding_agent/catalog.rs` `BUNDLED_DEFINITIONS` and the five fixture `include_bytes!` at `../../assets/profiles/…`, keeping every `native/<backend>.json` key <!-- completed: 2026-11-08T09:51 -->

### Step 6: Verify

- [x] `mise //:test` passes <!-- completed: 2026-11-08T09:53 -->
- [x] `mise //:lint` passes <!-- completed: 2026-11-08T09:53 -->
- [x] `mise //docs:build` passes <!-- completed: 2026-11-08T09:53 -->
- [x] `git diff -M --name-status main` shows `R100` for every moved file and `ls` confirms the old roots are gone <!-- completed: 2026-11-08T09:53 -->
- [x] The rerun-trigger check shows `embedded_data.rs` regenerated after touching `assets/presets/codex/cafleet.rules` <!-- completed: 2026-11-08T09:53 -->
- [x] `gh skill install ./ --from-local | cut -f1` prints `cafleet` and `cafleet-design-doc` <!-- completed: 2026-11-08T09:53 -->
- [x] The residue `git grep` returns only the install-target and generic-prose hits listed in § Verification <!-- completed: 2026-11-08T09:53 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-08 | Initial draft |
| 2026-11-08 | Implemented; status Complete |
