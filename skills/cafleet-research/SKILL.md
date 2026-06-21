---
name: cafleet-research
description: >-
  Comprehensive multi-source research reports, Slidev presentations, charts, and
  data visualizations. Use to create a research report (multi-agent
  Director/Manager/Researcher team writing to researches/<topic>/), build a
  Slidev presentation / slide deck / reading transcript from a report, create a
  chart / plot / graph / figure or visualize data with matplotlib, or author
  slides with the custom Slidev theme. Members in agent teams load this skill by
  its name cafleet-research via their backend's skill-loader. Do NOT do a quick
  web search and summarize — route through this skill's workflows.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, Agent, TaskCreate, TaskUpdate, TaskList, TaskGet, TaskStop
---

# Research Skill (CAFleet Edition)

This skill is the umbrella for research and presentation media: two CAFleet-native **orchestration workflows** (research report, presentation) plus two load-on-demand utility **reference pages** (matplotlib visualization, the custom Slidev theme). It is a dispatcher — route to the matching workflow body to run a team, or read a reference page for a standalone utility.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

**Teammates in agent teams** that need this skill load it by its name `cafleet-research` via their backend's skill-loader — never by reading the skill files directly.

## When to use

| You want to… | Go to |
|:--|:--|
| Create a comprehensive multi-source research report (Director/Manager/Researcher team → `researches/<topic>/`) | [report/report.md](report/report.md) |
| Build a Slidev presentation / slide deck / reading transcript from an approved report | [presentation/presentation.md](presentation/presentation.md) |
| Create a chart / plot / graph / figure or visualize data with matplotlib | [reference/visualization.md](reference/visualization.md) |
| Author slides with the custom Slidev theme (layouts, components, techniques) | [reference/slidev.md](reference/slidev.md) |

The report workflow chains into the presentation workflow after user approval. The two reference pages are standalone utilities the presentation workflow also reads. Do NOT do a quick web search and summarize — route through the workflows above.
