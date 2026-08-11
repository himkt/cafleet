# Custom Config-Path Env Vars for Coding-Agent Asset Locations

**Status**: Approved
**Progress**: 0/12 tasks complete
**Last Updated**: 2026-08-12

## Overview

The CLI hard-codes each coding-agent backend's user-level config directories when installing and checking assets. This design makes every path-computing site resolve those directories through the backends' native config-location environment variables — `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, and `OPENCODE_CONFIG_DIR` — falling back to today's defaults when unset (GitHub issue #296).

## Success Criteria

- [ ] `cafleet setup` installs skills and presets under the env-var-resolved directories when the variables are set (opencode skills excepted — their install dir is a fixed discovery path), and under today's defaults when unset.
- [ ] `cafleet member create --coding-agent opencode` checks the preset-existence spawn precondition at the same resolved path `setup` installs to.
- [ ] A set variable whose value is not an absolute path (including the empty string) fails loudly with the pinned error message and exit 1.
- [ ] Resolution is stateless per invocation — no schema change, no recorded paths.
- [ ] `SPEC.md`, `docs/docs/spec/cli-options.md`, `docs/docs/spec/coding-agent-backends.md`, and `docs/docs/quickstart.md` describe the resolution.
- [ ] Unit tests cover the resolver (all four outcomes per variable) and both consuming sites via injected lookups.

---

## Background

Three sites compute backend config-dir paths today, all hard-coded against `$HOME`:

| Site | Location | Paths |
|---|---|---|
| `setup` skills install | `cafleet/src/assets.rs` `skills_dir` | `~/.claude/skills`, `~/.codex/skills`, `~/.config/opencode/skills` |
| `setup` preset install | `cafleet/src/assets.rs` `preset` | `~/.codex/rules/cafleet.rules`, `~/.opencode/agents/cafleet.md` |
| opencode spawn precondition | `cafleet/src/coding_agent/opencode.rs` `ensure_available` | `~/.opencode/agents/cafleet.md` |

The stale-assets guard and `cafleet doctor`'s assets report read only the `asset_installs` DB table (agent + version, no paths) and stay unchanged; `doctor` gains no filesystem checks. Any path a future check ever computes must go through the shared resolver introduced here.

Backend variable semantics, per each backend's official docs: `CLAUDE_CONFIG_DIR` and `CODEX_HOME` relocate the whole config directory (`~/.claude` / `~/.codex`). opencode's `OPENCODE_CONFIG` names a config **file**; the directory-shaped variable is `OPENCODE_CONFIG_DIR`, documented as "searched for agents, commands, modes, and plugins just like the standard `.opencode` directory" — skills are absent from that list, and opencode's skills docs enumerate only fixed discovery paths (`~/.config/opencode/skills` among the user-level ones). cafleet therefore reads `OPENCODE_CONFIG_DIR` for the preset base only, keeps the opencode skills install at its fixed discovery path, and ignores `OPENCODE_CONFIG`.

The agent-executed base-dir resolution procedure in the cafleet skill (its user-level config-dir table) is out of scope: this design touches only the CLI.

---

## Specification

### Resolution table

| Backend | Variable | Base when set | Base when unset | Skills dir | Preset target |
|---|---|---|---|---|---|
| claude | `CLAUDE_CONFIG_DIR` | `$CLAUDE_CONFIG_DIR` | `~/.claude` | `<base>/skills` | — |
| codex | `CODEX_HOME` | `$CODEX_HOME` | `~/.codex` | `<base>/skills` | `<base>/rules/cafleet.rules` |
| opencode (skills) | — (fixed discovery path) | — | `~/.config/opencode` | `<base>/skills` | — |
| opencode (preset) | `OPENCODE_CONFIG_DIR` | `$OPENCODE_CONFIG_DIR` | `~/.opencode` | — | `<base>/agents/cafleet.md` |

opencode splits by purpose because its discovery rules differ. `agents/` is in `OPENCODE_CONFIG_DIR`'s documented search list, so the preset may relocate to `<custom>/agents/cafleet.md` and remain a valid `--agent cafleet` discovery path; when the variable is unset it keeps today's `~/.opencode/agents/cafleet.md` (SPEC §6.7's mandated discovery path). Skills are not in that search list — opencode discovers them only at fixed paths, of which `~/.config/opencode/skills` is cafleet's user-level install target — so the skills install ignores the variable.

### Validation

A set variable must hold an absolute path. Any other value — the empty string, a relative path, a literal unexpanded `~/…` — fails at resolution time with:

```
Error: <VAR> must be an absolute path (got '<value>')
```

Exit 1 (`CafleetError::App`, matching the `CAFLEET_*` numeric-validation style in `config.rs`). Validation is lazy: a variable is read and validated only when a site actually resolves that backend's directory. `cafleet setup --skip codex` with an invalid `CODEX_HOME` succeeds; `member create --coding-agent claude` never reads any of the three variables (claude and codex spawn preconditions are PATH-check-only).

### The shared resolver

New module `cafleet/src/config_dir.rs`, the single owner of backend config-dir resolution. Each env-resolving function takes an injected env lookup (the `Settings::from_lookup` pattern) and the home directory, so tests run against fakes; `opencode_skills_base` reads no variable and stays infallible:

```rust
type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

pub fn claude_config_dir(env: EnvLookup, home: &Path) -> Result<PathBuf, CafleetError>;
// CLAUDE_CONFIG_DIR | home/.claude

pub fn codex_home(env: EnvLookup, home: &Path) -> Result<PathBuf, CafleetError>;
// CODEX_HOME | home/.codex

pub fn opencode_skills_base(home: &Path) -> PathBuf;
// always home/.config/opencode — opencode discovers skills only at fixed paths

pub fn opencode_preset_base(env: EnvLookup, home: &Path) -> Result<PathBuf, CafleetError>;
// OPENCODE_CONFIG_DIR | home/.opencode
```

A private `resolve(var: &str, env: EnvLookup, default: PathBuf)` implements the shared behavior: unset → default; set and absolute → `PathBuf::from(value)`; set otherwise → the pinned error. The three variables stay out of `Settings` — `Settings` is the `CAFLEET_*` configuration surface, while these are backend-native variables read at point of use.

### Call-site changes

**`assets.rs`**: `skills_dir` and `preset` delegate to `config_dir` and become fallible; `install_assets` gains the env-lookup parameter alongside the existing `home`. `setup`'s assets half passes `&|name| std::env::var(name).ok()`. The install procedure, echo lines, and failure messages keep their exact shapes — they now print the resolved directories.

**`coding_agent`**: the `SpawnProbe` trait gains `fn env_var(&self, name: &str) -> Option<String>`. `SystemProbe` (`cli/system.rs`) reads `std::env::var(name).ok()`; the test-support `FakeProbe` gains a settable env map defaulting to empty. `opencode::ensure_available` resolves the preset as `opencode_preset_base(...)?.join("agents/cafleet.md")` — the existing `opencode agent preset not found at <preset>; run 'cafleet setup' first` message now carries the resolved path, and an invalid `OPENCODE_CONFIG_DIR` surfaces the validation error before the existence check.

### Documentation deltas

| Surface | Delta |
|---|---|
| `docs/docs/spec/cli-options.md` | The setup assets-half section gains the resolution table and the validation error; the preset install-target table and the claude/codex skills-dir mentions switch from fixed paths to resolved defaults (the opencode skills dir stays fixed); the `member create` section notes the opencode precondition resolves through `OPENCODE_CONFIG_DIR`. The `CAFLEET_*` env table stays CAFLEET-only. |
| `docs/docs/spec/coding-agent-backends.md` | The codex rules-file and opencode preset path mentions note the `CODEX_HOME` / `OPENCODE_CONFIG_DIR` override. |
| `docs/docs/quickstart.md` | The per-backend table's fixed paths gain a note that the backend config-location variables relocate them. |
| `SPEC.md` | §6.3 shared helpers (skills dirs, preset targets) specify the resolution and the pinned error string; §6.7 opencode `ensure_available`, the opencode-preset and codex-rules sections, and the `SpawnProbe` seam description follow. |

No `SKILL.md` names an install path, and the one skills-tree mention — the opencode discovery hint in `skills/cafleet-research/reference/slidev.md`, naming `~/.config/opencode/skills/` — stays accurate because this design keeps that directory fixed. `README.md`'s thin surface is untouched. Neither changes.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [ ] Update `docs/docs/spec/cli-options.md` per the documentation-deltas table <!-- completed: -->
- [ ] Update `docs/docs/spec/coding-agent-backends.md` and `docs/docs/quickstart.md` <!-- completed: -->
- [ ] Update `SPEC.md` §6.3 and §6.7 <!-- completed: -->

### Step 2: Shared resolver

- [ ] Add `cafleet/src/config_dir.rs` with the four public functions and the private `resolve` <!-- completed: -->
- [ ] Colocated tests: per variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`) — unset → default, absolute → used verbatim, empty → pinned error, relative → pinned error; `opencode_skills_base` pinned to `home/.config/opencode` <!-- completed: -->

### Step 3: Setup assets half

- [ ] Route `assets.rs` `skills_dir` / `preset` through `config_dir`; thread the env lookup from `setup.rs` <!-- completed: -->
- [ ] Tests: installs land under custom dirs when the variables are set (claude/codex skills, codex/opencode presets), defaults when unset; opencode skills stay at `~/.config/opencode/skills` even when `OPENCODE_CONFIG_DIR` is set; `--skip` prevents reading a skipped backend's variable <!-- completed: -->

### Step 4: opencode spawn precondition

- [ ] Add `SpawnProbe::env_var`; implement on `SystemProbe` and `FakeProbe` <!-- completed: -->
- [ ] Resolve the preset path in `opencode::ensure_available` via `opencode_preset_base`; tests for the custom-dir check, the default-dir check, and the invalid-variable error <!-- completed: -->

### Step 5: Verification

- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` all pass <!-- completed: -->
- [ ] Confirm the stale-assets guard and `doctor` behavior are byte-identical (no path reads added) <!-- completed: -->
- [ ] Confirm no migration is needed (`asset_installs` schema untouched) and the chain-guard test is unchanged <!-- completed: -->
