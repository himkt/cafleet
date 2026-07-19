---
name: clean-docs
description: >
  Aggressively simplify the repository's docs, comments, and rules on an
  affirmative basis, as a CAFleet team of scanner members gated by a reviewer.
  Use when the user asks to clean the docs, simplify docs/comments, run an
  affirmative-writing sweep, de-duplicate documentation, tighten verbose prose,
  remove redundant comments, or fix prohibition-only rule sections. Scanners
  read every file in their slice in full and propose apply-ready rewrites;
  nothing is applied before the reviewer approves the merged findings. Members
  load this skill by its name clean-docs via their backend's skill-loader.
---

# Clean-Docs Skill (CAFleet-orchestrated)

On demand, review the whole tracked tree for prose that can be **simplified**
(redundant restatements, verbose phrasing) or made **affirmative** (prohibition
piles lacking a positive spec, unpaired "never X" rules, silent-fallback code)
per `~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md`,
then apply the reviewer-approved rewrites. After a run, every touched surface
states the desired behavior directly, in fewer words, with every constraint,
contract detail, and live test assertion intact.

This is a **judgment review, not a grep**: each scanner reads every file in its
slice in full and proposes exact replacement text. That is what distinguishes
this skill from its sibling `historical-residue-cleanup`, whose pattern catalog
targets past-tense narration and sentinel tests — route requests about
historical narration there, and requests about simplification/affirmative
rewriting here.

## Three invariants (never violated by any run)

1. **No contract or behavior lost.** Every CLI flag, error string, schema,
   command example, IMPORTANT line, and cross-reference a reader relies on
   survives — in fewer words, never in weaker form. The single exception is an
   approved class-(P4) code fix, which changes behavior deliberately
   (fallback → fail-fast) and is individually justified and test-covered.
2. **No live test coverage lost.** Test logic, assertions, fixtures, and
   parametrizations are untouchable; only their comments and docstrings are in
   scope.
3. **No new narration (R1).** Every rewrite reads as a clean present-tense
   statement of current behavior — no "previously / now / formerly", no "this
   replaces X", no "renamed from Y".

## Required reading

Identify your coding agent first — a member's spawn prompt names it on the
`CODING AGENT:` line; the Director (main session) uses its own identity — then
Read your overlay and **resolve** it before your first action.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`](../../../skills/cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you emit a literal `{monitor_model}` / `{reviewer_model}` / `{skill_loader}`, guess a wrong value, or ignore a backend note |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../../skills/cafleet/reference/base-dir.md) | the task-scope BASE resolution, the no-bypass write protocol, and the `<unset>` contract — you mis-root run artifacts or fall back to `/tmp` |
| 3 | the `cafleet-design-doc` skill's [`reference/coordination.md`](../../../skills/cafleet-design-doc/reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema and the two skill-local extensions (`scanner` role, `findings` pointer) — your status hops mis-route |
| 4 | this skill's [`reference/rubric.md`](reference/rubric.md) | the finding classes, KEEP guardrails, and the apply-ready row format — your proposals are unreviewable or unsafe |

Before acting, resolve every `{token}` you will use to its overlay value (or the
documented default); a literal `{token}` in any command, message, or user-facing
string is a defect.

## Scope

**In scope**: the whole tracked tree minus the exempt set. **Exempt (never
modified)**: `design-docs/`, `researches/`,
`cafleet/src/cafleet/db/alembic/versions/**`, `cafleet/src/cafleet/webui/dist/**`,
and lock files. A migration legitimately references prior state; the generated
`dist/` bundle is not authored prose; the design/research folders are the
historical record.

Per surface, the scope of what may change:

| Surface | In scope | Untouchable |
|---|---|---|
| `docs/`, `README.md`, `SPEC.md`, skills, rules | prose structure and wording | contract detail (exact flags, error strings, schemas, layouts), command examples, IMPORTANT / lossless-rule lines, backend-neutral base/overlay separation |
| source code | comments, docstrings, class-(P4) fallback sites | flags, options, columns, code paths, log/user-facing/error strings |
| tests | comments and docstrings | all logic, assertions, fixtures, names |

## Team shape

```
User → /clean-docs
 └─ Director (main session — resolves BASE, bootstraps fleet, partitions slices,
              merges findings, holds the apply behind review, verifies, commits
              when asked, tears down)
     ├─ monitor    (first-in, --role monitor; heartbeat; gates the first ordinary spawn)
     ├─ scanner-1  (owns a disjoint file slice: full read → propose → apply after gate)
     ├─ scanner-N  (…)
     └─ reviewer   (lost-meaning / lost-contract / behavior-change guard, --model {reviewer_model})
```

| Role | Responsibility |
|---|---|
| **Director** | Resolve task-scoped `${BASE}`; `cafleet fleet create`; spawn the monitor first and gate on `ready: monitor live`; partition the in-scope tree into **disjoint file-ownership** slices (one file → one scanner, whole surfaces per scanner); merge partial findings into the run's canonical `findings.md`; route it to the reviewer and **hold the apply until `approved (findings)`**; relay approval to scanners; apply any edit a scanner's harness denies (verify the staged diff first); run verification; escalate drift observations to the user; tear down monitor-first. |
| **monitor** | The mandatory dedicated monitoring member (canonical conditional idle-nudge). Reuses the `cafleet` skill's `roles/monitor.md`; the Director never runs `cafleet monitor start`. |
| **scanner** (×N) | For its slice: read every file in full, propose apply-ready findings per `reference/rubric.md`, record drift observations separately, write `findings-<slice>.md` under `${BASE}`. After approval is relayed: apply its own slice's rows, re-verify its diff, route harness-denied writes to the Director. |
| **reviewer** | Validate the merged findings **before** any edit: per-row APPROVE / REJECT / REVISE verdicts against the rubric's guardrails and the legitimacy carve-outs. After apply: confirm the git diff stays within the approved rows. |

**Disjoint file-ownership is the concurrency contract.** No two scanners edit the
same file, so parallel apply is safe without worktrees. A useful default
partition: docs+root files / skills+`.claude` / source+admin / tests — merge or
split slices to balance the scanner count against the tree size.

## Coordination

The skill adopts the `cafleet` verb + pointer schema
(`skills/cafleet-design-doc/reference/coordination.md`) with two skill-local
extensions, since a run produces findings files, not a design document:

- **Role taxonomy** gains a `scanner` role for `COMMENT(scanner)` markers.
- **Pointer** `findings` denotes the run's findings as a whole. Scanners report
  `complete (findings)`; the reviewer's sign-off is `approved (findings)` (or
  `blocked (findings)`); per-row routing uses `<file>:<line>` pointers.

## Per-run process

1. **Resolve `${BASE}`** — task-scoped per `reference/base-dir.md`, convention
   `researches/clean-docs-<UTC-compact>/` (gitignored, one folder per run).
2. **Bootstrap** — `cafleet doctor` (gating), `cafleet fleet create`; spawn the
   monitor first (`--role monitor --model {monitor_model}`), gate on
   `ready: monitor live`.
3. **Spawn workers** — scanners (one per slice) and the reviewer
   (`--model {reviewer_model}`), each from a rendered prompt at
   `${BASE}/.prompts/<role>-<UTC-compact>.md`.
4. **Review + propose (fan-out)** — each scanner reads its slice in full and
   writes `${BASE}/findings-<slice>.md`: apply-ready rows per the rubric, plus a
   separate *Observations* section for content drift it noticed but must not fix.
5. **Merge + review (gate)** — the Director merges partials into
   `${BASE}/findings.md` (verdict space per row + observations consolidated) and
   routes it to the reviewer with `ready (findings)`. **No repository edit
   happens before `approved (findings)`.**
6. **Apply** — the Director relays approval; each scanner applies its own
   slice's approved rows exactly as written. A scanner whose harness denies a
   write (commonly `.claude/`) stages the full target file under
   `${BASE}/.apply/` and routes to the Director, who diff-reviews the staged
   file and applies it.
7. **Verify** — `mise //cafleet:test`, `mise //cafleet:lint`,
   `mise //cafleet:typecheck` green; the reviewer confirms the git diff is
   confined to the approved rows.
8. **Report + teardown** — the Director reports the applied set and the
   escalated observations, commits only when the user asks, and tears down
   monitor-first (monitoring member → ordinary members → verify via
   `member list` → `fleet delete`).

## Per-run output artifacts (under `${BASE}`)

- `findings-<slice>.md` per scanner — apply-ready rows (the format is canonical
  in `reference/rubric.md`) + an Observations section.
- `findings.md` — the merged review target with the reviewer's per-row verdicts;
  the run's audit record.
- `.prompts/` and `.apply/` — agent-only scratch (dot-prefixed per base-dir.md).

The **applied cleanup — the git diff plus green verification — is the real
deliverable**; drift observations are escalated to the user, never self-applied.

## Spawn-prompt skeleton

The Director renders each `[INSERT …]` marker to a literal before writing;
leaves the four `{fleet_id}` / `{member_id}` / `{director_member_id}` /
`{coding_agent}` identity placeholders for the CLI's `str.format` at spawn;
doubles every OTHER literal brace as `{{` / `}}`:

```
You are the <role> in a clean-docs team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/<role>.md] with the Read tool BEFORE any other action.

Load these skills at startup:
- the clean-docs skill — for the rubric and per-run process
- the cafleet skill — for the broker primitives and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path the Director resolved]
CODING AGENT: {coding_agent}

<role-specific assignment: the scanner's slice path list + findings file path,
 or the reviewer's merged-findings path>

IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Do NOT edit any repository file until the Director relays the reviewer's approved (findings). Phase 1 is read + propose only.
IMPORTANT: Do NOT commit code or run git write operations — the Director handles all git.

On spawn, as your first Bash call, send the ready signal: cafleet message send --fleet-id {fleet_id} --from-member-id {member_id} --to-member-id {director_member_id} --text "ready"

When you see cafleet message poll output with a message from the Director, act on those instructions.

<start cue: scanner — begin the full read of your slice; reviewer — wait for ready (findings)>
```

The monitor's spawn prompt comes from the `cafleet` skill's `roles/monitor.md`
canonical skeleton, not from this one.

## Skill file layout

```
.claude/skills/clean-docs/
  SKILL.md              # this file — dispatch + orchestration, backend-neutral
  roles/
    scanner.md          # full read → propose → apply own slice after the gate
    reviewer.md         # lost-meaning / lost-contract / behavior-change guard
  reference/
    rubric.md           # finding classes, KEEP guardrails, row format — canonical
```

`SKILL.md` and `roles/*.md` are backend-neutral: `{monitor_model}` /
`{reviewer_model}` / `{skill_loader}` / `{permission_flags}` tokens resolve from
`../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`, and every
spawn prompt's `CODING AGENT: {coding_agent}` line tells the member which
overlay to read.
