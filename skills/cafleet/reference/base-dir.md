# Base Directory Resolution

This file owns the generic resolver and scratch/audit/figure write contract. Consumers supply their task convention; members inherit the Director's result.

## Member input and write states

Read this contract at member startup. Use a supplied absolute `BASE:` literally; members never re-resolve BASE or derive another root. An explicit consumer-supplied absolute output target remains valid independently of audit BASE.

| Input/state | Intended behavior |
|---|---|
| Absolute `BASE:` supplied | Root scratch, audit and figure writes there, or at the consumer's explicit absolute target. |
| `BASE:` omitted | Audit files are disabled. Send `audit-disabled no BASE in spawn prompt` once to the Director as a parens-free anchorless status. Use an explicit consumer output target when supplied. |
| Resolver returns the exact case-sensitive `<unset>` sentinel | Guard and skip audit writes; omit `BASE:` from spawn prompts. |
| Unguarded sentinel path construction | Fail with `Error: BASE is <unset>; refusing to fall back to /tmp`. |

The anchorless status follows the shared [coordination convention](../../cafleet-design-doc/reference/coordination.md). Use no ad-hoc temporary fallback. `/tmp/cafleet` is a valid BASE when selected through the resolver's decision branch below.

## No-bypass write protocol

1. Write scratch, audit, figures and intermediate spawn renders under resolved BASE or the explicit consumer-supplied absolute target. Every audit write checks `${BASE} != <unset>` before constructing a path; guarded skip is the defined disabled state.
2. Classify output before creating its folder: agent-only scratch/audit/build inputs use hidden dot-prefixed folders such as `.prompts/`; user-facing deliverables use visible folders.
3. Write each spawn prompt before calling `member create --file <absolute-path>`. That pre-substitution input is the permanent audit artifact. Read the Director's [audit convention](../roles/director.md#member-create--scratch-and-audit-files) and [prompt-size contract](../roles/director.md#spawn-prompt-size-limit) before rendering/writing/spawning: they own UTC names, collision suffixes, four placeholders and the compact inline fallback when BASE is unset.
4. Preserve member inheritance: use the literal supplied BASE or the missing-line disabled state above. The resolver is Director/consumer work.

## The `<unset>` sentinel

Only the absolute-path branch of the resolver produces `<unset>`. Omit the BASE line entirely in a disabled-audit spawn; never write `BASE: <unset>`. The call site explicitly guards each audit write, while an unguarded `Path(BASE) / …` raises the exact error above. Explicit consumer document/output paths remain available.

## Procedure

The resolver writes nothing. It uses `git rev-parse --show-toplevel` for task scope and {decision_surface} only for the shared-root HOME/config branch. Create the selected folder lazily on the first consumer write. Consumers use this one resolver rather than computing BASE independently.

### Step 0. Task-scope resolution

Use task scope when the consumer operates on a task folder; otherwise use Step 1.

1. Run `git rev-parse --show-toplevel` from CWD. A non-zero exit or empty output means no Git ancestor: stop and tell the user to change to the repository root and retry.
2. Normalize the consumer's argument to its actual task folder **before** containment resolution. The resolver receives a relative or absolute folder, not a child filename. [Design-doc File Layout](../../cafleet-design-doc/reference/guidelines.md#file-layout) owns design-doc normalization; other consumers own their bucket conventions.
3. Resolve the folder against the repository root when relative. Reject a relative path that equals the root (`the repo root is not a task folder`) or escapes it (`refusing to create a task folder outside the repo`). Resolve an absolute path directly: if it equals the root or is not strictly below it, return `<unset>` and create nothing.
4. For a folder strictly below the repository root, return its absolute path as BASE. Lazy output creation places audits under the task folder.

The resolver performs no bucket prefixing, slug folding or filename stripping.

### Step 1. Shared-root resolution

Determine CWD and HOME. Check all backend config roots regardless of the executing backend:

| Backend | User-level config root |
|---|---|
| claude | `~/.claude` |
| codex | `~/.codex` |
| opencode | `~/.config/opencode` |

If CWD is neither HOME itself nor a listed config root/subtree, BASE is CWD itself, not the repository root. An ordinary project elsewhere under HOME therefore resolves directly. Otherwise proceed to Step 2 with candidates `/tmp/cafleet` and the literal CWD.

### Step 2. Decision-surface prompt

Ask through {decision_surface}: `Select the base directory for output files:`. Offer `/tmp/cafleet (recommended)`, the literal CWD, and the backend's free-text choice. Use the selected path; resolve relative free text against CWD. Persist nothing: a later resolution from HOME/config scope asks again.
