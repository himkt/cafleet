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
  Members in agent teams load this skill by its name cafleet-research via their
  backend's skill-loader.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, Agent, TaskCreate, TaskUpdate, TaskList, TaskGet, TaskStop
---

# Research Skill (CAFleet Edition)

This skill is the umbrella for research and presentation media: two CAFleet-native **orchestration workflows** (research report, presentation) plus two load-on-demand utility **reference pages** (matplotlib visualization, the custom Slidev theme). It is a dispatcher — route to the matching workflow body to run a team, or read a reference page for a standalone utility.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

**Teammates in agent teams** that need this skill load it by its name `cafleet-research` via their backend's skill-loader — never by reading the skill files directly.

## Dispatch

When the user's request matches a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, the moment the request matches, without waiting for the user to say "use cafleet".

**Routing into the report or presentation workflow means executing its entire orchestration.** The dedicated monitoring member is spawned first-in (`cafleet member create --role monitor`, gating the team behind its `ready: monitor live` handshake), then the role team, then the message-broker quality loop iterates to approval. The linked workflow body is the authoritative procedure.

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Research a topic / create a multi-source research report | the **report** workflow ([report/report.md](report/report.md)) — Director/Manager/Researcher team → `researches/<topic>/` |
| Build a presentation / slide deck / reading transcript from a report | the **presentation** workflow ([presentation/presentation.md](presentation/presentation.md)) |

Consult these reference pages directly (no team):

| When the user wants to… | Consult |
|:--|:--|
| Create a chart / plot / graph / figure or visualize data | the **visualization** reference ([reference/visualization.md](reference/visualization.md)) |
| Author slides with the custom Slidev theme (layouts, components, techniques) | the **slidev** reference ([reference/slidev.md](reference/slidev.md)) |

The report workflow chains into the presentation workflow after user approval. The two reference pages are standalone utilities the presentation workflow also reads. Always route research work through the workflow bodies above — each runs the full CAFleet team.
