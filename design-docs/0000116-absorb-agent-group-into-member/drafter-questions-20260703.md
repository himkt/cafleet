# Drafter clarifying questions — 0000116 absorb-agent-group-into-member

My understanding of the already-decided direction (please correct if wrong):

- Drop `cafleet agent register` entirely — no live workflow consumer; `fleet create` bootstraps the root Director and Administrator internally (direct row inserts, not via the CLI).
- Extend `cafleet member delete` to registry-soft-delete a placementless agent, replacing the current "agent {id} has no placement; use `cafleet agent deregister` instead" error path in `cli/member.py`. The root-Director guard and the broker-level Administrator guard stay.
- Preserve the introspection currently exclusive to `agent list` / `agent show` (root Director + Administrator visibility, kind, skills/card, placement detail) somewhere on the `member` surface.
- Broker functions (`register_agent`, `deregister_agent`, `list_agents`, `get_agent`) stay — this is a CLI-surface + documentation change; the WebUI API keeps using them.
- Hard break per the removal rule: after this lands, the corpus reads as if the `agent` group never existed (except design docs / git history).

Questions, grouped by category:

## A. Purpose & Scope — fate of the paneless registry-only external agent

Context: today anyone can `agent register` a paneless agent; the broker accepts messages to paneless agents (the Administrator is itself paneless and messageable — the WebUI depends on that); `skills/cafleet/reference/cli.md` documents a self-registration bootstrap workflow for external agents.

**Q1. Is the user-creatable paneless external-agent concept dead or dormant?**
- (a) **Dead (recommended)** — no user-facing way to create a paneless agent remains. `broker.register_agent(placement=None)` stays as internal machinery only (fleet bootstrap, WebUI reads). The self-registration bootstrap workflow in `skills/cafleet/reference/cli.md` is deleted outright. Paneless rows can then only be the built-in Administrator (plus any pre-existing rows), so "messages to paneless agents" degenerates to "messages to the Administrator" — no broker change needed. Docs stop describing external agents anywhere (removal rule: total cleanup).
- (b) **Dormant** — the broker/HTTP layer is documented as a registry that *could* host paneless A2A agents, but the CLI creation path is removed. Docs keep a paneless-agent concept section.

If (a): the restoration plan for a future A2A/HTTP registration lives only in this design doc, correct?

## B. API / Interface — introspection surface shape

Context: `member list` today excludes the root Director and shows placement columns only (no kind, no skills, no paneless rows). `agent list`/`agent show` show every active agent including the Administrator, with kind/skills/description via the agent card.

**Q2. What is the introspection surface after absorption?**
- (a) **Both (recommended)** — `member list --all` (adds root Director, Administrator, and any placementless rows to the table, with a kind/backend indication) **plus** a new `member show --member-id <id>` (full detail: description, kind, skills, status, placement block, `--full` semantics like today's `agent show`).
- (b) `member list --all` only; per-agent detail via `--json`.
- (c) `member show` only; `member list` unchanged (Director/Administrator visible only if you already know the id).

**Q3. `member show` flag + gate shape** — today `agent show` takes `--agent-id` (requester, fleet-membership-gated) + `--id` (target). The `member` group convention is a bare `--member-id` target with no requester flag (`member list` has none). Adopt `--member-id <target>` only and drop the requester gate? (recommend yes — consistent with the rest of the `member` group)

## C. Testing / drift guard

**Q4. Spawn-prompt drift guard scope** — add the legacy names to `FORBIDDEN_PATTERNS` in `spawn_prompt_guard.py`?
- (a) **Single pattern `"cafleet agent "` (recommended)** — catches all four subcommands and any future reintroduction; the trailing space + lowercase `cafleet` prefix keeps prose like "a CAFleet agent" safe.
- (b) Four exact patterns (`cafleet agent register`, `… list`, `… show`, `… deregister`).
- (c) No new guard patterns — rely on the one-time migration only.

## D. Dependencies / WebUI

**Q5. WebUI scope** — the only WebUI change is the Dashboard empty-state string ("Use the `cafleet agent register` CLI to add one." → point at `cafleet member create`); the HTTP API (`GET /api/agents` etc.) is unchanged. Correct? (recommend yes — minimal)

One assumption I will bake in unless told otherwise: a regression guard test asserting `cafleet agent …` now fails with Click's "No such command" is in scope (testing the absence, allowed by the removal rule), and `cafleet/tests/cli/test_agent.py` is replaced by member-side coverage.
