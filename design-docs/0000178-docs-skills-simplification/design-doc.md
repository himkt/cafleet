# Simplify documentation and skills while preserving behavior

**Status**: Complete
**Progress**: 18 tasks complete; 6 skipped under user scope
**Last Updated**: 2026-09-11

## Overview

Execution scope (2026-09-11): the user said "no need for further verification...". Complete implementation, necessary existing-test reference migrations, ordinary Director source review and phase commits. Preserve prior evidence and report additional test cases/runs, browser checks, live smoke, final audits and independent verification as skipped. Source-byte accounting supplies the final savings report; per-role reading and final model comparisons remain unmeasured. This scope supersedes the original verification gates and acceptance tasks below. The final implementation awaits the user's publication decision.

Consolidate repeated documentation and agent instructions into clearly owned sections, removing twelve source files while retaining complete operator journeys and independently loadable roles. Keep the current hand-authored Markdown, runtime symlinks, binary embedding, and installation pipeline. Verify the resulting instructions through contract review, source and installed-link checks, existing tests, and real create/interview/execute teams.

## Success Criteria

- [ ] All twelve removals in the path map are complete, with every current reference, required read, example, navigation entry, and test locator migrated to its surviving owner.
- [ ] Both shipped skill entrypoints, seven specialized design-workflow member roles, three generic CAFleet roles, four maintenance skill entrypoints, and six clean-docs roles remain usable with their intended authority and startup order.
- [ ] Every changed normative clause has a reviewed disposition: retained, transferred, confirmed duplicate, or one of the explicit accuracy/portability corrections below; all applicable timing, error, approval, and cleanup distinctions survive.
- [ ] The six public specification pages, twelve runtime aliases, self-contained public monitoring procedure, and numbered standalone root SPEC retain their respective reader contracts.
- [ ] Source, installed, and published-site links and fragments resolve; rendered spawn prompts reach the intended role and mandatory prerequisites without relying on a checkout outside the installation.
- [ ] Appropriate existing checks pass against the revised artifact; live workflow coverage follows the execution-scope amendment below, with evidence for completed cases and explicit reasons for skipped cases.
- [ ] Before/after measurements report unique source size, delivered size, and required reading by role; text reduction is demonstrated without treating transferred text or symlink aliases as duplicate authorship.

---

## Background

Eight scans and their challenge/rebuttal rounds covered public documentation, public specifications, shared broker instructions, supervision, design workflows, distribution integrity, hidden maintenance instructions, and root SPEC. The agreed changes remove circular required reads and repeated catalogs while preserving useful separation between an operator guide, an agent role, and an independent reimplementation contract. The full evidence and reading inventories are in this task's `.audit/scan-*.md` reports; `.audit/synthesis.md` records the Director's final synthesis.

The user permits documentation and skill path changes, selected the existing Markdown/symlink pipeline with no generation step, and requires real-team smoke validation. This task delivers the design and its review; source implementation, installation, deployment, commits, and PRs belong to subsequent work. Installed skills govern the current team's operation; the repository at `e08298925e7f687ad22d264661138d10412b09f5` is the audited implementation baseline.

---

## Specification

### Scope and ownership

Apply the structural changes to current `docs/`, `skills/`, root documentation, and `.claude` maintenance instructions, with corresponding navigation, preset-prose, and documentation-test updates. Runtime commands, schema/migration files, permission maps, model data, release machinery, and installer behavior retain their current contracts. Test code may change to verify the new documentation structure and installed content; this work introduces no documentation generator or new public projection of skill files.

Use one complete owner within each audience and delivery contract. Root SPEC remains independently sufficient for its declared reimplementation scope; public/offline references remain sufficient for operation; shipped roles retain mandatory local dependencies; public contributors retain their own understandable style guidance. A link replaces repeated mechanics only when the destination is available to that reader and the consumer retains the required timing and applicability.

| Surface | Retained responsibility | Boundary |
|---|---|---|
| `README.md`, `docs/docs/index.md` | Project entry, short installation route, navigation | Keep the README's four-surface contract and the home hero/video/Markdown rendering |
| Public concepts and how-to pages | Complete user journeys and operator explanations | Exact implementation contracts remain in public spec owners where the guide can link them |
| Six public spec pages | CLI, persistence, messages, backends, and HTTP operational contracts | Existing runtime aliases deliver these same authored bytes offline |
| `SPEC.md` | Complete inline reimplementation specification with numbered sections | Internal links may share definitions within the file; public/source links supplement its contract |
| `skills/cafleet` | Broker usage, role authority, runtime bindings, supervision, and output-root rules | Ordinary members load their own prerequisites; Director-only policy stays outside their startup route |
| `skills/cafleet-design-doc` | Workflow phase transitions, role deltas, document format, coordination | Shared CAFleet mechanics are invoked through required reads at their action boundaries |
| `.claude` maintenance skills/rules | Project maintenance and authoring workflows | These checkout-local consumers remain separate from the shipped skill payload |

### Final file and section map

The following twelve source files are removed. Paths may break externally as authorized; update every current consumer in the same implementation change. Historical designs/research remain the historical record, and removed paths acquire no compatibility or deprecation stub.

| Removed source | Surviving owner and destination | Mandatory migration |
|---|---|---|
| `docs/docs/concepts/fleet-isolation.md` | `docs/docs/concepts/overview.md#fleet-isolation` | Move routing/non-authentication explanation; update Overview and concept navigation |
| `docs/docs/concepts/model-selection.md` | `docs/docs/concepts/coding-agents.md#model-choice` | Keep operator summary and link actual policy/data owners; update concept navigation and prose references |
| `skills/cafleet/reference/cli.md` | Core `SKILL.md#broadcast` and command index; existing runtime CLI/envelope/data-model sections | Retain unique usage before deleting the catalog; retarget all core, role, supervision, and maintenance lookups |
| `skills/cafleet/reference/director.md` | `skills/cafleet/roles/director.md` | Merge unique selection, spawn, audit, replacement, and action policy; rebase outgoing links and replace incoming paths |
| `skills/cafleet/reference/recovery.md` | `skills/cafleet/reference/supervision.md#recovery` and `#shutdown` | Move complete procedures and require those sections before recovery/teardown |
| `skills/cafleet-design-doc/create/roles/director.md` | `skills/cafleet-design-doc/create/create.md` | Move Director accountability, feedback, diagnostics, phase nudges, and finalization ownership |
| `skills/cafleet-design-doc/execute/roles/director.md` | `skills/cafleet-design-doc/execute/execute.md` | Move composition, marker arbitration, commit protocol, diagnostics, and milestones |
| `.claude/skills/clean-docs/affirmative/reference/rubric.md` | `affirmative/affirmative.md#finding-classes` within the same skill | Move P1/P2/P4 classes, tie-breaks, and legitimate prohibitions/defaults; require workflow read from both roles |
| `.claude/skills/clean-docs/simplification/reference/rubric.md` | `simplification/simplification.md#finding-classes` | Move P3/P5 and paragraph-comparison rules; require workflow read from both roles |
| `.claude/skills/clean-docs/residue/reference/rubric.md` | `residue/residue.md#classification` | Move classifications and known-benign/KEEP rules with the absence-test correction below; retarget roles and pattern catalog |
| `.claude/rules/documentation-tables.md` | `.claude/rules/documentation-maintenance.md#table-conventions` | Move formatting, decision/list precedence, ownership, and exact-string exceptions |
| `.claude/rules/user-facing-docs.md` | `.claude/rules/documentation-maintenance.md#audiences-and-paths` | Move conceptual writing and all four legitimate path surfaces; update rule cross-references |

The generic Director role retains `#model-selection` and gains real headings for `#model-name-to-backend-inference`, `#canonical-spawn-prompt-skeleton`, `#spawn-prompt-size-limit`, `#member-create--scratch-and-audit-files`, and `#model-replacement`. Retain the existing action headings `#member-prompt`, `#member-ping-manual-inbox-poll`, and `#answering-a-members-relayed-question` at the new path. Promote bold paragraph targets to headings so both links and section citations have an actual destination.

Create and execute retain their numbered process steps and add an early `Director responsibilities` section; execute also adds `Team composition` and `Commit protocol` at their existing setup/phase boundaries. Each local diagnostic note preserves `--lines 200` and the current workflow-specific disclosure constraint on deleting and re-spawning members. A generic replacement link carries the algorithm without discarding those local constraints.

| Retained source | Section-level restructuring |
|---|---|
| `skills/cafleet/SKILL.md` | Keep core identity, isolation, Send/Poll/ACK and role-aware failure behavior; add concise broadcast usage and on-demand command links |
| `skills/cafleet/reference/base-dir.md` | Put member input/write states first, followed by the sole generic resolver; transfer only design-doc normalization to guidelines |
| `skills/cafleet/reference/coding-agents.md` | Keep subject selectors, backend sections/data/cues, and bound notes; shorten repeated author scaffolding while retaining six required subsections |
| `skills/cafleet/reference/prompt-routing.md` | Retain the shared denied-command protocol; point to new Director actions and member `#command-execution` heading |
| `skills/cafleet/roles/member.md`, `roles/monitor.md` | Keep distinct entrypoints, local read gates, startup and command authority; shorten repeated rationale |
| `skills/cafleet-design-doc/reference/guidelines.md#file-layout` | Own the one design-doc argument normalization algorithm |
| `skills/cafleet-design-doc/reference/coordination.md` | Retain grammar and marker rules; own all three payload exemptions and full-body retrieval requirements |
| All seven specialized design-doc member roles | Keep role-specific scope, start cues, required reads, and independent review contracts |
| `docs/docs/quickstart.md` | Own install/configure/trust and prompt-based entry; link the manual lifecycle at its new owner |
| `docs/docs/how-to/mixed-backend-team.md` | Retitle `Run a fleet`; own one complete baseline at `#manual-lifecycle` with backend variations |
| `docs/docs/how-to/design-doc-development.md` | Use `Design document workflow` consistently; own three prompts and stage-specific outcomes |
| `docs/docs/contributing.md` | Own contributor setup and checkout installation order; retain a concise public style summary |
| `docs/docs/concepts/monitoring.md` | Retain the complete public procedure, cadence anchor, and lifecycle distinctions; shorten local repetition |
| `docs/docs/concepts/storage.md` | Retain complete `#duplicate-monitor-recovery` and retention guidance; link exact schema mechanics |
| `docs/docs/concepts/member-lifecycle.md` | Retain placement/ownership and uncertain-cleanup explanation; link exact CLI/parser contracts |
| All six `docs/docs/spec/*.md` pages | Apply the exact owner layout below without removing pages or aliases |
| `.claude/skills/skill-author/SKILL.md` | Become a concise integration guide with phase-triggered prerequisites and one complete worked example |
| `.claude/skills/clean-docs/SKILL.md` | Own common process and approved application; use the shared spawn frame plus preserved deltas |
| Six clean-docs roles, shared `reference/review-format.md`, residue `reference/patterns.md` | Keep independent roles, judgment review format, and full sweep catalog |
| Model-refresh and update-readme skill entrypoints | Retain names, invocation and ownership; correct verified release explanation only |
| Root `CLAUDE.md`, `CONTRIBUTING.md`, unrelated project rules | Retain paths; only concise accurate entry wording and necessary consumer updates |

The twelve runtime aliases remain at their existing names: `quickstart.md`, `how-to/mixed-backend-team.md`, concepts `coding-agents`, `member-lifecycle`, `monitoring`, `storage`, and all six specification pages. Neither removed public concept has a runtime alias. The expected shipped skill payload becomes 33 paths from 38; the two preset assets retain their paths.

### Required reading and prompt contracts

Each independently loadable role keeps a `Required reading` block with backend resolution as row one. The remaining rows state what is required and when; repeated explanations of hypothetical failures are shortened. Required reads remain ordered prerequisites, including on-trigger reads, rather than optional reference links.

| Reader/action | Mandatory route and timing |
|---|---|
| Ordinary member startup | Authoritative role first, ready handshake, own runtime/bound notes and CAFleet core/member rules, supplied BASE contract, then applicable format/coordination/workflow reads before task work |
| Monitor startup | Its authoritative role and required backend lifecycle read precede its role-specific startup; monitor overrides ordinary-member startup where specified |
| Director setup | Own runtime, generic Director role, BASE where applicable, and supervision before orchestration; workflow coordination/format before its phase actions |
| Director composing a spawn | Selected backend catalog/defaults/capabilities plus Director skeleton, audit and prompt-size sections before render/write/spawn |
| Any denied-command route | `prompt-routing.md` before routing; member first attempts/reconsiders the actual command within its authority |
| Director recovery/teardown | Supervision `Recovery` or `Shutdown` immediately before that procedure, even if earlier supervision sections were read at startup |
| Design-doc path selection | Guidelines `File Layout` normalization before generic BASE resolution; discovery/result mapping remains in the invoking workflow |
| Clean-docs classification/application | Own workflow class section and umbrella mechanics before classification; shared approved-write procedure before application or staging |
| Skill author | BASE before output design, skeleton/audit before spawn design, coordination before grammar, backend convention before backend-sensitive text, recovery/shutdown before those steps |

Choose an explicit tool-portability rule: use an available non-shell text reader for prerequisite files; where shell is the only text reader, prerequisite file-reading commands may precede ready. Ready remains the first operational broker shell command and precedes ordinary task work. Amend the shared frame and role startup wording consistently; the monitor's pre-launch backend read and confirmation sequence remain distinct. A generic JavaScript orchestration tool is not presumed to provide filesystem access.

Load skills through the executing backend's supported loader. Codex/OpenCode absolute-path loading includes the CAFleet core, own backend section, design-doc skill core, and required role references. A spawned workflow member consults its assigned role/references; loading the umbrella does not create a second team for the enclosing user request.

Preserve two-stage prompt rendering, the four CLI identity placeholders, doubled other braces, literal post-spawn IDs, absolute role paths, the role-specific start cue, and every hard constraint. Apart from the explicitly reviewed startup and missing-host-rule corrections in this design, reconstruction must reproduce current IMPORTANT/role-constraint lines verbatim. Keep a line-level exception ledger for those selected corrections rather than weakening the lossless rule generally.

Missing optional host-rule files use equivalent instructions already supplied in the session when available, with the Director recording that source in the spawn context. If an essential host rule is unavailable and its content is unknown, route that concrete missing prerequisite to the Director before the dependent action. Preserve the Bash-hygiene and member-authority obligations; a nonexistent path is not a reason to drop them or fabricate their contents.

Spawn audit files remain pre-substitution inputs under `.prompts/<role>-<UTC-compact>.md`, with collision suffixes and no overwrite. File input avoids embedding the body in the initial CAFleet invocation; the resolved prompt still enters downstream backend/multiplexer argv. Keep role-by-path prompts compact and retain current launch-failure/cleanup behavior without promising an unlimited input path or an unmeasured safe size.

### Shared behavior to preserve

| Contract | Required invariant after consolidation |
|---|---|
| Identity and body input | Positional integer subjects and explicit relationship flags retain their roles; inline/file/stdin XOR, UTF-8/newlines, four-name creation substitution, and literal message bodies remain distinct |
| Broker messaging | Persistence precedes preview; full JSON differs from truncated text; summary versus recipient delivery, recipient ACK, counts and grouping remain explicit |
| Persisted send failure | Preserve the original ID and consume/ACK the existing row; ordinary members report through their permitted route while Director/monitor pane actions remain role-gated |
| Secondary relay failure | Report actual observed failure; preserve any persisted relay ID and avoid recursive duplicate reports or fabricated success |
| Shell isolation | Each one-shot CAFleet process occupies its own shell invocation; leading environment assignments and the backend-owned long-lived monitor mechanism retain their existing exceptions |
| Requested shell dispatch | The successful member-requested `prompt --shell → ping → ACK` sequence is one explicit exception, serialized per request; failure skips success ping and plain prompt receives no ping |
| Bootstrap | Gating doctor result, fleet/Director/monitor bootstrap, actual monitor startup confirmation, and ordinary-member ready remain different gates from placement or registry existence |
| Dispatch | Dispatch an individually ready member when its inputs exist, then end/yield; unrelated readiness adds no barrier and elapsed time supplies no completion signal |
| Director capture | Fresh same-turn target capture precedes non-exempt send/ping; intervening keystrokes invalidate it; working/awaiting-user defer the whole send, unknown triggers diagnosis |
| Director quiet state | Assigned/new work is needed for finished-pane dispatch; unchanged consecutive facilitation-turn evidence is needed for stall re-engagement; actual fired attempts determine escalation |
| Gate exceptions | Reply to a current-turn reply-soliciting message differs from progress-only status; broadcast requires all recipients eligible or separately gated unicasts |
| Monitor quiet state | Use consecutive wake content/hashes, seed after restart, re-arm on change/activity, and send at most one fixed ping per quiet period; Director additionally needs unacked messages |
| Monitor boundaries | Wake processing uses scan/ping/Director reporting, ignores self, and classifies content with target-backend cues rather than native agent status |
| Codex monitor | Monitor alone owns retained execution; bounded initial confirmation and later-turn first liveness poll survive, including early exit/missing handle/unconfirmed termination and confirmed restart |
| Backend selection | Executing backend supplies local tools, selected spawn backend supplies model/default/capabilities, and observed backend supplies pane cues |
| Recovery | Connection/capture uncertainty is not death; confirmed exit and live unresponsiveness follow separate branches, with existing replacement evidence and caps |
| Shutdown | Delete monitor first, then authorized ordinary members, verify root-only registry, delete fleet, and verify closure; fleet deletion alone is not pane teardown |
| BASE | Preserve all task/shared root branches, relative containment rejection, absolute unset branch, HOME/config choice, valid selected temporary base, lazy writes, and member inheritance |
| Disabled audits | Omit BASE when unset, guard audit writes, retain explicit consumer output paths, emit the exact `audit-disabled no BASE in spawn prompt` status, and fail on unguarded sentinel path construction |
| Model policy | Preserve exact cost-mode trigger, explicit overrides, role exceptions, ordered inference, replacement limits, catalog/default membership, freshness and maintenance ownership |

Recovery actions apply to the task's authorized fleet scope. Listing another conversation's fleet supplies diagnostic information, not cleanup authorization. Preserve the specific workflow disclosure constraint alongside generic replacement policy; neither a structural merge nor a capture-only prompt establishes a new approval or keystroke permission.

### Workflow state and coordination

Guidelines `File Layout` owns the transform: strip a trailing `/design-doc.md`; for relative arguments strip a leading `design-docs/` and then prepend it once; absolute arguments use the stripped absolute folder. Generic BASE owns containment and disabled-audit decisions. Create/interview retain their explicit external document targets when audit BASE is unset; execute retains its literal-argument direct/directory handling and separate no-argument, repository-root, one-level Approved discovery with zero/one/many and pagination outcomes.

Keep the six verbs, three pointer forms, literal heading-path separator, paired issue-marker locations, five review tags, one issue per actionable marker, summary/enumeration limits, issue/status separation, and marker-free approval/finalization. Coordination owns these three free-form exceptions: create pre-draft questions/answers, interview Director/Analyzer exchange, and Verifier initial tool inventory. Each producer/consumer retrieves complete JSON, using file transport for long bodies; the Verifier inventory is its first substantive payload after ready.

The fresh-Drafter embargo covers design-document content until clarification answers arrive. BASE-rooted hidden broker-payload artifacts may carry questions without creating a draft; this explicitly resolves the current blanket any-file wording while preserving the substantive gate.

| Workflow | Required transitions and scope |
|---|---|
| Fresh create | Drafter asks before drafting, groups at least three relevant question categories, reuses answered requirements, and receives answers; one focused critical follow-up remains the limit |
| Resume create | Full marker scan, batched fixes, cascading consistency and scoped edits produce `addressed (doc)`, which enters the same review loop as a fresh draft's `complete (doc)` |
| Review-only create | Keep initial Drafter roster but use an explicit wait-for-revision delta; skip fresh clarification/drafting and begin review of the existing document |
| Create review/user loop | Reviewer approval precedes presentation; ordinary-language feedback is translated by the Director into actionable document markers, then revised and reviewed again |
| Create finalization | After Reviewer approval, explicit user approval and a marker-free document, Director sends `ready (doc) — user approved; finalize`; Drafter sets Approved/date/progress consistently, verifies actionable tasks and replies `addressed (doc)` |
| Execute preparation | Preserve branch handling, marker arbitration, FIXME sweep/DONE confirmation, partial task resumption, and checkbox/timestamp/progress updates before subsequent work |
| Execute composition | Code work uses Programmer+Tester; docs/config may use Programmer alone; conditional Verifier and a fresh Reviewer only at Step 5 retain their timing |
| Execute test ownership | Tester owns test regions, including inline Rust tests, in TDD teams; the existing no-Tester composition exception routes test findings to Programmer consistently in both review loops |
| Execute TDD/disputes | Tests and Director review precede implementation; phase commits remain distinct; same-pointer blockers, test-defect arbitration, counter-disputes, and escalation limits survive |
| Verification | Preserve inventory/tool selection, unavailable-tool evidence, E2E outcomes, per-step impl/test/spec classification, re-verification, and the on-demand matrix |
| Execute final review | All tasks, applicable verification, and independently checked Success Criteria precede a newly spawned full-diff Reviewer; retain read-execute checks, marker-only review edits, and uncapped revision loop |
| Execute user decisions | Preserve halt/abort, marker scan, ordinary-language revision and explicit approval; every accepted revision returns through fresh review before completion |
| Execute local/remote | Carry approve-local through finalization regardless of upstream; remote authorization, failed push, push-success/PR-failure, PR reuse and normal `gh pr create --fill` remain distinct |
| Interview | Preserve read-only Analyzer, seven categories, priority, exact question/options/footer shape, two corrective rounds, and complete JSON retrieval |
| Interview persistence | Preserve stable question numbering, answers/round ranges, batches of up to four, covered-heading progress, unanswered resume, early exit and final progress removal; teardown precedes the operator Q&A phase |

The create workflow and Drafter role both define the exact finalization route `ready (doc) — user approved; finalize`, using the existing optional one-line summary. The Director sends it only after recording Reviewer approval and explicit user approval for the current marker-free document; the Drafter recognizes that Director-originated route as the finalization instruction, rechecks marker absence, updates the required metadata and returns `addressed (doc)`. Approval and finalization status remain in broker messages and document metadata, with no status issue-marker added.

Plain `ready (doc)` routes revision or a work nudge. The Drafter processes actionable markers when present; when none are present, it acknowledges the delivery, preserves metadata and ends the turn unless other assigned work remains. A marker-free document or plain ready message alone never triggers finalization. Any unresolved issue encountered on the explicit finalization route uses the existing paired-marker blocker protocol.

The create and execute Reviewer roles stay separate because their criteria and mutation boundaries differ. Commit examples stage only files eligible under higher-priority user/host instructions; this task's design and audit files remain uncommitted. The ordinary-language feedback path is shared through coordination, while each workflow owns the affected target and mandatory re-review.

### Public and runtime specification structure

The manual guide carries a complete monitor prompt, correct backend-resolved installation paths, literal identity/ready lines, monitor-live gate, per-member ready, fresh capture before dispatch, and verified monitor-first shutdown. Keep install/configure/trust anchors in Quickstart; move references to `quickstart#raw-cli-walkthrough` to the guide's `#manual-lifecycle`, including public and installed Coding agents consumers. Use concept order Overview → Coding agents → Member lifecycle → Monitoring → Storage, and consistent task labels.

Public Monitoring retains its self-contained on-wake and lifecycle procedure. Its `#cadence-and-tick-precision` remains the runtime owner for first/restart baselines, heartbeat versus wake intervals, zero scheduled interval, forced coalescing, failed/skipped wake remaining due, successful atomic stamp/clear, and next-tick interval effects. Storage retains all four duplicate-monitor repair steps, including use of the compatible prior binary and restoration of compatible assets before removing surplus registrations.

| Specification owner | Final organization and preservation |
|---|---|
| CLI `#subcommand-summary`, `#environment-variables`, shared inputs | One command index including subject/JSON availability; keep one environment table and local command applicability/defaults |
| CLI `#output-shapes` | Own complete wrappers, key/order/null distinctions and command-specific success outcomes; envelope links these directly |
| CLI `#error-messages` and shared guards | Keep the discoverable exact command-error catalog; shared guard matrices own their errors and catalog links point there |
| CLI `#creation-failure-compensation` | Own cross-boundary failure order, primary/cleanup diagnostics and uncertainty; command ladders retain validation/success order |
| Data model | Own durable fields, three kinds, atomic DB/lock facts, query/state transitions and summary grouping |
| Message envelope | Retain full field projection/examples: status timestamp, omitted text fields, conditional kind/origin, empty-body behavior and summary semantics |
| Multiplexer | Own acquisition/transfer and Attempted/Unknown cleanup, resolver matrix, exact preview/wake/Esc grammar, deadlines and caller-specific delivery outcomes |
| Coding-agent backends | Retain exact argv, validation-before-side-effects, configuration, effort/model and permission prerequisites independently from human Model choice |
| WebUI API | One shared message schema and selection/order/cap table; shared header errors with endpoint-local body/fleet/action validation order |

Preserve CLI failed unicast notification versus intentional skip, broadcast and HTTP outcomes; pending placement versus absent placement; backend-specific capture/submission mechanics; PATCH row existence versus forced-wake liveness; stopped UI controls versus API capability; status-time selection versus creation-time display order; and the 200-delivery/partial-group behavior. Documentation-table typography preserves literal errors, glyphs, JSON fields, and codepoint transforms.

Remove the unsupported promise that the repository's design-doc directory provides published examples; keep the actual workflow output-location explanation. Public documentation need not add a replacement example artifact or depend on hidden maintenance rules. Keep Rspress content root, `/cafleet/` base, dead-link validation, generated LLM output, home theme and deployment behavior.

### Root SPEC and maintenance

Keep SPEC sections 1–11 and their complete inline contracts. Condense §4 to dependency boundaries and an internal sequence index; transfer unique §11 allowances to §1/§6.3/§6.5/§6.7 before reducing §11 to an internal decision index. Keep §10 as signatures/JSON availability and internal destinations with local required/default/XOR applicability intact.

Within SPEC, §6.3 owns shared body/template rules and cross-boundary compensation; §6.2 owns broker transitions and notification outcomes; §6.4 owns common formatting/snapshot transforms; §6.5 keeps common capture-windowing with exact per-backend argv/submission; §6.8 retains endpoint validation/wire projection; §8 retains migration semantics. Preserve each detailed unit in the root scanner's coverage ledger, including timestamp production, transaction/PRAGMA requirements, runner drain/deadline algorithm, Herdr layout/close behavior, exact presets and every error/null/output distinction. A shorter public paragraph never replaces a root-only algorithm.

Inline confirmed missing asset operation/output/symlink-entry details and complete duplicate-monitor recovery using the audited sources. Verify exact inheritance strings before completing their inline table. Keep unsettled migration-literal and architecture claims unchanged under the disposition table below, rather than declaring pre-existing completeness gaps solved by restructuring. Routine `update-readme` remains drift-only with its existing Agent/sonnet/default invocation; this one-time structural change creates no standing permission to simplify SPEC during normal synchronization.

The maintenance integration guide declares its prerequisites accurately, carries one complete worked example, and links mechanics at their phase triggers. Clean-docs keeps propose → merge → per-row review → Director approval relay → approved-slice application → post-apply review. Preserve disjoint whole-file ownership, exact replacement/REVISE text, KEEP/observations handling, all pattern passes and exemptions, and zero unaccounted residue matches. Affirmative alone retains individual P4 invariant/coverage approval, including explicit acceptance of uncovered rows; simplification preserves behavior/voice and its source scope remains comments/docstrings.

All scanners load the common `.apply/` full-file staging protocol before a denied write, and the Director checks the staged diff against approved rows and current authorization. Preserve all six role anchors, four hard clean-docs prompt obligations and start cues. Merge the three documentation rules with their docs-first order, four path exceptions, table threshold on either axis, ordered-rule precedence, two-row/two-sentence limits, exact-quote exception, and verified literal-pipe rendering; keep unrelated rules separate.

Model refresh retains its five-source fail-closed fetch contract, maintainer approval, exact IDs/curation/free-price handling, context/availability checks, and atomic catalog/default/freshness update. Structural edits preserve the existing model data and `2026-09-05` freshness date. Its release explanation becomes embedded-binary build/release → upgrade → setup; checkout skill installation stays a separate contributor operation.

### Accuracy and policy dispositions

These corrections are tracked separately from prose savings. The source evidence is identified in the corresponding audit report; implementation verifies any still-unverified fact before changing its contract.

| Issue | Chosen treatment |
|---|---|
| Secondary CLI v5 default, JSON-error promise, two-kind summaries | Correct to the audited v6 default, text stderr application errors, and director/monitor/member projection at surviving owners |
| Root startup ordering, template consumers, absent glyphs, delete-success claim | Match audited handler-registration → write/flush ordering, both creation consumers, formatter-specific ASCII/EM/EN DASH values, and success versus error exits |
| Wrong root configuration reference/table count and migration-reset summary | Correct internal reference/six-table count; preserve V6 asset-record reset exception and internally consistent examples |
| Fleet invisibility, synchronized capture, broker-versus-keystroke wording | Describe routing without authentication, a batch of individually timed captures, and broker sends with automatic preview |
| Full payload requested through truncated poll | Require JSON retrieval at the actual clarification/Analyzer/Verifier receiver boundary |
| Role-first versus first-Bash; direct-read prohibition; absent host rules | Apply the explicit capability, loader, and source-recording rules above; preserve prerequisite contents and monitor override |
| Fresh-Drafter file embargo, review-only mode, resume/finalize omissions | Apply the document-content embargo and explicit current-intent transitions above |
| Ordinary-member ping, shell exception, captured user prompt | Preserve role-gated actions, the requested-shell success sequence, and defer/no-answer for capture-only evidence |
| Approve-local, plain-language feedback, no-Tester routing, git conflict | Preserve explicit user intent and composition-specific ownership; stage eligible files only and retain required re-review |
| Cross-conversation cleanup and workflow replacement disclosure | Restrict action to authorized scope; retain more-specific disclosure and generic replacement evidence/caps |
| Author guide's visible scratch, old teardown/verb claims, unlimited file transport | Correct to actual BASE/skeleton/shutdown/coordination owners and verified downstream argv behavior |
| Residue blanket removal-test deletion and lockfile exclusion mismatch | Preserve current-behavior absence regressions and protected test logic; make mechanical exclusions cover the stated lockfile set |
| Model-refresh release and table renderer prose | Correct release to current packaging; inspect Rspress output before changing literal-pipe markup or its explanation |
| OpenCode preset explanatory body | Keep permission-map bytes unchanged; describe literal capabilities separately from role/task authorization and update SPEC's exact prose copy together |
| Header/parser/matcher grammar, wake input-validation wording | Preserve accepted-input/permission contracts; defer speculative rewrites until targeted evidence establishes exact current behavior |
| Root architecture/session ownership and migration literal/removal scope | Retain current constraints and live migration semantics unchanged in this pass; treat any relaxation or new historical literal insertion as a separate scoped decision |
| Execute direct-path/resume status eligibility | Preserve current direct-path and discovery branches without adding a new eligibility rule; a later explicit workflow decision may reconcile them |

Deferred topics remain isolated from the structural work and receive no savings credit. The implementation report lists any corresponding existing inconsistency that remains; it does not hide it under a global claim of runtime conformance. Runtime fixes, permission-map changes, model refresh and migration rewrites are outside this design.

### Validation and evidence

Maintain a task-local hidden migration ledger before editing: old file/section/line, normative clause, final owner/anchor, mandatory consumer/timing, correction or transfer classification, and verification evidence. Include ordinary links, inline-code/prose paths, section-name citations, fenced skeletons, hard lines, navigation and test slices. Rebase moved Director links from `reference/` to `roles/` rather than performing an indiscriminate basename replacement.

Extend the current documentation tests where needed; retain behavioral assertions as well as graph validation. Source/installed equality proves delivery fidelity, while the independently reviewed owner map proves retained obligations. A removed role disappearing from a file enumeration is insufficient preservation evidence.

| Existing check/consumer | Required migration |
|---|---|
| `shared_skill_pages_make_the_monitor_member_the_execution_owner` | Replace removed CLI catalog locators with core/Director owners and retain monitor ownership predicates |
| `the_director_and_member_roles_keep_the_ping_protocol`, fixed-ping surface checks | Use the generic Director role and supervision recovery section; retain actor boundaries and shell follow-up |
| Backend lifecycle and monitor-role lifecycle checks | Keep detailed lifecycle assertions at backend owner and mandatory launch/confirmation/later-turn gates at monitor role |
| Public monitoring concept checks | Retain public procedure coverage under the selected self-contained plan |
| Row-one required-reading/backend shape/default/vocabulary checks | Keep every surviving role covered, including hidden clean-docs; verify three backend sections plus Template and six subsections |
| Core isolation/no-resend and asynchronous workflow checks | Preserve behavioral sections; retarget removed create Director path to workflow and test its distinct obligations there |
| Create finalization route scenarios | Check plain ready with markers, plain ready without markers, and the exact Director finalization summary after approval; only the last sets Approved and replies with finalization `addressed (doc)` |
| `skill_author_guidance_keeps_the_heartbeat_backend_neutral` | Verify declared prerequisites and complete example rather than requiring copied monitor implementation |
| Source-link/path checks | Cover ordinary links, same-file fragments, Rspress explicit IDs, and hidden consumers; keep known dynamic backend placeholders explicitly classified |
| `cli_setup_doctor.rs` install fixtures | Compare the full expected surviving path set and bytes with installed SKILLS for every backend, preserving custom-path behavior and regular runtime files |
| SPEC/preset and exact-contract review | Verify preset fenced-copy equality and exact error/output/argv/schema/ordering inventory independently of page size |

Use fixture-based missing-target/missing-anchor cases to prove graph checks fail loudly. Check installed links from actual materialized roots with no repository fallback; require the reviewed 33-path payload independently of its embedded enumeration. Keep Claude config overrides, Codex config overrides and OpenCode's distinct skills/preset resolution covered by the existing isolated installation fixtures. Runtime content stays non-hidden and individual symlinks stay acyclic.

Run project-root commands through current mise tasks: `mise //cafleet:test --test docs_sync`, `mise //cafleet:test --test cli_setup_doctor`, and `mise //docs:build`. Use `mise //cafleet:format` and `mise //cafleet:lint` if Rust checks change; run existing targeted creation/message/monitor/preset/migration suites when their exact descriptive contracts are touched. The Rust tasks supply the admin-build prerequisite. Record actual command results; historical proposed docs-generate/docs-check tasks are not present tooling.

Inspect affected rendered navigation, moved anchors, literal contract cells, and the home Markdown/LLM output. Compare model data/default/freshness separately from moved runtime prose. Search all current tracked/hidden consumers for removed names, excluding historical design/research and generated/dependency outputs.

### Real-team smoke acceptance

Execution-scope amendment (2026-09-11): the user authorized skipping workflow tests if a container is needed. Assess an already available equivalent isolated environment through read-only evidence; when none is established and container provisioning would be required, record the live matrix as skipped under this authorization and proceed with local documentation, installation-fixture, contract and rendered-site checks. Container discovery/provisioning is outside the remaining execution work. A skipped case is reported as skipped, with its validation limitation, and supplies no live-pass evidence. This amendment governs the live preparation and Step 7 tasks below; their completion means recording either supported live results or the authorized skip disposition.

Run smoke validation during implementation against the rebuilt candidate artifact, with actual agent processes and an actual supported multiplexer. Existing fake-agent/tmux integration tests supplement this evidence and do not satisfy it. Use an isolated test account/container or equivalent provisioned environment with its own legitimate home, test repositories, skill installation, and `CAFLEET_DATABASE_URL`; preserve the operator's current installations, credentials, database and unrelated fleets. Use authorized backend authentication without copying credential contents into evidence.

Each fixture has a disposable local Git repository, no remote, an explicit output BASE, and recorded model/backend selections. Install the candidate's embedded skills into that environment and verify their hashes/paths before launch. Fixture authorization covers only its trivial code/doc changes and local phase commits; design/audit staging obeys the supplied host rules. A test operator answers actual workflow prompts and approves the concrete fixture deliverables through the backend's decision surface; elapsed time, canned transcript text and reviewer approval never substitute for user approval.

Use three fixture chains, one per Director backend, each covering create → interview → execute. Give each a small dependency-free function specification with distinct normal, boundary, and invalid-input cases so the TDD workflow has meaningful work. Record actual clarification, one ordinary-language revision, explicit document approval, persisted interview answers, implementation, reviewer approval, and approve-local finalization.

| Backend | Initial fixture member selection from the audited catalog | Monitor selection |
|---|---|---|
| Codex | `gpt-6-astra`, `high` for non-monitor members, including Reviewer | `gpt-5.6-luna`, backend-owned execution |
| Claude | `fable`, `high` for non-monitor members, including Reviewer | `haiku`, backend-owned execution |
| OpenCode | `opencode/glm-5.2`; omit unsupported effort | `opencode/big-pickle`, backend-owned execution |

These are fixture choices from the unchanged audited reference, not a model refresh. Verify actual availability before launch and record the resolved tokens; an unavailable required backend is incomplete coverage to report, not a silent substitution. Bootstrap exposes no monitor-effort flag, so use its supported model option and backend default effort.

| Live case | Required observations/evidence |
|---|---|
| Three create runs | Actual installed role/prerequisite reads; ready/live gates; questions and received answers precede document content; one revision is removed and reviewed; explicit operator approval precedes `ready (doc) — user approved; finalize`, Approved metadata and finalization `addressed (doc)` |
| Three interview runs | Analyzer reads full fixture and emits complete numbered questions; JSON retrieval, persisted numbering/answers/progress and early Analyzer-team teardown follow the workflow |
| Three execute runs | Tester then Programmer phase ownership; meaningful tests and timestamp/progress updates; conditional verification recorded; fresh full-diff Reviewer appears only after completion gates; local approval results in no remote action |
| Codex docs-only execute variant | Programmer-only composition and existing no-Tester routing work; fresh Reviewer and user approval still occur |
| One mixed-backend manual fleet | Use revised Run a fleet instructions with Codex Director, Claude and OpenCode ordinary members; confirm target cue/model lookup stays separate from Director tools and each member handles a real broker handoff |
| Codex monitor lifecycle | In the isolated fixture, confirm retained-session startup/live, a real later wake, and controlled loop-exit detection/restart confirmation; preserve IDs and recorded runtime outcomes |
| Cleanup of every fixture fleet | Monitor deletion precedes ordinary members; root-only registry check precedes fleet deletion; captures/runtime and final listing confirm fixture-owned processes/fleets are closed |

Record candidate revision/hash, installation roots, backend/model/effort, fixture paths, fleet/member/message IDs, read/pane evidence, tests, approval text, outcomes and cleanup in `.audit/live-smoke/`. Full read graphs and deterministic scenario checks cover additional branches: shell-only role reading, BASE unset/missing, startup early-exit/unconfirmed cases, unknown/working/awaiting panes, no-resend/failed relay, successful/failed requested-shell dispatch, create resume/review-only, test disputes, interview resume/early exit, and remote-approved push/PR failures. They are not all forced into live agent runs; any live failure is diagnosed and the affected case rerun after correction.

Use normal CAFleet monitor/facilitation, not a self-scheduled polling loop. A quiet pane alone supplies no failure or approval. Record missing backend/tool access or unmet case as incomplete validation, and retain the required case until it passes or the user explicitly revises acceptance.

### Baseline and expected savings

The integrity inventory counts decoded UTF-8 content, dereferences file symlinks, and deduplicates resolved physical source paths. It includes five navigation JSON files and excludes historical design/research, dependencies/build outputs, lockfiles, implementation code, LICENSE, and the Codex preset from the prose denominator.

| Measured baseline surface | Paths | Bytes |
|---|---:|---:|
| Public docs including navigation | 24 | 220,062 |
| Shipped skills including runtime aliases | 38 | 522,108 |
| Hidden maintenance Markdown | 28 | 163,685 |
| Root README/CONTRIBUTING/CLAUDE/SPEC | 4 | 215,521 |
| OpenCode preset Markdown | 1 | 2,008 |
| Delivered-path total | 95 | 1,123,384 |
| Unique authored total after alias deduplication | 83 | 927,500 |

Twelve aliases account for 195,884 bytes of the delivered/unique difference. Before rewriting, default three-backend setup projects 114 skill files plus two presets, totaling 116 files and 1,568,583 bytes; this is an installation projection, not compiled/compressed binary size.

The final map predicts 71 unique source files and 83 delivered prose/navigation paths, retaining all twelve aliases. The shipped payload predicts 33 files per backend and 101 total skill/preset files across the default three-backend setup. New verification code and hidden audit artifacts are measured separately from the consistent prose denominator.

The following drafting estimates partition physical sources, so they can be added without overlapping scanner spans. They narrow earlier estimates that assumed removal of the public monitoring procedure and subtract room for transferred text, preserved gates and corrections.

| Disjoint source group | Baseline bytes | Provisional net reduction |
|---|---:|---:|
| Public non-spec Markdown plus README | 70,373 | 5–9 KB |
| Six public spec Markdown files | 149,994 | 9–18 KB |
| Eleven CAFleet originals, excluding runtime aliases | 155,972 | 12–22 KB |
| Fifteen design-doc skill originals | 170,252 | 18–30 KB |
| Hidden maintenance Markdown | 163,685 | 25–38 KB |
| Root SPEC alone | 212,574 | 6–15 KB |
| Other root entries, nav JSON, OpenCode preset | 4,650 | No saving promised |
| Total | 927,500 | About 75–132 KB, or 8–14% |

KB here means 1,000 bytes. These are editorial estimates, not achieved savings or deletion quotas; actual rewrites may differ. Measure net source differences only after destination text is added, and report role-specific mandatory reading by phase/backend to catch a shorter tree that increases context burden. Whitespace words, physical lines, model tokens, installed bytes and compressed binaries are distinct metrics; this design makes no unmeasured token, latency or binary-size claim.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Freeze the contract and migration inventory

- [x] Recheck the audited revision/current changes and reproduce the unique/delivered baseline with the stated exclusions; save the task-local inventory. <!-- completed: 2026-09-11T18:01 -->
- [x] Build the old-clause/new-owner ledger for all twelve removals, root coverage units, hard prompt lines, named sections and test locators; mark selected corrections and unchanged deferred topics explicitly. <!-- completed: 2026-09-11T18:06 -->
- [x] Record the expected surviving owner/path set and per-role startup/read scenarios independently from embedded SKILLS; prepare isolated live fixtures and operator decision coverage. <!-- completed: 2026-09-11T18:07 -->

### Step 2: Consolidate public documentation

- [x] Move the complete manual lifecycle to Run a fleet, simplify Quickstart, and merge Fleet isolation/Model choice into their selected concept owners with all current reference/navigation updates. <!-- completed: 2026-09-11T18:13 -->
- [x] Apply the six-spec-page owner layout, preserving exact error/output/input/cleanup distinctions, Monitoring cadence/procedure and Storage recovery; apply verified public drift corrections. <!-- completed: 2026-09-11T18:27 -->
- [x] Align the Design document workflow and contributor journeys, remove the unsupported published-example promise, and preserve home/theme/configuration and literal rendering behavior. <!-- completed: 2026-09-11T18:27 -->

Step 2 status finalized under the 2026-09-11 user scope update. Completed source and validation evidence remains in `.audit/execute/step2-implementation.md`; remaining rendered inspection and further verification are skipped under that instruction. The Director reported 151 targeted runtime tests passing. No visual or live-workflow pass is claimed.

### Step 3: Consolidate shared CAFleet instructions

- [x] Merge Director reference into the generic role and recovery into supervision, create the named headings, rebase outgoing links and update every incoming/trigger/prose consumer. <!-- completed: 2026-09-11T18:33 -->
- [x] Move unique secondary CLI usage into core with exact runtime links, then remove the catalog; preserve actor-aware no-resend, shell exception, capture and shutdown scope. <!-- completed: 2026-09-11T18:35 -->
- [x] Shorten shared/role/backend boilerplate, implement the explicit reader/loader startup contract, and preserve BASE states, monitor lifecycle, prompt constraints and unchanged model data. <!-- completed: 2026-09-11T18:41 -->

Step 3 implementation dispositions and prompt exceptions are recorded in `.audit/execute/step3-implementation.md`, `step3-clause-dispositions.json` and `step3-prompt-exceptions.json`. Further verification is skipped under the Overview user scope update; existing tests and prior audit evidence are retained.

### Step 4: Consolidate design workflows

- [x] Merge create/execute Director roles into their workflows with composition, commit protocol, 200-line diagnostic/disclosure deltas, phase nudges and required reads, then delete the old roles. <!-- completed: 2026-09-11T18:47 -->
- [x] Centralize normalization and three coordination exemptions, and apply the documented fresh/resume/review-only/finalize, payload, feedback and approval-scope corrections, including the exact finalization handoff and non-finalizing plain-ready route. <!-- completed: 2026-09-11T18:51 -->
- [x] Reconstruct all seven specialized role prompts and mode variants from the shared frame/deltas, preserving hard lines, scope, timestamps, test ownership and phase timing. <!-- completed: 2026-09-11T18:54 -->

Step 4 dispositions, resolved Analyzer transport arbitration and nine source prompt templates are recorded in `.audit/execute/step4-implementation.md`, `step4-clause-dispositions.json` and `step4-prompt-manifest.json`. Analyzer uses supported tool-provided stdin without file edits; unavailable transport routes to the Director. Further verification is skipped under the Overview user scope update.

### Step 5: Consolidate root and maintenance documentation

- [x] Deduplicate SPEC internally under its numbered owners, transfer unique clauses first, complete verified asset/recovery/error detail and apply selected corrections while retaining deferred constraints. <!-- completed: 2026-09-11T19:03 -->
- [x] Merge the three clean-docs rubrics and two documentation rules into their selected owners; preserve six gated roles, review/application stages, legitimate absence coverage and complete sweep exclusions. <!-- completed: 2026-09-11T19:07 -->
- [x] Shorten skill-author with required phase reads and one complete example, correct verified transport/release descriptions, and preserve updater/model-refresh/preset-map contracts and SPEC-copy equality. <!-- completed: 2026-09-11T19:14 -->

### Step 6: Migrate checks and verify delivery

- [x] Migrate existing docs_sync locators and wording predicates to surviving owners while preserving coverage; additional cases and verification infrastructure are skipped under the revised scope. <!-- completed: 2026-09-11T19:23 -->
- [ ] SKIPPED under user scope: extend isolated setup fixtures and verify full installed paths/bytes and offline closure. Existing unaffected fixtures remain unchanged.
- [ ] SKIPPED under user scope: final mise checks, site build and rendered inspection. Prior phase results retain their recorded snapshot limits.

### Step 7: Run real workflow teams

- [ ] SKIPPED under user scope: candidate installation and all three backend workflow chains. See `.audit/execute/live-plan.md`.
- [ ] SKIPPED under user scope: docs-only execution, mixed-backend handoff and Codex monitor lifecycle cases.
- [ ] SKIPPED under user scope: fixture teardown verification; no fixture fleet was created. The actual execution team's eventual cleanup remains required.

### Step 8: Review the simplified result

- [ ] SKIPPED under user scope: final clause/owner/consumer audit, current-reference sweep and delivery checks. Existing implementation dispositions are retained.
- [x] Report source/delivered byte totals: unique sources save 73,457 bytes (7.92%); delivered accounting saves 77,957 bytes (6.94%). Per-role reading and final model comparison remain unmeasured. <!-- completed: 2026-09-11T19:22 -->
- [x] Present completed implementation, deferred issues and explicit verification limits; user approved publication with "push". Branch pushed and PR #375 created. Independent review is skipped under user scope. <!-- completed: 2026-09-11T19:25 -->

## Execution changelog

2026-09-11: Completed the authorized implementation through 75aa75de and published https://github.com/himkt/cafleet/pull/375 using gh pr create --fill. Unique authored documentation saves 73,457 bytes (7.92%). Additional verification and independent review were skipped under user instruction; prior results apply only to their recorded snapshots. Design and audit files remain local and uncommitted under user Git rules.
