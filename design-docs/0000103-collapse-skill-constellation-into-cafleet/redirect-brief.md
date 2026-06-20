# Redirect brief — design 0000103 (supersedes spawn-prompt scope where it conflicts)

The user redirected the scope. **Do NOT keep any separate `cafleet-agent-team-*` skill.** Fold everything into the single `cafleet` skill (`skills/cafleet/`). Three top-level skill directories are DELETED entirely: `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, and `cafleet-base-dir`. `/cafleet` becomes the umbrella skill.

## Target structure under `skills/cafleet/`

- **`SKILL.md`** (broker CLI core, loaded by ALL agents) stays lean. Add a **"Team supervision"** section that:
  - reminds Directors the FIRST member spawned is the dedicated monitoring member (`cafleet member create --role monitor`), which owns the heartbeat and gates ordinary members behind its `ready: monitor live` handshake;
  - points to the load-on-demand reference files below.
  - Do **NOT** put Director governance, the monitoring mechanism, or base-dir content in the SKILL.md body.

- **`reference/supervision.md`** (NEW, Director-only, load-on-demand): the merged governance + monitoring mechanism, absorbing BOTH current team skills:
  - Governance (from `cafleet-agent-team-supervision`): Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, Asynchronous Wait Rule, User Delegation, Stall Response, Cleanup, Quick Reference.
  - Monitoring mechanism (from `cafleet-agent-team-monitoring`): the monitor heartbeat, watched-set per-agent intervals (Director 180 s, members 720 s), how members are woken, Monitor Lifecycle table, the 5-step team-facilitation loop.
  - A single `supervision.md` is preferred (no cross-file ping-pong). Split the monitoring mechanism into its own `reference/` file ONLY if you find it genuinely cleaner — and justify it.

- **`roles/monitor.md`** (NEW, parallel to `roles/director.md` / `roles/member.md`): the monitoring member's authoritative role definition — startup sequence, the on-wake capture→classify→re-engage routine, the two-command constraint (`cafleet member capture` + `cafleet member nudge`), teardown. Use angle-bracket placeholders the monitor substitutes from its spawn-prompt CONTEXT lines, exactly as `roles/member.md` does. The monitoring-member spawn prompt then collapses to the SAME canonical spawn-prompt skeleton ordinary members use — eliminating the "monitoring member is the documented exception" carve-out in `reference/director.md` (the canonical-spawn-prompt-skeleton section, ~line 106).

- **`reference/base-dir.md`** (NEW, load-on-demand): the base-dir resolution + the no-bypass write protocol + the `<unset>` sentinel contract, moved (substance-preserving) from the `cafleet-base-dir` skill.

## Key safeguard (resolves the bloat/layering objection)

All Director-only and base-dir content lives in **load-on-demand reference files**, NOT the `SKILL.md` body — exactly like the existing `reference/director.md`. Ordinary members and standalone agents that load `/cafleet` are not bloated; only Directors and consuming skills read the heavy files.

## Also evaluate: move CLI usage into `reference/cli.md`

The user also asked to consider moving the broker CLI usage out of `SKILL.md` into a reference file, so `SKILL.md` becomes a lean entry point / index. Evaluate this and recommend, honoring this tension:

- The broker **core lifecycle** — `message poll` / `message send` / `message ack` — is the ONE thing every agent (including ordinary members) needs on first load. Moving 100% of CLI usage to a reference file forces a second Read for the universal path.
- **Recommended shape:** keep a lean "core lifecycle" quick-reference (poll / send / ack, plus the Required Flags and Placeholder convention) in `SKILL.md`, and move the fuller command catalog — `message cancel` / `message show` / `agent list` / `agent show` / `doctor` / `agent deregister` / `fleet delete` / Global Options / CLI env vars / the self-registration recipe detail / Message Lifecycle / Error Handling — into a NEW `reference/cli.md` (load-on-demand). Confirm this split or propose a better one in the design doc.

## base-dir caveat to DOCUMENT (do not re-litigate — user decided)

Non-broker consumers (`cafleet-create-figure`, `cafleet-research-report`, `cafleet-research-presentation`, `cafleet-design-doc`) will now load `skills/cafleet/reference/base-dir.md`. Record this as an accepted trade-off in the design doc's Alternatives/Trade-offs section (the cleaner-layering alternative — keeping base-dir standalone — was considered and overridden by the user's consolidation directive).

## Cross-reference updates

Grep for BOTH the old team-skill names (`cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`) AND `cafleet-base-dir` across `skills/` and `docs/`:
- Every skill that loaded the two team skills now loads the `cafleet` skill and reads `reference/supervision.md` (plus `reference/director.md` / `roles/monitor.md` as relevant).
- Every skill that loaded `cafleet-base-dir` now reads `skills/cafleet/reference/base-dir.md`.
- Affected (verify by grep, do not trust this list as exhaustive): `cafleet-design-doc-create` / `-execute` / `-interview`, `cafleet-research-report`, `cafleet-research-presentation` (+ their `roles/director.md`), `.claude/skills/skill-author/SKILL.md`, `.claude/rules/commands.md`, `docs/concepts/monitoring.md`, `docs/how-to/monitor-and-recover.md`, `docs/how-to/mixed-backend-team.md`, `docs/get-started/configure.md`, `README.md`, and `skills/cafleet/SKILL.md` / `reference/director.md` / `reference/recovery.md`.

## Answers to your three questions, reframed

- **Q1 (name):** moot — there is no separate merged skill; everything is under `/cafleet`.
- **Q2 (monitor layout):** `roles/monitor.md` + `reference/supervision.md`. No separate `reference/monitoring.md` unless a split is clearly cleaner (justify it).
- **Q3 (base-dir):** merge into `/cafleet` as `reference/base-dir.md`.

## Confirmed constraints

Docs/skills-only change, NO source code. Follow `.claude/rules/removal.md` (delete all three skill directories, leave no deprecation residue), `affirmative-writing.md` (prescribe desired behavior), `design-doc-numbering.md`. Design number 0000103.

Proceed to draft the design document at the OUTPUT PATH from your spawn prompt.
