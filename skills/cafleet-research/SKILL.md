---
name: cafleet-research
description: >-
  Use when the user asks to research a topic or create a multi-source research
  report (report workflow → Director/Manager/Researcher team writing to
  researches/<topic>/); to build a presentation, slide deck, or reading
  transcript from a report (presentation workflow); to create a chart, plot,
  graph, figure, or data visualization (visualization reference); or to author
  slides with the custom Slidev theme (slidev reference). Always invoke this
  skill and route into the matching workflow, orchestrated as a CAFleet team.
  Members in member teams load this skill by its name cafleet-research via their
  backend's skill-loader.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, Agent, TaskCreate, TaskUpdate, TaskList, TaskGet, TaskStop
---

# Research Skill (CAFleet Edition)

This skill is the umbrella for research and presentation media: two CAFleet-native **orchestration workflows** (research report, presentation) plus two load-on-demand utility **reference pages** (matplotlib visualization, the custom Slidev theme). It is a dispatcher — route to the matching workflow body to run a team, or read a reference page for a standalone utility.

**Teammates in member teams** that need this skill load it by its name `cafleet-research` via their backend's skill-loader — never by reading the skill files directly.

## Required reading

Before routing into a workflow or consulting a reference page, Read your overlay — it is row #1 below. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; a standalone / main-session reader uses its own identity.

**Load-bearing — Read before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — you emit a literal `{monitor_model}` / `{task_coord}` / `{decision_surface}` in the workflow you route into, **or** guess a wrong/default value, **or** ignore a backend note (codex has no harness task list) |

**On-demand — consult directly (no team), only when the task needs it:**

| Read | When |
|------|------|
| [reference/visualization.md](reference/visualization.md) | you are creating a chart / plot / graph / figure or visualizing data |
| [reference/slidev.md](reference/slidev.md) | you are authoring slides with the custom Slidev theme (layouts, components, techniques) |

Each workflow body (report / presentation) carries its own Required-reading block for the team it runs — base-dir and supervision are gated there, not on this dispatch surface.

Before acting, resolve every `{token}` you will use to its overlay value (or the documented default); a literal `{token}` in any command or message is a defect.

## Dispatch

When the user's request matches a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, the moment the request matches, without waiting for the user to say "use cafleet".

**Routing into the report or presentation workflow means executing its entire orchestration.** The dedicated monitoring member is spawned first-in (`cafleet member create --role monitor`, gating the team behind its `ready: monitor live` handshake), then the role team, then the workflow body's review and revision rounds run through to approval. The linked workflow body is the authoritative procedure.

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Research a topic / create a multi-source research report | the **report** workflow ([report/report.md](report/report.md)) — Director/Manager/Researcher team → `researches/<topic>/` |
| Build a presentation / slide deck / reading transcript from a report | the **presentation** workflow ([presentation/presentation.md](presentation/presentation.md)) |

For a chart/figure or the Slidev theme — consult the **On-demand** reference pages in § Required reading above (no team needed).

The report workflow chains into the presentation workflow after user approval. The two reference pages are standalone utilities the presentation workflow also reads. Always route research work through the workflow bodies above — each runs the full CAFleet team.
