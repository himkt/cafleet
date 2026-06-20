# Collapse the CAFleet skill constellation into a single `/cafleet` umbrella skill

**Status**: Approved
**Progress**: 21/32 tasks complete
**Last Updated**: 2026-06-21

## Overview

Collapse the four-skill CAFleet constellation (`cafleet` + `cafleet-agent-team-monitoring` + `cafleet-agent-team-supervision` + `cafleet-base-dir`) into ONE `/cafleet` umbrella skill, deleting the latter three directories with zero residue. Director-only governance, the monitor heartbeat mechanism, the monitor's role definition, base-dir resolution, and the fuller broker CLI catalog all move into load-on-demand files under `skills/cafleet/`, so the universally-loaded `SKILL.md` body shrinks to a lean index. This is a docs/skills-only change — **no source code**.

## Success Criteria

- [ ] `grep -rn "cafleet-agent-team-monitoring\|cafleet-agent-team-supervision\|cafleet-base-dir" skills/ docs/ README.md .claude/ .claude-plugin/` returns nothing (the single source-string mirror at `cli-options.md:986` is the one documented exception, see R5; `site/`, repo-root `prompts/`, and `design-docs/` are excluded by the inclusion list as generated/historical artifacts).
- [ ] `grep -rn "agent-team-supervision\|agent-team-monitoring" skills/ docs/ README.md .claude/ .claude-plugin/` returns nothing (catches the prefix-less path at `.claude/rules/commands.md`).
- [ ] `grep -rn "supervision skill\|monitoring skill\|monitoring/supervision" skills/ docs/ README.md .claude/` returns nothing (catches the prefix-less prose phrasings at `docs/concepts/monitoring.md:35` (#75) and `docs/spec/cli-options.md:843` (#76)).
- [ ] The three skill directories `skills/cafleet-agent-team-monitoring/`, `skills/cafleet-agent-team-supervision/`, `skills/cafleet-base-dir/` are deleted (staged via `git rm -r`).
- [ ] `.claude-plugin/plugin.json`'s `"skills"` array no longer registers the three deleted dirs (array shrinks 12→9).
- [ ] Four new files exist and are non-empty: `skills/cafleet/reference/supervision.md`, `skills/cafleet/roles/monitor.md`, `skills/cafleet/reference/base-dir.md`, `skills/cafleet/reference/cli.md`.
- [ ] `skills/cafleet/SKILL.md` description carries team-management trigger keywords AND a "Team supervision" body section that states the monitor-first gating rule inline plus pointers to `reference/supervision.md` and `roles/monitor.md`.
- [ ] Every former consumer of the deleted skills now points at the correct new file: Director entries → `reference/supervision.md`; former `§ The monitoring member` citers → `roles/monitor.md`; base-dir consumers → `reference/base-dir.md`.
- [ ] No migrated `##` heading remains in the `SKILL.md` body; the body retains only the keeper headings (incl. `Team supervision`).
- [ ] The monitoring-member carve-out parenthetical is gone from `reference/director.md`, and the monitor is documented as the canonical spawn skeleton + a per-role delta.
- [ ] Every relative link in the four new files resolves (depth-3 path rewrites applied; see R9).
- [ ] The literal `<unset>` sentinel is preserved byte-for-byte in `reference/base-dir.md`.
- [ ] `mise //cafleet:lint` passes (no source code touched; lint confirms nothing regressed).

---

## Background

CAFleet ships four skills that are tightly coupled at load time:

| Skill | Role today | Lines | Loaded by |
|---|---|---|---|
| `cafleet` | Broker CLI core | ~189 | EVERY agent (Directors, ordinary members, standalone) |
| `cafleet-agent-team-monitoring` | Heartbeat mechanism + 5-step facilitation loop | 173 | Director role files only, by name |
| `cafleet-agent-team-supervision` | Governance (Core Principle, Auth-Scope Guard, Spawn Protocol, …) | 126 | Director role files only, by name |
| `cafleet-base-dir` | `${BASE}` resolution + no-bypass write protocol + `<unset>` sentinel | 78 | Many skills (broker + non-broker: figures, research) |

Three problems motivate the collapse:

1. **Cross-file ping-pong.** `cafleet-agent-team-supervision` declares `cafleet-agent-team-monitoring` a "hard prerequisite" and its Stall Response is a *stub* that defers to monitoring; monitoring's Dispatch step in turn defers back to supervision's Authorization-Scope Guard. Directors must load both in a fixed order ("monitoring first — it is the foundation layer"). The two are one Director-only concern artificially split across two skills.
2. **The monitoring-member carve-out.** `reference/director.md`'s canonical spawn-prompt skeleton documents the monitoring member as "the exception — its canonical prompt lives in the `cafleet-agent-team-monitoring` skill." Every other role uses the shared skeleton + a per-role delta; only the monitor has a bespoke, fully-inline prompt. There is no `roles/monitor.md` parallel to `roles/director.md` / `roles/member.md`.
3. **Proliferation without payoff.** `reference/director.md` already proves the organizing principle: Director-only content lives in a load-on-demand `reference/` file inside the `cafleet` skill, so ordinary members and standalone agents never load it. The three satellite skills predate that precedent and duplicate the separate-skill overhead (a second permission surface, a load-ordering incantation, a deprecation surface) for no benefit ordinary members can use.

The collapse extends the `reference/director.md` precedent to all three satellites: governance + mechanism merge into one `reference/supervision.md`; the monitor's first-person routine becomes `roles/monitor.md` (the 3rd role anchor); base-dir moves 1:1 to `reference/base-dir.md`; the broker CLI catalog beyond the poll/send/ack trio moves to `reference/cli.md`. The body becomes a lean index every agent loads.

### Discovery: how an agent finds this guidance after the collapse

There are two paths an agent reaches team-supervision guidance by, and the collapse affects them differently:

- **Orchestrated path (dominant, and it improves).** Every CAFleet-native team skill (`cafleet-design-doc-create`/`-execute`/`-interview`, `cafleet-research-report`, `cafleet-research-presentation`) loads the doomed skills *by name* in its Director role-file startup block — never via description auto-trigger. Post-collapse, the Director loads `/cafleet` (already named in those blocks) and Reads `reference/supervision.md`, reinforced by the new `SKILL.md` "Team supervision" section. Strictly more reliable; nothing lost.
- **Ad-hoc path (the genuine regression to mitigate).** An operator who says "spawn a cafleet team" *without* entering a team skill relies on the deleted skills' **descriptions** auto-firing. After deletion those triggers vanish, and the current `/cafleet` description is messaging-only (zero supervision language). The mitigation is the single highest-leverage edit in this design: **rewrite the `/cafleet` `SKILL.md` description** to carry team-management trigger keywords, and keep the body "Team supervision" section unmissable with the one non-negotiable rule (spawn the monitoring member first) stated inline.

---

## Specification

### Target file layout under `skills/cafleet/`

| File | Action | Audience | Notes |
|---|---|---|---|
| `SKILL.md` | Modified | Universal | Lean index + broadened description + new "Team supervision" section |
| `reference/supervision.md` | **New** | Director-only | Governance + heartbeat mechanism merged (absorbs both deleted team skills); no frontmatter |
| `roles/monitor.md` | **New** | Monitor member | 3rd role anchor beside `director.md`/`member.md`; placeholder-based |
| `reference/base-dir.md` | **New** | Consumers | 1:1 move from `cafleet-base-dir`; drop frontmatter |
| `reference/cli.md` | **New** | On-demand | Fuller broker CLI catalog (everything beyond poll/send/ack) |
| `reference/director.md` | Modified | Director-only | Remove carve-out; repoint `L31`×2 / `L106` / `L136` + base-dir refs `L122`/`L158`/`L163` |
| `reference/recovery.md` | Modified | Director-only | Repoint `L7` to `./supervision.md`; keep Shutdown Protocol as canonical |
| `.claude-plugin/plugin.json` | Modified | Plugin manifest | Remove the 3 deleted-dir entries from the `"skills"` array (12→9); skill manifest, not runtime source |
| `skills/cafleet-agent-team-monitoring/` | **Delete** | — | Content fully migrated |
| `skills/cafleet-agent-team-supervision/` | **Delete** | — | Content fully migrated |
| `skills/cafleet-base-dir/` | **Delete** | — | Content fully migrated |

### `SKILL.md` body boundary (what stays universal)

The body keeps ONLY content that is **(1) universal AND (2) core-lifecycle or load-index**. The test: *if an ordinary member or standalone agent never needs it on a normal turn, it is not in the body.*

**Keepers (stay in `SKILL.md`):** frontmatter/intro, Reference files index (extended with the 4 new files), Apply-overlay, Required Flags (+ trailing CLI-env-vars line stays as a one-line pointer to `reference/cli.md`), Placeholder convention, Soliciting user reactions, the **poll / send / ack** trio, and the NEW "Team supervision" section (the single intentional non-lifecycle exception — rule + pointers only).

**Movers (out of the body):** Global Options, Coding-agent backends, Self-registration recipe detail (body keeps at most a 1-line pointer), `message cancel`, `message show`, `agent list`/`agent show`, `doctor`, `agent deregister`, `fleet delete`, Typical Workflow bootstrap, Message Lifecycle, Error Handling — all to `reference/cli.md`. (`cafleet member *` already lives in `reference/director.md`.)

The CLI split line is **core-lifecycle vs everything-else**, NOT CLI vs not-CLI: `send`/`poll`/`ack` stay in the body even though they are CLI; `cancel`/`show` move out even though they are adjacent commands.

### New "Team supervision" section in `SKILL.md` (placement: immediately after Required Flags)

Three short paragraphs — the affirmative rule stated inline so even an agent that ignores the pointers gets the one non-negotiable rule:

1. **Inline rule:** When a Director spawns a team, the **FIRST** member created is the dedicated monitoring member (`cafleet member create --role monitor --model {monitor_model}`). It owns the heartbeat and gates every ordinary `member create` behind its `ready: monitor live` handshake. The Director never runs `cafleet monitor start` itself.
2. **Pointer:** For the full governance + heartbeat mechanism (Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, Stall Response, Cleanup, the 5-step facilitation loop, Monitor Lifecycle), Read [`reference/supervision.md`](reference/supervision.md).
3. **Pointer:** For the monitoring member's own role definition (startup, on-wake routine, teardown), Read [`roles/monitor.md`](roles/monitor.md).

### `SKILL.md` description rewrite (R2 — user-approved)

The current description is messaging-only and is the *only* governance pull for the ad-hoc Director path. The user approved this rewrite verbatim on 2026-06-21 (it leads broker-core, appends one team-management clause, and carries the trigger keywords the deleted descriptions carried without over-broadening):

> **Interact with the CAFleet message broker and supervise CAFleet agent teams. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents; or when a Director is about to spawn, monitor, health-check, or recover a stalled team of CAFleet members (any `cafleet member create`), which requires the dedicated monitoring member, the heartbeat, and the supervision governance.**

This is the single highest-leverage, must-not-skip edit (R1/R2). The clause "monitor, health-check, or recover a stalled team" plus "heartbeat" carries the recovery-phase trigger verbs the deleted skills' descriptions had, so the ad-hoc path R1 protects still auto-fires on phrasings like "my cafleet team stalled / health-check my team / recover a stalled team".

### `reference/supervision.md` — merged governance + mechanism (single file)

A single file (no two-file split — see Alternatives B). Content outline, in order, with all paths rewritten to depth-3 (see R9):

| Section | Source | Merge notes |
|---|---|---|
| Intro | both skills' intros | Drop all load-ordering language ("foundation layer", "load before", "hard prerequisite", "load both", "in that order") |
| Overlay note (ONE copy) | dedup of the identical `L10` note in both | Path `./coding-agent/<name>.md` |
| Core Principle | supervision | — |
| Communication Model | supervision + monitoring | **Merge** the Facilitation cue para with the heartbeat description; `tmux-push.md` → `../../../docs/concepts/tmux-push.md` |
| The monitor heartbeat | monitoring `L16-24` | Watched-set: root Director **180 s**, ordinary members **720 s**, each on its own per-agent interval; `[The monitoring member]` anchor → `../roles/monitor.md`; keep concepts URL |
| How ordinary members are woken | monitoring `L26-33` | — |
| The monitoring member (Director-facing who/what) | monitoring `L35-37` | Spawn-first + who/what only; the first-person routine lives in `../roles/monitor.md` |
| Idle Semantics | supervision | — |
| Authorization-Scope Guard (CRITICAL) | supervision | Keep the Real-stop-signals table; Soliciting ref → `../SKILL.md` |
| Spawn Protocol + Asynchronous Wait Rule | supervision | monitor refs → `../roles/monitor.md`; `cli-options` → `../../../docs/spec/cli-options.md`; director ref → `./director.md`; member ref → `../roles/member.md` |
| Team-facilitation 5-step loop | monitoring `L118-127` | poll → ACK → dispatch → health-check → escalate |
| Monitor Lifecycle table | monitoring `L128-138` | Shutdown refs → `./recovery.md` (×2) |
| Stall Response | monitoring `L140-173` + supervision stub `L104-106` | **Collapse** the supervision stub INTO the full monitoring version (one section); overlay → `./coding-agent`; director ref → `./director.md`; convert the "do not duplicate" sentence to an affirmative Quick-Reference pointer |
| User Delegation Protocol | supervision `L87-102` | Keep the MUST-NOT list verbatim; overlay → `./coding-agent`; director → `./director.md` |
| Cleanup Protocol | supervision | → `./recovery.md` |
| Quick Reference (11-row table) | supervision `L114-126` | Repoint every cell; keep `{decision_surface}` / `{monitor_model}` placeholders |

### `roles/monitor.md` — the monitor's role definition (NEW, parallel to `roles/member.md`)

First-person role anchor for the `--role monitor` member. Shape mirrors `roles/member.md`. Content:

- **Intro:** "You are a member spawned with `--role monitor`" — the dedicated monitoring member.
- **Two-command constraint:** the on-wake routine acts through exactly two commands — `cafleet member capture` (read-only inspection) and `cafleet member nudge` (re-engage the idle Director). All member-driving routes back through the Director.
- **Startup (4 steps):** (1) send `ready: monitoring member`; (2) launch `cafleet monitor start` as a background task in this pane (`{bg_run}` per overlay); (3) confirm `cafleet monitor status`; (4) only after status shows running, send `ready: monitor live` — the gate signal for the Director's first ordinary `member create`.
- **On each wake:** read the freshly-due agents the wake nudge names (`<role> <id> (<name>)`); capture each named due agent + always the Director (read-only); classify ACTIVE vs IDLE / progressing vs stalled; `cafleet member nudge` the Director when it is IDLE with un-acked inbox / stalled members OR any named due agent looks stalled; otherwise do nothing and end the turn.
- **The wake nudge you consume:** the literal `[monitor] wake: N agent(s) due — …` example.
- **Teardown:** when the Director messages you to wrap up, stop the `monitor start` background task (`{bg_stop}` per overlay), confirm, return to the prompt; the Director then runs `member delete` on you.
- **Canonical spawn prompt:** the SAME `../reference/director.md` skeleton ordinary members use, plus a per-role delta — **omit `--coding-agent`** (so the monitor inherits the Director's backend and the `{coding_agent}` placeholder is substituted by `member create`), pass `--role monitor --model {monitor_model}`.
- **Overlay path (from `roles/`):** `../reference/coding-agent/<name>.md`. Verify placeholder names (`{monitor_model}`, `{coding_agent}`, `{permission_flags}`, `{bg_run}`, `{bg_stop}`) match the overlay table keys 1:1 (R12).

Use angle-bracket placeholders (`<fleet-id>`, `<my-agent-id>`, `<director-agent-id>`) the monitor substitutes from its spawn-prompt CONTEXT lines, exactly as `roles/member.md` does.

### `reference/base-dir.md` — base-dir resolution (NEW, 1:1 move)

Move every section from `cafleet-base-dir/SKILL.md` substance-preserving:

- Drop the frontmatter and the "Do NOT invoke directly" line; rewrite the lead to "Read this file for the base-directory resolution procedure and the no-bypass write protocol."
- Preserve the `Consumer contract` labeled paragraph (a bold inline label at `cafleet-base-dir/SKILL.md:31`, not a heading) and the two `##` headings `No-bypass write protocol` (L58) and `The <unset> sentinel` (L76), so the prose `§` pointers consumers use (e.g. `cafleet-design-doc-create/SKILL.md:57`) still resolve by text — do NOT add a spurious `Consumer contract` heading.
- **Preserve the literal `<unset>` byte-for-byte.**
- Keep the `coordination.md § Anchorless Status` cross-ref (rewrite the relative path for the new depth).
- The file is CLI-neutral (uses only `git rev-parse` + `AskUserQuestion`, zero `cafleet` CLI), which is what makes it safe to host under `reference/` despite non-broker consumers (R6).

### `reference/cli.md` — the fuller broker CLI catalog (NEW)

Migrate the body movers, with depth-3 path rewrites:

- Intro → `../SKILL.md`; CLI env vars (output-flags → `./output-flags.md`, cli-options → `../../../docs/spec/cli-options.md`).
- Global Options; Coding-agent backends — **fix the dangling reference**: `(and the cafleet-agent-team-monitoring skill for the monitor)` → `../roles/monitor.md` + `../reference/supervision.md`; overlay → `./coding-agent`.
- Self-registration recipe; Cancel; Show; List Agents; Doctor; Deregister; Fleet Delete (→ `./recovery.md`); Typical Workflow (→ `./director.md` / `./recovery.md`); Message Lifecycle (→ `./broadcast.md`); Error Handling.

### `reference/director.md` edits

| Location | Edit |
|---|---|
| `L31` (A) | `(… see the cafleet-agent-team-monitoring skill)` → `../reference/supervision.md` |
| `L31` (B) | `… § The monitoring member for the canonical prompt and first-in/first-out lifecycle` → `../roles/monitor.md` |
| `L106` | **Delete** the carve-out parenthetical ("The dedicated monitoring member is the exception …") |
| `L136` | **Keep** the `{coding_agent}`-placeholder behavior but **reframe** it as a per-role delta (the monitor is the skeleton + delta, not an exception); repoint to `../roles/monitor.md` |
| `L122` / `L158` / `L163` | base-dir refs → `./base-dir.md` |

### `reference/recovery.md` edits

`L7` `see the cafleet-agent-team-monitoring skill § Stall Response` → `./supervision.md § Stall Response`. The Shutdown Protocol (`L43-52`) **stays** as the canonical full teardown that `supervision.md`'s Monitor Lifecycle and Cleanup point TO (no duplication — R10). `L20` (→ `./exec-routing.md`) is unchanged.

### Relative-path depth rewrites (R9)

The deleted skills lived at `skills/<skill>/SKILL.md` (depth 2). Content moving into `skills/cafleet/reference/` or `skills/cafleet/roles/` is depth 3. Apply:

| Link target | From `reference/*.md` | From `roles/*.md` |
|---|---|---|
| `docs/…` | `../../../docs/…` | `../../../docs/…` |
| Sibling `reference/` file | `./x.md` | `../reference/x.md` |
| `roles/` file | `../roles/x.md` | `./x.md` |
| `SKILL.md` | `../SKILL.md` | `../SKILL.md` |
| Overlay `coding-agent/<name>.md` | `./coding-agent/<name>.md` | `../reference/coding-agent/<name>.md` |

Grep-assert every link in the four new files resolves before completing.

### Cross-reference blast radius (80 enumerated; 79 net edits)

All sites verified live against the working tree (including `.claude-plugin/plugin.json` and `SKILL.md:61`). Grouped by category; de-duplicated entries reuse an earlier edit pattern. Ids run #1–#80; the eight section parentheticals (5.A–5.H) sum to 79 real edits, and the one remaining id, #27, is a zero-edit de-dup guard in prose after 5.C — so 80 enumerated, 79 net edits.

**5.A — Inside `skills/cafleet/` (5):**

| # | Site | Edit |
|---|---|---|
| 1 | `reference/director.md:31` (A) | → `../reference/supervision.md` |
| 2 | `reference/director.md:31` (B) | → `../roles/monitor.md` |
| 3 | `reference/director.md:106` | DELETE carve-out parenthetical |
| 4 | `reference/recovery.md:7` | → `./supervision.md § Stall Response` |
| 79 | `SKILL.md:61` (Coding-agent backends block) | `(and the cafleet-agent-team-monitoring skill for the monitor)` → moves with the block to `reference/cli.md`; repoint to `../roles/monitor.md` + `../reference/supervision.md` |

**5.B — base-dir startup bullets (11, identical text "the `cafleet-base-dir` skill — for the no-bypass write protocol and BASE-derived path conventions"; fold into the existing `cafleet` startup bullet → "Read `skills/cafleet/reference/base-dir.md`"):** #5 `cafleet-design-doc-create/roles/reviewer.md:8`; #6 `cafleet-design-doc-create/roles/drafter.md:8`; #7 `cafleet-design-doc-execute/roles/programmer.md:8`; #8 `cafleet-design-doc-execute/roles/tester.md:8`; #9 `cafleet-design-doc-execute/roles/verifier.md:8`; #10 `cafleet-research-report/roles/scout.md:8`; #11 `cafleet-research-report/roles/manager.md:8`; #12 `cafleet-research-report/roles/researcher.md:8`; #13 `cafleet-research-presentation/roles/presentation.md:8`; #14 `cafleet-research-presentation/roles/transcript.md:8`; #15 `cafleet-research-presentation/roles/visual-reviewer.md:10`.

**5.C — Director-role supervision/monitoring refs (11, #16-26):**

| # | Site | Edit |
|---|---|---|
| 16 | `cafleet-design-doc-create/roles/director.md:11` | three-skill load+ordering → "Load `cafleet`, then Read `reference/supervision.md`" (drop ordering) |
| 17 | `cafleet-design-doc-create/roles/director.md:21` | supervision § Idle Semantics + monitoring § Stall Response → `reference/supervision.md` (both anchors) |
| 18 | `cafleet-design-doc-create/roles/director.md:63` | "paired cafleet-agent-team-supervision skill" → `reference/supervision.md` |
| 19 | `cafleet-design-doc-execute/roles/director.md:11` | = #16 |
| 20 | `cafleet-design-doc-execute/roles/director.md:32` | = #17 |
| 21 | `cafleet-design-doc-execute/roles/director.md:99` | = #18 |
| 22 | `cafleet-research-report/roles/director.md:7` | "cafleet and cafleet-agent-team-monitoring skills" → "cafleet, then Read `reference/supervision.md`" |
| 23 | `cafleet-research-report/roles/director.md:27` | "per the cafleet-agent-team-monitoring skill" → per `reference/supervision.md` |
| 24 | `cafleet-research-report/roles/director.md:66` | "canonical in the cafleet-agent-team-monitoring skill" → `reference/supervision.md` |
| 25 | `cafleet-research-presentation/roles/director.md:7` | = #22 |
| 26 | `cafleet-research-presentation/roles/director.md:105` | "Follow the cafleet-agent-team-monitoring skill …" → Follow `reference/supervision.md` |

(#27 de-dup guard: `researcher.md:8` is already #12 — zero extra edits.)

**5.D — base-dir SKILL bodies (19, #28-46):**

| # | Site | Edit |
|---|---|---|
| 28 | `cafleet-design-doc-create/SKILL.md:55` | "Load the cafleet-base-dir skill" → "Load `cafleet`, Read `reference/base-dir.md`" |
| 29 | `cafleet-design-doc-create/SKILL.md:57` | § Consumer contract → `reference/base-dir.md § Consumer contract` |
| 30 | `cafleet-design-doc-create/SKILL.md:61` | § The `<unset>` sentinel → `reference/base-dir.md` |
| 31 | `cafleet-design-doc-create/SKILL.md:118` | § No-bypass write protocol → `reference/base-dir.md` (keep the `director.md` pointer) |
| 32-35 | `cafleet-design-doc-execute/SKILL.md:62/66/70/184` | = #28/#29/#30/#31 |
| 36-39 | `cafleet-design-doc-interview/SKILL.md:70/72/76/124` | = #28/#29/#30/#31 |
| 40 | `cafleet-research-report/SKILL.md:54` | = #28 |
| 41 | `cafleet-research-report/SKILL.md:123` | § No-bypass → `reference/base-dir.md` |
| 42 | `cafleet-research-presentation/SKILL.md:66` | = #28 |
| 43 | `cafleet-research-presentation/SKILL.md:75` | = #30 |
| 44 | `cafleet-research-presentation/SKILL.md:118` | = #41 |
| 45 | `cafleet-create-figure/SKILL.md:25` | "Load the cafleet-base-dir skill and follow its procedure" → "Load `cafleet`, Read `reference/base-dir.md` and follow its procedure" (R6 — strongest layering concern) |
| 46 | `cafleet-design-doc/guidelines.md:32` | "cafleet-base-dir skill integration" → `reference/base-dir.md` integration |

**5.E — supervision/monitoring SKILL bodies (17, #47-63):**

| # | Site | Edit |
|---|---|---|
| 47 | `cafleet-design-doc-create/SKILL.md:78` | three-skill load → "cafleet, Read `reference/supervision.md`" (drop ordering) |
| 48 | `cafleet-design-doc-create/SKILL.md:107` | "§ The monitoring member … and … supervision skill" → `roles/monitor.md` and `reference/supervision.md` |
| 49-50 | `cafleet-design-doc-execute/SKILL.md:141/158` | = #47/#48 |
| 51 | `cafleet-design-doc-execute/SKILL.md:160` | "canonical cafleet-agent-team-monitoring prompt/routine" ×2 → canonical `roles/monitor.md` prompt/routine (**preserve the execute-only unconditional-nudge delta** — R14) |
| 52 | `cafleet-design-doc-execute/SKILL.md:405` | "the cafleet-agent-team-monitoring skill runs" → the monitoring loop runs (per `reference/supervision.md`) |
| 53 | `cafleet-design-doc-execute/SKILL.md:420` | "unchanged from the cafleet-agent-team-monitoring skill" → unchanged from `reference/supervision.md` |
| 54 | `cafleet-design-doc-interview/SKILL.md:106` | "load both … monitoring … and … supervision (in that order)" → Read `reference/supervision.md` (drop order) |
| 55 | `cafleet-design-doc-interview/SKILL.md:118` | § The monitoring member → `roles/monitor.md` |
| 56 | `cafleet-research-report/SKILL.md:26` | "loads cafleet and cafleet-agent-team-monitoring and embeds them" → "loads cafleet (reading `reference/supervision.md`) and embeds it" |
| 57 | `cafleet-research-report/SKILL.md:79` | "cafleet and cafleet-agent-team-monitoring skills" → "cafleet, Read `reference/supervision.md`" |
| 58 | `cafleet-research-report/SKILL.md:91` | § The monitoring member → `roles/monitor.md` |
| 59 | `cafleet-research-report/SKILL.md:99` | "apply per the cafleet-agent-team-monitoring skill" → per `reference/supervision.md` |
| 60 | `cafleet-research-report/SKILL.md:103` | "2-stage health-check from the cafleet-agent-team-monitoring skill" → from `reference/supervision.md` |
| 61-63 | `cafleet-research-presentation/SKILL.md:22/81/108` | = #56/#57/#58 |

**5.F — `.claude/` (7, #64-70):**

| # | Site | Edit |
|---|---|---|
| 64 | `.claude/rules/commands.md:40` | "skills/agent-team-supervision/SKILL.md § Authorization-Scope Guard" → `skills/cafleet/reference/supervision.md § Authorization-Scope Guard` (prefix-less path — grep-evading, R11) |
| 65 | `.claude/skills/skill-author/SKILL.md:10` | "no cafleet-base-dir skill cross-reference" → "no `cafleet` `reference/base-dir.md` cross-reference" (keep the self-contained claim) |
| 66 | `.claude/skills/skill-author/SKILL.md:58` | "cafleet-base-dir skill's task-scope resolution procedure" → `cafleet` `reference/base-dir.md` task-scope |
| 67 | `.claude/skills/skill-author/SKILL.md:95` | "cafleet-agent-team-monitoring skill documents the monitoring member's canonical spawn prompt …" → `cafleet` `roles/monitor.md` (mechanism in `reference/supervision.md`) |
| 68 | `.claude/skills/skill-author/SKILL.md:165` | "resolved via the cafleet-base-dir skill" → via `cafleet` `reference/base-dir.md` |
| 69 | `.claude/skills/skill-author/SKILL.md:342` | "cafleet-base-dir skill procedure" → `cafleet` `reference/base-dir.md` procedure |
| 70 | `.claude/skills/skill-author/SKILL.md:480` | "cafleet-base-dir skill's task-scope procedure" → `cafleet` `reference/base-dir.md` task-scope |

**5.G — `docs/` (8, #71-78):**

| # | Site | Edit |
|---|---|---|
| 71 | `docs/how-to/mixed-backend-team.md:30` | "cafleet plus cafleet-agent-team-supervision (which loads cafleet-agent-team-monitoring)" → "cafleet and reads its Director-only `reference/supervision.md`" (no cross-link — two independent homes) |
| 72 | `docs/how-to/monitor-and-recover.md:23` | "canonical spawn prompt lives in the cafleet-agent-team-monitoring skill" → in the `/cafleet` skill's `roles/monitor.md` |
| 73 | `docs/how-to/monitor-and-recover.md:55` | "plus cafleet-agent-team-monitoring / cafleet-agent-team-supervision" → "and reads its Director-only `reference/supervision.md` (recovery ladder and idle semantics, plus the monitoring mechanism)" |
| 74 | `docs/concepts/monitoring.md:30` | "defined by the cafleet-agent-team-supervision skill (the what)" → defined in the `/cafleet` skill's `reference/supervision.md` |
| 75 | `docs/concepts/monitoring.md:35` | table "the Director, per the supervision skill" → per `/cafleet` `reference/supervision.md` |
| 76 | `docs/spec/cli-options.md:843` | "the monitoring/supervision skills point to" → the `/cafleet` skill's `roles/monitor.md` and `reference/supervision.md` point to |
| 77 | `docs/get-started/configure.md:23-25` | **DELETE** all three `Skill(cafleet:cafleet-agent-team-monitoring)` / `-supervision` / `-base-dir` allowlist lines — `Skill(cafleet:cafleet)` at `L22` already covers them; no residue comment (R13) |
| 78 | `docs/spec/cli-options.md:986` | "see the `cafleet-base-dir` skill" — **CONFLICT**, mirrors source string `_prompt.py:35`; see R5 (user-approved: leave verbatim this cycle) |

**5.H — Plugin manifest (1):**

| # | Site | Edit |
|---|---|---|
| 80 | `.claude-plugin/plugin.json:13-15` | DELETE the three `"./skills/cafleet-agent-team-monitoring"` / `-supervision` / `-base-dir` array entries (`"skills"` array 12→9); no residue comment |

`README.md`: verified zero deleted-skill references — **no edit**.

### Section-to-destination mapping (nothing lost)

| From | Section | To |
|---|---|---|
| monitoring | frontmatter | DROP |
| monitoring | intro | `supervision.md` intro (drop load-order) |
| monitoring | overlay note `L10` | `supervision.md` (1 copy) |
| monitoring | Placeholder convention `L14` | DROP (duplicate of `cafleet` SKILL) |
| monitoring | The monitor heartbeat `L16-24` | `supervision.md § The monitor heartbeat` |
| monitoring | How members woken `L26-33` | `supervision.md` |
| monitoring | The monitoring member (Director-facing) `L35-37` | `supervision.md` + `SKILL.md` Team-supervision reminder |
| monitoring | Canonical spawn prompt + on-wake routine `L39-117` | `roles/monitor.md` |
| monitoring | Wake-nudge example `L107-116` | `roles/monitor.md` |
| monitoring | 5-step facilitation loop `L118-127` | `supervision.md § Team-facilitation` |
| monitoring | Monitor Lifecycle `L128-138` | `supervision.md` |
| monitoring | Stall Response `L140-173` | `supervision.md` (supervision stub collapses in) |
| supervision | frontmatter / intro `L8` / overlay `L10` | DROP |
| supervision | Core Principle … Quick Reference | `supervision.md` |
| supervision | Stall Response stub `L104-106` | collapsed into `supervision.md § Stall Response` |
| base-dir | every section | `reference/base-dir.md` (anchors + literal `<unset>` preserved) |
| `SKILL.md` body | env vars, Global Options, Coding-agent backends, Self-reg detail, Cancel, Show, List Agents, Doctor, Deregister, Fleet Delete, Typical Workflow, Message Lifecycle, Error Handling | `reference/cli.md` |
| `.claude-plugin/plugin.json` | the 3 deleted-dir `"skills"` entries (L13-15) | REMOVED (manifest no longer registers the deleted dirs; array 12→9) |

### Settled decisions

The user approved R2, R5, R6, and R8 on 2026-06-21; the Send preview-mechanics trim is an accepted low-stakes design decision.

| Ref | Decision | Resolution | Rationale |
|---|---|---|---|
| **R2** | Exact `SKILL.md` description wording | The proposed rewrite above is adopted verbatim | Highest-leverage discovery edit; carries the team-management + stall/health/recover trigger keywords without over-broadening |
| **R5** | `cli-options.md:986` ↔ source `_prompt.py:35` desync | Both strings stay verbatim this cycle; the follow-up is recorded as a Changelog row; source is not edited | Docs/skills-only scope; editing only the doc would desync from the runtime error string; no test asserts it |
| **R6** | `cafleet-create-figure` (standalone, non-broker) depends on `reference/base-dir.md` inside the broker skill | Accepted | `base-dir.md` is CLI-neutral (zero `cafleet` CLI), so safe to host; the `/cafleet` single-umbrella principle wins |
| **R8** | Stale global mirror `~/.claude/skills/{cafleet-base-dir,cafleet-agent-team-*}` | The Step 17 post-merge action re-syncs/removes the global mirror; recorded in the Changelog | Physical copies still auto-trigger globally; the post-merge action clears them |
| (opt) | Send preview-mechanics prose | Trimmed to a one-liner in the body | Keeps the universally-loaded body lean |

### Risk register

| ID | Severity | Risk | Mitigation |
|---|---|---|---|
| R1 | High | Ad-hoc Director discovery regresses — deleted descriptions were its only governance pull | Description rewrite (R2) + inline gating rule in the body Team-supervision section |
| R2 | High | Description rewrite is load-bearing yet must not over-broaden | Lead broker-core, append ONE team clause; verify it still reads right for a plain member; user-approved (2026-06-21) |
| R3 | High | Monitor is NOT trivially the plain skeleton | Preserve the real per-role delta (`{coding_agent}` placeholder, omit `--coding-agent`, `ready: monitor live` gate, two-command constraint, Teardown) atop the unified skeleton |
| R4 | High | `director.md` has MORE residue than first mapped | Verified TWO refs on `L31` (not just `L106`/`L136`); repoint all; add a `director.md`-specific grep to the checklist; treat the map as a floor |
| R5 | High | Source-string conflict (`_prompt.py:35` ↔ `cli-options.md:986`) | User-approved: leave both verbatim; follow-up recorded as a Changelog row |
| R6 | Medium | base-dir layering — non-broker consumers depend on a file in the broker skill | `base-dir.md` is CLI-neutral (verified); word non-broker consumers as "Read `reference/base-dir.md`", not "load the cafleet skill"; accepted trade-off |
| R7 | Medium | Member bloat | Keep supervision/cli/monitor strictly load-on-demand; the body net SHRINKS for members; assert no member role file gains a Read of those refs |
| R8 | Medium | Global `~/.claude/skills` mirror orphan — stale copies auto-trigger | Step 17 post-merge action removes/re-syncs the three global mirror dirs; recorded in this doc's Changelog (do not rely on the mirror) |
| R9 | Medium | Relative-path depth 2→3 move | Apply the path table; grep-assert every link resolves |
| R10 | Medium | Duplicate wake-mechanism / Stall Response / Shutdown overlap | Merge not concatenate; `recovery.md` owns full Shutdown, `supervision.md` points to it; convert "do not duplicate" to an affirmative pointer |
| R11 | Medium | `commands.md:40` prefix-less path evades a `cafleet-` grep | Repoint + add the broadened grep pattern to verification |
| R12 | Low | `roles/monitor.md` placeholder names must match overlay keys 1:1 | Verify `{monitor_model}`/`{coding_agent}`/`{permission_flags}`/`{bg_run}`/`{bg_stop}` against the overlay table |
| R13 | Low | `configure.md` allowlist residue | Delete `L23-25`; `Skill(cafleet:cafleet)` already covers; no residue comment |
| R14 | Medium | execute's extended (unconditional-nudge) monitor routine | Author `roles/monitor.md` delta-friendly so execute expresses it as a per-role delta; preserve on repoint |

### Alternatives & accepted trade-offs

| Option | Decision | Rationale |
|---|---|---|
| Collapse to `/cafleet` load-on-demand refs | **CHOSEN** | Extends the proven `reference/director.md` precedent; removes the three-skill load-ordering incantation + a 2nd permission/deprecation surface; the cost (Read a ref vs invoke a Skill) is already paid for `director.md` |
| Keep a merged top-level supervision skill | Rejected | Perpetuates a separate-skill surface ordinary members can't use |
| One `supervision.md` | **CHOSEN** | Kills cross-file ping-pong; governance + mechanism interleaved; identical Director-only audience; the only separable content (the monitor's first-person routine) is already extracted to `roles/monitor.md` |
| Two-file supervision split | Rejected | Re-introduces the ping-pong it set out to remove |
| `reference/cli.md` | **CHOSEN** | Body is loaded by ALL agents; line is core-lifecycle vs everything-else; bootstrap is once-per-fleet |
| Keep full CLI catalog in body | Rejected | Re-bloats members |
| `base-dir.md` under `/cafleet` | **CHOSEN (user overrode the standalone recommendation)** | The standalone argument is a genuine layering concern (base-dir is CLI-neutral; several consumers are non-broker), but `/cafleet` single-umbrella is the organizing principle and base-dir was always a cafleet-family concern; CLI-neutrality makes it safe under `reference/`; accepted price: non-broker consumers + create-figure reference a file inside the broker skill's tree |
| `roles/monitor.md` parallel anchor | **CHOSEN** | The role/reference split precedent; the monitor IS a `--role monitor` member, so its anchor belongs beside `member.md`; this seam is what lets the `director.md:106` carve-out retire (monitor = skeleton + per-role delta) |
| Inline the monitor in `supervision.md` | Rejected | Conflates two audiences and perpetuates the exception |

**Clean seam rule:** Director obligations (spawn-first, gate, teardown ordering) = governance in `supervision.md`; the monitor's own actions = role in `roles/monitor.md`; the wake nudge sits on the seam (generated = mechanism in `supervision.md`, consumed = role in `roles/monitor.md`).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-06-21T14:30 -->`
> Documentation/skills ARE the deliverable here (no source code), so the steps are ordered create-new → repoint-consumers → delete-old → verify. Create the new files before repointing so every new link resolves at edit time.

### Step 1: Create `reference/base-dir.md` (no inbound dependencies)

- [x] Move every section from `cafleet-base-dir/SKILL.md` substance-preserving; drop frontmatter + the "Do NOT invoke directly" line; rewrite the lead to "Read this file …" <!-- completed: 2026-06-21T07:36 -->
- [x] Preserve the `Consumer contract` labeled paragraph (a bold inline label, not a heading) and the two real `##` headings `No-bypass write protocol` / `The <unset> sentinel` — do NOT add a `## Consumer contract` heading; preserve the literal `<unset>` byte-for-byte; rewrite the `coordination.md § Anchorless Status` relative path for the new depth <!-- completed: 2026-06-21T07:36 -->

### Step 2: Create `reference/cli.md`

- [x] Migrate the body movers (env vars, Global Options, Coding-agent backends, Self-registration recipe, Cancel, Show, List Agents, Doctor, Deregister, Fleet Delete, Typical Workflow, Message Lifecycle, Error Handling) with depth-3 path rewrites <!-- completed: 2026-06-21T07:45 -->
- [x] Fix the dangling monitor pointer in Coding-agent backends → `../roles/monitor.md` + `../reference/supervision.md` <!-- completed: 2026-06-21T07:45 -->

### Step 3: Create `roles/monitor.md`

- [x] Author the monitor role (intro, two-command constraint, 4-step Startup with the `ready: monitor live` gate, on-wake capture-classify-reengage routine, wake-nudge example, Teardown) using angle-bracket placeholders, shape mirroring `roles/member.md` <!-- completed: 2026-06-21T07:54 -->
- [x] Document the canonical spawn prompt as the `../reference/director.md` skeleton + a per-role delta (omit `--coding-agent`, carry `{coding_agent}`, `--role monitor --model {monitor_model}`); overlay path `../reference/coding-agent/<name>.md`; verify placeholder names match the overlay keys 1:1 (R12) <!-- completed: 2026-06-21T07:54 -->

### Step 4: Create `reference/supervision.md` (merge both deleted team skills)

- [x] Merge governance + heartbeat mechanism per the content outline; collapse the supervision Stall Response stub into the full version; dedup the overlay note to one copy; drop ALL load-ordering language <!-- completed: 2026-06-21T08:05 -->
- [x] Merge the wake mechanism (not concatenate); point Monitor Lifecycle/Cleanup to `./recovery.md` for the canonical Shutdown (R10); convert every "do not duplicate" to an affirmative pointer <!-- completed: 2026-06-21T08:05 -->
- [x] Apply all depth-3 path rewrites and repoint anchors (`../roles/monitor.md`, `../roles/member.md`, `./director.md`, `../SKILL.md`, `../../../docs/…`) <!-- completed: 2026-06-21T08:05 -->

### Step 5: Edit `SKILL.md`

- [x] Broaden the description (R2 default wording) <!-- completed: 2026-06-21T08:14 -->
- [x] Extend the Reference files index with the 4 new files; remove the migrated sections; keep the keepers <!-- completed: 2026-06-21T08:14 -->
- [x] Add the "Team supervision" section (inline monitor-first gating rule + pointers to `reference/supervision.md` and `roles/monitor.md`) immediately after Required Flags; trim the Send preview-mechanics prose to a one-liner <!-- completed: 2026-06-21T08:14 -->

### Step 6: Edit `reference/director.md`

- [x] Delete the `L106` carve-out parenthetical; reframe `L136` as a per-role delta (keep `{coding_agent}` behavior) → `../roles/monitor.md` <!-- completed: 2026-06-21T08:20 -->
- [x] Repoint `L31` (A)→`../reference/supervision.md` and (B)→`../roles/monitor.md`; base-dir refs `L122`/`L158`/`L163`→`./base-dir.md` <!-- completed: 2026-06-21T08:20 -->
- [x] Run a `director.md`-specific grep to confirm zero deleted-skill refs remain (R4) <!-- completed: 2026-06-21T08:20 -->

### Step 7: Edit `reference/recovery.md`

- [x] `L7`→`./supervision.md § Stall Response`; confirm the Shutdown Protocol stays as the canonical full teardown `supervision.md` points to <!-- completed: 2026-06-21T08:24 -->

### Step 8: Rewrite broker-family Director role + SKILL lines (#16-21, #47-55)

- [x] `cafleet-design-doc-create`/`-execute`/`-interview` Director role files + SKILL bodies: three-skill loads → "Load `cafleet`, Read `reference/supervision.md`" (drop ordering); `§ The monitoring member`→`roles/monitor.md`; preserve execute's unconditional-nudge delta (R14) <!-- completed: 2026-06-21T08:34 -->

### Step 9: Rewrite research-family lines (#22-26, #56-63)

- [x] `cafleet-research-report` + `cafleet-research-presentation` Director role files + SKILL bodies: monitoring refs → `reference/supervision.md`; `§ The monitoring member`→`roles/monitor.md` <!-- completed: 2026-06-21T08:40 -->

### Step 10: Rewrite the 11 member base-dir startup bullets (#5-15)

- [x] Fold each "the `cafleet-base-dir` skill — …" bullet into the existing `cafleet` startup bullet → "Read `skills/cafleet/reference/base-dir.md`" across the 11 role files <!-- completed: 2026-06-21T08:48 -->

### Step 11: Rewrite base-dir SKILL bodies (#28-46)

- [x] Repoint every base-dir SKILL-body reference (create/execute/interview/research-report/research-presentation SKILLs, `cafleet-create-figure/SKILL.md:25` (R6), `cafleet-design-doc/guidelines.md:32`) → `reference/base-dir.md` (preserve anchor names) <!-- completed: 2026-06-21T08:58 -->

### Step 12: Rewrite `.claude/` references (#64-70, R11)

- [x] `.claude/rules/commands.md:40` (prefix-less path) → `skills/cafleet/reference/supervision.md § Authorization-Scope Guard`; `.claude/skills/skill-author/SKILL.md` lines 10/58/95/165/342/480 → `reference/base-dir.md` / `roles/monitor.md` / `reference/supervision.md` as mapped <!-- completed: 2026-06-21T08:26 (Director — member Edit denied on .claude/ under dontAsk) -->

### Step 13: Rewrite docs prose (#71-76)

- [ ] `docs/how-to/mixed-backend-team.md`, `docs/how-to/monitor-and-recover.md`, `docs/concepts/monitoring.md`, `docs/spec/cli-options.md:843` → new locations; keep the agent-facing and operator-facing homes independent (no cross-link) <!-- completed: -->

### Step 14: Delete `configure.md` allowlist lines (#77, R13)

- [ ] Delete `docs/get-started/configure.md:23-25` (the three deleted-skill `Skill(...)` entries); `Skill(cafleet:cafleet)` already covers; no residue comment <!-- completed: -->

### Step 15: Decide `cli-options.md:986` (#78, R5)

- [ ] Leave the source-string mirror verbatim this cycle; append a row to this design doc's Changelog table flagging the `_prompt.py:35` ↔ `cli-options.md:986` desync for a future source-scoped change; do NOT edit source <!-- completed: -->

### Step 16: Delete the three skill directories and prune the plugin manifest

- [ ] Remove the three deleted-dir entries (`./skills/cafleet-agent-team-monitoring`, `-supervision`, `-base-dir`, lines 13-15) from `.claude-plugin/plugin.json`'s `"skills"` array (12→9 entries) <!-- completed: -->
- [ ] `git rm -r skills/cafleet-agent-team-monitoring skills/cafleet-agent-team-supervision skills/cafleet-base-dir` (LAST, after all repointing AND the manifest prune) <!-- completed: -->

### Step 17: Verification (zero-residue proof)

- [ ] Run the three zero-residue greps over `skills/ docs/ README.md .claude/ .claude-plugin/` (the two skill-name patterns + the prefix-less prose pattern `"supervision skill\|monitoring skill\|monitoring/supervision"`), all returning nothing — sole documented exception `cafleet/src/cafleet/cli/_prompt.py:35` (R5). The inclusion list deliberately omits `site/` (generated docs-site artifact that rebuilds from `docs/`), repo-root `prompts/` (historical spawn audit), and `design-docs/` (this doc) — naive-grep matches there are not residue. Confirm the three dirs are gone (staged deletions) and `plugin.json`'s `"skills"` array no longer lists them (12→9) <!-- completed: -->
- [ ] Confirm the 4 new files exist and are linked from every former consumer; the body has no migrated `##` headings but retains the keepers incl. `Team supervision`; `grep "role monitor\|ready: monitor live" SKILL.md` ≥ 1 each; carve-out gone from `director.md`; no `../cafleet/reference/coding-agent` or `../../docs/…` left in `reference/` or `roles/`; no "foundation layer / load before / hard prerequisite / in that order / load both" residue; allowlist cleaned + `Skill(cafleet:cafleet)` present; every relative link in the 4 new files resolves (R9); literal `<unset>` preserved <!-- completed: -->
- [ ] `mise //cafleet:lint` passes (no source touched; lint confirms nothing regressed) <!-- completed: -->
- [ ] Post-merge deployment action (R8): remove or re-sync the global mirror dirs `~/.claude/skills/{cafleet-agent-team-monitoring,cafleet-agent-team-supervision,cafleet-base-dir}` so the stale copies stop auto-triggering by description; confirm they are gone; record the action in this doc's Changelog <!-- completed: -->

### Step 18: Finalize

- [ ] Rename the design-doc directory to the new slug as part of the finalize commit (the Director executes this; the directory is currently untracked, so the rename is cheap — a plain `mv design-docs/0000103-merge-monitoring-into-supervision design-docs/0000103-collapse-skill-constellation-into-cafleet` while still untracked, or `git mv` once tracked); then grep the repo to confirm no reference to the old slug `0000103-merge-monitoring-into-supervision` remains, and use the new path in the finalize commit <!-- completed: -->
- [ ] Set Status → Complete, update Progress, verify all tasks checked with timestamps <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-21 | Initial draft |
| 2026-06-21 | Reviewer round: directory renamed to `0000103-collapse-skill-constellation-into-cafleet` (Director executes the directory move at finalization); added `.claude-plugin/plugin.json` as blast-radius site #80 (+ Target-layout row, section-map row, Step 16 task, Success-Criteria check); enumerated `SKILL.md:61` as site #79 (blast radius now 80 enumerated / 79 net edits); broadened the zero-residue greps (`.claude-plugin/` scope + a prefix-less prose pattern, with `site/` / repo-root `prompts/` / `design-docs/` excluded as generated/historical); corrected the Background line counts (monitoring 173, supervision 126) and the Quick Reference (11 rows); reworded the base-dir `Consumer contract` note (inline label, not a heading); added a stall/health/recover clause to the R2 description; added the R8 global-mirror post-merge cleanup action (Step 17); fixed the task count to 0/31. |
| 2026-06-21 | R5 follow-up (pending source-scoped change): `cafleet/src/cafleet/cli/_prompt.py:35` emits "see the `cafleet-base-dir` skill", mirrored verbatim at `docs/spec/cli-options.md:986`; both left as-is this docs/skills cycle — a future source change retargets both to `reference/base-dir.md`. |
| 2026-06-21 | R8 deployment note (post-merge): remove or re-sync `~/.claude/skills/{cafleet-agent-team-monitoring,cafleet-agent-team-supervision,cafleet-base-dir}` so the stale global mirror stops auto-triggering by description. |
| 2026-06-21 | Round-2 reviewer: aligned the Step-1 base-dir wording with the corrected Spec note (`Consumer contract` is a labeled paragraph, not a heading — no spurious `## Consumer contract`); added a Finalize task that executes the directory rename (`mv`/`git mv` + old-slug grep + new path in the finalize commit) so the Changelog rename claim is backed by an executable action; task count now 0/32. |
| 2026-06-21 | Finalized: user approved R2 (description wording verbatim), R5, R6, and R8; the sign-off section rewritten as settled user-approved decisions (pending-state language removed throughout); Status → Approved. |
