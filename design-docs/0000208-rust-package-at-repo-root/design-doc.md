# Rust package at the repository root

**Status**: Approved
**Progress**: 27/27 tasks complete
**Last Updated**: 2026-11-02

## Overview

Move the CAFleet Rust package from `cafleet/` to the repository root, because the only other package is the `docs/` Node site. The Rust tasks move into the root `mise.toml`, so every `mise //cafleet:<task>` becomes `mise //:<task>` as a hard break with no aliases, and every build, CI, release, permission and documentation surface follows the new layout.

## Success Criteria

- [ ] `Cargo.toml`, `Cargo.lock`, `build.rs`, `src/` and `migrations/` exist at the repository root, `git mv` history is preserved (`git log --follow src/main.rs` reaches commits before the move), and the `cafleet/` directory no longer exists in the working tree.
- [ ] `build.rs` contains no `../` path; it embeds `skills/` and `presets/` from beside the manifest.
- [ ] The root `mise.toml` defines `test`, `lint`, `format`, `typecheck`, `install` and `build`; `monorepo.config_roots` is `["docs"]`; `mise tasks --all` lists those tasks as `//:<task>` and lists no `//cafleet:` task.
- [ ] `mise //:test`, `mise //:lint`, `mise //:typecheck` and `mise //docs:build` pass.
- [ ] `cargo build --release` run at the repository root produces `target/release/cafleet`.
- [ ] ``git grep -n -E 'mise //cafleet|`cafleet/`|cafleet/(src|target|Cargo|migrations|build\.rs|mise\.toml|tests)|working-directory: cafleet' -- ':!design-docs/**'`` returns nothing. On the pre-change tree this pattern matches every surface listed in the Specification and nothing else (`skills/cafleet/…`, `himkt.github.io/cafleet/`, `https://cafleet-probe-one.invalid/…` in the embedded Claude profile and `["cafleet", …]` stay unmatched).
- [ ] `git grep -n '\.\./' -- build.rs` returns nothing.
- [ ] Design documents `0000001`–`0000207` are byte-identical to their pre-change content.

---

## Background

The repository is a mise monorepo whose root `mise.toml` lists two `config_roots`: `cafleet` (the Rust package with its own `mise.toml`) and `docs` (the Node/pnpm site). With no other Rust or admin-UI package left, the `cafleet/` subdirectory only adds a path segment to every build, CI and documentation reference, and forces `build.rs` to reach into `../skills` and `../presets`.

Rust sources need no edits: `include_str!`/`include_bytes!` resolve relative to the source file, and `refinery::embed_migrations!("migrations")` resolves relative to `CARGO_MANIFEST_DIR`, so both keep working once `src/` and `migrations/` sit beside the manifest.

---

## Specification

### Repository layout

| Path before | Path after | Change |
|---|---|---|
| `cafleet/Cargo.toml` | `Cargo.toml` | `git mv`, content unchanged |
| `cafleet/Cargo.lock` | `Cargo.lock` | `git mv`, content unchanged |
| `cafleet/build.rs` | `build.rs` | `git mv`, then the path edit below |
| `cafleet/src/` | `src/` | `git mv`, content unchanged |
| `cafleet/migrations/` | `migrations/` | `git mv`, content unchanged |
| `cafleet/mise.toml` | — | Deleted; tasks merge into the root `mise.toml` |
| `cafleet/target/` (ignored, local) | `target/` | The local build cache under `cafleet/target/` is deleted from the working tree so `cafleet/` disappears; builds recreate `target/` at the root |

The move is a single commit of `git mv` operations plus the path edits that keep that commit building, so rename detection keeps per-file history. The Director makes that commit after `mise //:test` passes on the moved tree.

### `build.rs`

Assets sit beside the manifest. Replace the four `../` references:

```rust
println!("cargo:rerun-if-changed=migrations");
println!("cargo:rerun-if-changed=skills");
println!("cargo:rerun-if-changed=presets");

let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
let skills = Path::new(&manifest_dir).join("skills");
let presets = Path::new(&manifest_dir).join("presets");
```

The rest of `build.rs` is unchanged; `walk` already skips hidden entries, so the root's dotfiles never enter the embed tables (the walk is rooted at `skills/` and `presets/`, not the repository root).

### Root `mise.toml`

The final file, with the Rust tasks appended after the existing root tasks and their `run`/`description` values carried over verbatim except the `build` description and the `all-install` reference:

```toml
monorepo_root = true

[monorepo]
config_roots = [
  "docs",
]

[tools]
"aqua:pnpm/pnpm" = "12.15.0"
"core:node" = "26.12.1"
"core:rust" = { version = "1.99.1", components = ["clippy", "rustfmt"] }
"core:zig" = "0.15.2"

[tasks.skill-install]
run = [
  "gh skill install ./ --from-local --agent claude-code --force --scope user",
  "gh skill install ./ --from-local --agent codex --force --scope user",
  "gh skill install ./ --from-local --agent opencode --force --scope user",
]
description = "Install skills"

[tasks.all-install]
run = [{ task = "//:install" }, { task = "//:skill-install" }]
description = "Install the CLI and the working-tree skills"

[tasks.test]
run = "cargo test --bin cafleet"
description = "Run inline unit tests (positional args forward to the cargo test harness)"

[tasks.lint]
run = ["cargo clippy --all-targets -- -D warnings", "cargo fmt --check"]
description = "Run clippy (deny warnings) and the rustfmt check"

[tasks.format]
run = "cargo fmt"
description = "Format the Rust sources with rustfmt"

[tasks.typecheck]
run = "cargo check"
description = "Type-check the package without producing a binary"

[tasks.install]
run = "cargo install --path ."
description = "Install the cafleet CLI from the working tree (re-run after source edits to refresh the installed binary)"

[tasks.build]
run = "cargo build --release"
description = "Build the release binary into target/release/cafleet"
```

Task names stay short (`//:install` sits beside `//:skill-install` and `//:all-install` without collision). Root tasks run from the repository root, which is now the manifest directory, so no `dir` setting is needed.

### Task path mapping

Every old task path maps to exactly one new path; no alias task is defined.

| Before | After |
|---|---|
| `mise //cafleet:test` | `mise //:test` |
| `mise //cafleet:lint` | `mise //:lint` |
| `mise //cafleet:format` | `mise //:format` |
| `mise //cafleet:typecheck` | `mise //:typecheck` |
| `mise //cafleet:install` | `mise //:install` |
| `mise //cafleet:build` | `mise //:build` |

### Permission surfaces

| Surface | Before | After |
|---|---|---|
| `.claude/settings.json` `permissions.allow` | `Bash(mise //cafleet*)` | Six entries in its place: `Bash(mise //:test*)`, `Bash(mise //:lint)`, `Bash(mise //:format)`, `Bash(mise //:typecheck)`, `Bash(mise //:build)`, `Bash(mise //:install)` |
| `presets/opencode/cafleet.md` and `presets/opencode/cafleet-ask.md` `permission.shell` | `mise //cafleet:test`, `mise //cafleet:test *`, `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:build` | The same six entries rewritten to `mise //:<task>` in the same positions; install stays excluded |

The settings entries preserve today's authority exactly: `//:skill-install` and `//:all-install` (which overwrite user-scope installed skills) stay unallowed. The two opencode presets keep identical allowlists, as SPEC.md requires. The preset change is a contract hard break: an operator's previously installed presets keep the old patterns until `cafleet setup` reinstalls them, and no migration note is written anywhere.

### CI, release and versioning

`.github/workflows/ci.yml` cache and run step:

```yaml
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ matrix.task }}-${{ hashFiles('Cargo.lock') }}
          restore-keys: ${{ runner.os }}-cargo-${{ matrix.task }}-
      - run: mise //:${{ matrix.task }}
```

`.github/workflows/release.yml`: remove both `working-directory: cafleet` lines, and replace `cafleet/target/${{ matrix.target }}/release` with `target/${{ matrix.target }}/release` in the archive `tar -C` argument and in the bottle `cp` source. The archive and bottle names are unchanged.

`.bumpversion.toml`: the two `filename` values become `Cargo.toml` and `Cargo.lock`; search/replace patterns are unchanged. The version bump itself stays with the Bump workflow.

`.gitignore`: `/cafleet/target/` becomes `/target/`.

`docs.yml`, `bump.yml` and `renovate.jsonc` need no edit (Renovate's cargo manager discovers the root manifest).

### Documentation surfaces

Documentation is updated before code, in the order of `.claude/rules/documentation-maintenance.md` § Implementation Order.

| File | Edit |
|---|---|
| `SPEC.md` §6.7 opencode preset usage-limit table | The interpreter-execution example becomes `mise //:test *`. The allowlist prose ("the project's cargo-backed mise tasks") stays as written. |
| `docs/docs/contributing.md` § Project structure | Replace the `cafleet/` row with a `src/` row ("Rust CLI, typed synchronous SQLite broker, multiplexer/process adapters; built by the root `Cargo.toml` and `build.rs`, which embeds `skills/` and `presets/`") and a `migrations/` row ("Embedded SQL migration chain"). |
| `docs/docs/contributing.md` § Development | The lead-in "Enter the checkout, then install the CLI through its package task:" becomes "Enter the checkout, then install the CLI through its root task:"; `mise //cafleet:install` → `mise //:install`; validation becomes `mise //:test` and `mise //:lint`; the task-ownership sentence states that the root `mise.toml` pins tools and owns the Rust tasks, and selects the documentation site's task file through `monorepo.config_roots`. |
| `docs/docs/mixed-backend-team.md` | Both `mise //cafleet:test` examples become `mise //:test`. |
| `README.md` | No affected mention; verified by the grep criterion. |
| `skills/cafleet-design-doc/reference/coordination.md` | Both `cafleet/src/cli/member.rs:142` pointer examples become `src/cli/member.rs:142`. |
| `.claude/rules/code-quality.md` | `mise //cafleet:test` → `mise //:test`; forbidden path `cafleet/tests/` → `tests/`; `cafleet/migrations/` → `migrations/`; `cafleet/src/db.rs` → `src/db.rs`. |
| `.claude/rules/commands.md` | Opening paragraph: the root `mise.toml` pins tools and owns the Rust tasks; `monorepo.config_roots` selects the documentation site's package task file, which owns its tasks and working directory. Validation becomes `mise //:test` and `mise //:lint`, with `//:typecheck` as a registry task; the test-filter example becomes `mise //:test test_name`. |
| `CLAUDE.md` | "The Rust package under `cafleet/`" → "The Rust package at the repository root"; the Commands sentence states that the root `mise.toml` owns the Rust tasks and selects the documentation site's task file through `monorepo.config_roots`. |

Past design documents (`design-docs/0000001`–`0000207`) are point-in-time records and stay unchanged.

### Test behavior at the new root

`src/runtime/system.rs` tests assert the test process cwd equals `CARGO_MANIFEST_DIR` and create hidden `.path-test-*` / `.process-test-*` temporary entries there. After the move these are created at the repository root; the `.gitignore` `.*` rule already ignores them and `tempfile` removes them on drop. No test change is needed.

### Execution in an active fleet

Installed runtime assets of the executing fleet are preserved during implementation (`.claude/rules/commands.md`). Once the move lands, a member whose harness still carries the old allow rules (an installed opencode preset, or a Claude session that loaded the previous `.claude/settings.json`) can be denied `mise //:<task>`; it routes the exact command to the Director under the member command-routing protocol. Reinstalling the CLI and presets is a post-merge operator step, outside this change.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Specification and user documentation

- [x] Update `SPEC.md` §6.7 opencode preset usage-limit table example to `mise //:test *` <!-- completed: 2026-11-02T14:17 -->
- [x] Update `docs/docs/contributing.md` § Project structure (`src/` and `migrations/` rows replace `cafleet/`) <!-- completed: 2026-11-02T14:17 -->
- [x] Update `docs/docs/contributing.md` § Development (install lead-in, install/validation commands and task-ownership sentence) <!-- completed: 2026-11-02T14:17 -->
- [x] Update both `mise //cafleet:test` examples in `docs/docs/mixed-backend-team.md` <!-- completed: 2026-11-02T14:17 -->
- [x] Confirm `README.md` has no affected mention <!-- completed: 2026-11-02T14:17 -->

### Step 2: Skills and project rules

- [x] Update both pointer examples in `skills/cafleet-design-doc/reference/coordination.md` to `src/cli/member.rs:142` <!-- completed: 2026-11-02T14:17 -->
- [x] Update `.claude/rules/code-quality.md` (task path, `tests/`, `migrations/`, `src/db.rs`) <!-- completed: 2026-11-02T14:17 -->
- [x] Update `.claude/rules/commands.md` (task ownership, validation task paths, filter example) <!-- completed: 2026-11-02T14:17 -->
- [x] Update `CLAUDE.md` (package location and Commands sentence) <!-- completed: 2026-11-02T14:17 -->

### Step 3: Move the package

- [x] Run `git mv cafleet/Cargo.toml cafleet/Cargo.lock cafleet/build.rs cafleet/src cafleet/migrations .` from the repository root <!-- completed: 2026-11-02T14:30 -->
- [x] Edit `build.rs` to drop every `../` (rerun-if-changed and asset roots) <!-- completed: 2026-11-02T14:30 -->
- [x] Merge the Rust tasks into the root `mise.toml`, set `config_roots = ["docs"]`, point `all-install` at `//:install`, update the `build` description; `git rm cafleet/mise.toml` <!-- completed: 2026-11-02T14:30 -->
- [x] Change `.gitignore` `/cafleet/target/` to `/target/` <!-- completed: 2026-11-02T14:30 -->
- [x] Delete the local ignored `cafleet/target/` so the `cafleet/` directory no longer exists <!-- completed: 2026-11-02T14:30 -->
- [x] Director commits the move as one commit, once `mise //:test` passes on the moved tree, so rename detection keeps history <!-- completed: 2026-11-02T14:30 -->

### Step 4: Permission surfaces

- [x] Replace `Bash(mise //cafleet*)` in `.claude/settings.json` with the six enumerated `Bash(mise //:…)` entries <!-- completed: 2026-11-02T14:31 -->
- [x] Rewrite the six `mise //cafleet:*` entries in `presets/opencode/cafleet.md` to `mise //:<task>` in place <!-- completed: 2026-11-02T14:31 -->
- [x] Rewrite the six `mise //cafleet:*` entries in `presets/opencode/cafleet-ask.md` identically <!-- completed: 2026-11-02T14:31 -->

### Step 5: CI, release and versioning

- [x] Update `.github/workflows/ci.yml` cache path (`target`), `hashFiles('Cargo.lock')` and run step `mise //:${{ matrix.task }}` <!-- completed: 2026-11-02T14:31 -->
- [x] Update `.github/workflows/release.yml`: drop both `working-directory: cafleet`, use `target/${{ matrix.target }}/release` in the `tar -C` and `cp` paths <!-- completed: 2026-11-02T14:31 -->
- [x] Update `.bumpversion.toml` filenames to `Cargo.toml` and `Cargo.lock` <!-- completed: 2026-11-02T14:31 -->

### Step 6: Verification

- [x] `mise tasks --all` lists `//:test`, `//:lint`, `//:format`, `//:typecheck`, `//:install`, `//:build` and no `//cafleet:` task <!-- completed: 2026-11-02T14:36 -->
- [x] `mise //:test` and `mise //:lint` pass <!-- completed: 2026-11-02T14:36 -->
- [x] `mise //:typecheck` and `mise //docs:build` pass <!-- completed: 2026-11-02T14:36 -->
- [x] `cargo build --release` at the root produces `target/release/cafleet` <!-- completed: 2026-11-02T14:36 -->
- [x] The Success Criteria stale-reference `git grep` (`mise //cafleet|`…) and the `build.rs` `../` `git grep` both return nothing <!-- completed: 2026-11-02T14:36 -->
- [x] `git diff --stat main -- design-docs` shows only `design-docs/0000208-rust-package-at-repo-root/design-doc.md`, and `git log --follow src/main.rs` reaches pre-move history <!-- completed: 2026-11-02T14:36 -->
