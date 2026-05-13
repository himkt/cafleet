---
name: base-dir
description: >
  Resolve the base directory for output files. Loaded by consuming skills
  via Skill(cafleet:base-dir). Do NOT invoke directly.
---

# Base Directory Resolution

`Skill(cafleet:base-dir)` is the single authoritative resolver for every CAFleet scratch / audit / figure path. The resolution outcome (`${BASE}`) is the only legitimate root for those writes. Consuming skills MUST NOT compute `${BASE}` independently and MUST NOT fall back to `/tmp` when resolution returns the `<unset>` sentinel.

`/tmp/claude-code` remains a perfectly valid resolved `${BASE}` when this skill explicitly selects it via the `AskUserQuestion` branch — only **bypassing** base-dir to write to `/tmp` without its consent is forbidden.

## Procedure

The deterministic side of resolution is implemented in Python and exposed via `cafleet base-dir resolve` / `cafleet base-dir record` (see § *CLI surface* below). Claude's job is to (a) invoke `cafleet base-dir resolve`, (b) branch on the `status` field, (c) drive `AskUserQuestion` only when `status == "needs-user-input"`, and (d) call `cafleet base-dir record` to persist the answer.

### Step 1. Probe via the CLI

Run:

```bash
cafleet base-dir resolve --json
```

If the consuming skill received an argument that is already an absolute path (e.g. `/abs/path/to/design-doc.md`), pass it through:

```bash
cafleet base-dir resolve --json --path <abs-or-rel-arg>
```

The CLI returns one of four JSON shapes:

| `status` field | Meaning | `${BASE}` outcome |
|:--|:--|:--|
| `resolved` | A concrete BASE was determined from CWD inference or from an existing anchor. The `base` field is the absolute path; the `source` field is `cwd-inference` or `anchor`. | `${BASE} = <base>`. Done. |
| `unset` | The caller passed an absolute-path argument. `${BASE}` is intentionally undefined. The `base` field is `null`; the `source` field is `absolute-path-arg`. | `${BASE} = <unset>` (literal sentinel). Done. See § *The `<unset>` sentinel* for caller obligations. |
| `needs-user-input` | The CLI could not resolve deterministically (CWD is `$HOME` or under `$HOME/.claude`, and no usable anchor exists). The `candidates` field lists the options to present. | Go to Step 2. |
| (error / non-zero exit) | Anchor schema mismatch, version mismatch, or other fatal condition. | Surface the error and stop — do NOT fall back to `/tmp`. |

### Step 2. AskUserQuestion (only when `status == "needs-user-input"`)

Present the candidates via `AskUserQuestion` ("Select the base directory for output files:"):

- `/tmp/claude-code (recommended)` → `${BASE} = /tmp/claude-code`
- `${CWD}` → `${BASE} = ${CWD}` (the CWD literal from the `candidates` array)
- `Other` (free text) → `${BASE} = user input` (resolve against `${CWD}` if relative)

Then persist the answer:

```bash
cafleet base-dir record --base <abs-path> --source askuserquestion
```

`record` is idempotent: re-running with the same `--base` is a no-op; re-running with a mismatched `--base` against an existing anchor errors and exits non-zero (the existing anchor wins; the caller must delete it manually if intentional).

### Anchor file

`cafleet base-dir record` writes an anchor file alongside the resolved BASE so that re-loading this skill (after auto-compression, after a fresh Director, after the next session) returns the same answer without re-prompting.

- **Path**: `${BASE}/.cafleet-base-dir.json`
- **Schema** (version 1):

  ```json
  {
    "version": 1,
    "base": "/abs/path/to/base",
    "source": "askuserquestion",
    "resolved_at": "2026-05-12T13:00:00.000000+00:00"
  }
  ```

  - `version`: `1`. Future versions land alongside an explicit upgrade path. `resolve` rejects any anchor whose `version` field is missing, not a positive integer, or not `1` with a single standardized error message — silent forward-compatibility would let two installations on the same machine at different cafleet versions disagree about BASE.
  - `base`: absolute path. MUST equal the directory the anchor lives in. Mismatch is a fatal error in `resolve`.
  - `source`: `"cwd-inference"` (anchor written automatically when CWD inference produced a BASE) or `"askuserquestion"` (anchor written after user picked an option).
  - `resolved_at`: ISO 8601, UTC, microsecond precision, `+00:00` suffix (same convention as cafleet `status_timestamp`).

The anchor at `/tmp/claude-code/.cafleet-base-dir.json` does NOT survive reboot — acceptable. After reboot the `AskUserQuestion` branch fires once and re-creates the anchor.

### Gitignore handling

When `${BASE}` is a git-repo CWD (the normal case in this project), the anchor file would otherwise show up in `git status`.

1. **The cafleet repo's own `.gitignore` already excludes `/.cafleet-base-dir.json` at repo root**, so the file never appears as untracked in this repo.
2. **`cafleet base-dir record` does NOT mutate any `.gitignore`** — silently editing a user repo's gitignore as a side-effect would be a surprising write.
3. **Users who do not want anchor noise in arbitrary repos should add `.cafleet-base-dir.json` to their global excludes** (`~/.config/git/ignore` or `core.excludesFile`). The first `record` call under a fresh git-repo BASE prints a one-line stderr tip pointing at this recommendation.

## CLI surface

The two subcommands live under the `cafleet` CLI as the `base-dir` subgroup. Like `cafleet doctor`, they operate on the local filesystem only and do NOT require `--session-id`.

```bash
# Non-interactive resolution. Never prompts the user. On the CWD-inference
# branch it auto-writes the anchor at ${CWD}/.cafleet-base-dir.json so the
# next call returns from "anchor" instead.
cafleet base-dir resolve [--path <abs-or-rel-arg>] [--json]

# Possible JSON outputs:
# {"status": "resolved", "base": "/home/user/proj", "source": "cwd-inference", "anchor": "/home/user/proj/.cafleet-base-dir.json"}
# {"status": "resolved", "base": "/tmp/claude-code", "source": "anchor", "anchor": "/tmp/claude-code/.cafleet-base-dir.json"}
# {"status": "unset", "base": null, "source": "absolute-path-arg", "anchor": null}
# {"status": "needs-user-input", "base": null, "source": null, "candidates": ["/tmp/claude-code", "<cwd>"]}

# Persist an anchor after AskUserQuestion. Claude calls this after presenting the picker.
cafleet base-dir record --base <abs-path> --source askuserquestion
# Writes <base>/.cafleet-base-dir.json with version=1 and a UTC resolved_at.
# Idempotent: if the anchor already exists and matches, no-op; if it mismatches, error.
```

Default text output is human-readable; `--json` switches to machine-parseable.

## No-bypass write protocol

Every CAFleet member, every consumer skill, and every Director MUST follow this protocol for scratch / audit / figure / spawn-prompt-render writes:

1. **Every write under `${BASE}` or an explicit consumer-supplied absolute target.** Scratch (audit re-renders of spawn prompts, working notes), audit files, figure artifacts, and any other ephemeral output MUST land under `${BASE}` or under a consumer-supplied absolute path — e.g., the design-doc directory delivered to spawned members via `[INSERT abs design-doc directory]`, or the research folder delivered via `[INSERT abs research folder]`. Never `/tmp` unless `${BASE}` itself is `/tmp/claude-code` (which is a legitimate base-dir choice).

2. **`${BASE} == <unset>` is a hard stop, not a fallback.** If `${BASE}` is the literal sentinel `<unset>` (absolute-path argument branch), any code that tries to compute a path from `${BASE}` MUST abort with `Error: BASE is <unset>; refusing to fall back to /tmp`. The loud failure is the safety net for sites that forgot to guard explicitly.

3. **Members never re-resolve BASE.** The Director's spawn-prompt substitution delivers `${BASE}` to each spawned member as a literal absolute path baked into the spawn prompt. Members MUST use that literal path. Members MUST NOT load `Skill(cafleet:base-dir)` to compute their own `${BASE}` — that would invite drift if the Director's anchor changed mid-session.

4. **Missing-BASE-line anchorless status.** If a member's spawn prompt is missing the `BASE:` line entirely (an expected outcome when the Director resolved `${BASE} = <unset>`), the member treats the audit-file feature as disabled and emits a single CAFleet message back to the Director as a parens-free anchorless status (per `skills/design-doc/coordination.md` § *Anchorless Status*):

   ```
   audit-disabled no BASE in spawn prompt
   ```

   The phrasing deliberately omits parentheses so a Director reading the broker log does not misinterpret it as a malformed `<verb> (<pointer>)` hop. The member MUST NOT fall back to `/tmp`.

## The `<unset>` sentinel

`Skill(cafleet:base-dir)` is the only place that may return `BASE=<unset>`. The sentinel is the literal string `"<unset>"` (case-sensitive). Any consumer skill that has `${BASE}` set to `<unset>` MUST:

1. **Guard audit-file writes.** Audit-file writes that derive their path from `${BASE}` are guarded by an explicit `${BASE} != <unset>` check at the call site. The guard is mandatory; reaching `Path(BASE) / …` without the guard is the failure mode item 3 catches. A guarded skip is the intended path when `${BASE}` is `<unset>` — no silent no-op, no fallback to `/tmp`, just an explicit skip whose condition is visible in the source.
2. **Omit `BASE:` from spawn prompts.** The Director MUST NOT spawn members with `BASE: <unset>` in the spawn prompt — the line is omitted entirely so the member's existence-check naturally treats audit-file features as disabled. The literal string `BASE: <unset>` is never written into any spawn prompt.
3. **Loud-error on unguarded BASE-derivation.** If a code path under `${BASE}=<unset>` reaches an unguarded `Path(BASE) / …` computation, abort with the standardized error: `Error: BASE is <unset>; refusing to fall back to /tmp`.

Skip (item 1) and loud-error (item 3) are different stages of the same protocol, not alternative resolutions: every BASE-dependent write site MUST be guarded; the loud error is the safety net for sites that forgot to guard. The historical silent-skip behavior masked bypass bugs and is no longer accepted.
