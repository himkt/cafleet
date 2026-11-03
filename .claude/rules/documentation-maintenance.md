# Documentation Maintenance

Rules for keeping the project's documentation in sync with its code. These apply
to every change that touches a user- or contract-facing surface; the design-doc
numbering/format rules live in the shared [design-document guidelines](../../skills/cafleet-design-doc/reference/guidelines.md#file-layout).

## Three audiences

| Audience | Home | Content |
|---|---|---|
| Everything (contract) | `SPEC.md` | Every contract: CLI, output, errors, environment, schema, runtime, multiplexer, backends, presets, wire. §6.7 Coding-agent backends covers role requirements, per-backend role eligibility, backend argv, model/effort validation, permission posture, presets and host evidence. |
| Users | `docs/` site | Concepts, usage, setup and configuration steps a user performs; exact contract detail is a link to `SPEC.md`. `docs/docs/concepts/coding-agents.md` explains backend differences, selection and configuration. |
| Agents | `skills/` | Necessary-and-sufficient instructions to drive cafleet, with command facts in `skills/cafleet/reference/cli.md`. `skills/cafleet/reference/coding-agents.md` owns the runtime overlay. |

Skills never link `docs/` or `SPEC.md`; they carry the facts they need. `SPEC.md` never links `docs/`.

## Implementation Order

When implementing a design document, ALWAYS update documentation FIRST before writing any code, in this order: `SPEC.md`, then `docs/` pages, then `README.md`, then the affected `skills/` files, then project rules if needed. Only after documentation is complete should code implementation begin.

## First-class documentation targets

Every target below is updated in the same design-doc cycle as the change that touches its surface, with the smallest edit that removes the drift. **Drift on any target below is a blocker for "documentation complete".**

| Target | Update trigger | Drift consequence |
|---|---|---|
| `SPEC.md` | Any change to a contract surface — CLI options, `CAFLEET_*` configuration, the SQLite schema, error strings, JSON key order, text layouts, runtime, multiplexer, backend or preset behavior (maintained via the `/update-readme` skill) | The single authoritative specification no longer matches the shipped contract |
| `docs/` | Any change to features, architecture, usage, setup or configuration a user performs (a new architectural axis gets its own Concepts page) | The user documentation goes stale |
| `README.md` | The thin entry point itself changes — the pitch drifts from `docs/docs/index.md`, the install commands change, or a docs-site section is added or removed (sync via the `/update-readme` skill) | The entry point misleads new users |
| `skills/*/SKILL.md` and skill references | Any change to CLI commands, flags, environment variables, required arguments, output formats, or the expected invocation workflow | Agents load stale skills as ground truth and produce broken tool calls |
| `skills/cafleet/reference/cli.md` | Any change to a `SPEC.md` fact that `cli.md` carries, updated in the same change | Agents run stale commands |
| `.claude/rules/` | The project convention the rule captures changes | Agents follow outdated conventions |

`SPEC.md` edits preserve its section structure and contract-level detail, and stay descriptive — specification only, no recommendations or implementation advice. Paths that are part of a specified contract (embedded preset paths and their install targets, migration files) are `SPEC.md` content.

## Audiences and paths

`docs/` is the user- and operator-facing documentation site. Its pages explain
what the tool does and how to use it — in terms of behavior, concepts, and
user-visible artifacts, never in terms of where the implementation lives.

### Describe behavior, not code locations

- Name components by concept ("the broker", "the finite receiver", "the
  receive procedure"), anchored to the Core terms table in the concepts
  overview — not by source file or module path.
- When a page needs to say where a protocol or catalog is defined, attribute
  it to the shipping artifact the reader has ("part of the cafleet skill",
  "bundled with every deployed skill replica") without the repo-internal file
  path, or link the owning `SPEC.md` section for an exact contract.
- When removing a path mention, replace it with a self-contained explanation
  of the behavior or a link to the page that owns the fact — never a bare
  deletion that leaves the sentence emptier.

### Exemptions — where a concrete path is the content

| Surface | Why paths are legitimate there |
|---|---|
| `docs/docs/contributing.md` | Contributor instructions: paths stay where the path IS the instruction (project-structure table, dependency-edit recipes, design-doc layout). Incidental implementation pointers are still rewritten. |
| User-machine paths | `~/...` install and runtime locations describe the reader's own machine. |
| Deliverable locations in the user's repo | e.g. `design-docs/NNNNNNN-<slug>/design-doc.md` as a workflow output layout. |

## Table conventions

Rules for choosing between a table, a list, and prose in the documentation, and
for keeping the resulting tables consistent. These apply to every page under
`docs/`, and to any user-facing prose that enumerates parallel items.

### The tabulate rule

Content becomes a table when **three or more parallel items each carry two or
more of the same attributes**. Content stays prose when it is a single item, an
ordered procedure, or rationale for one decision.

The counted parallel items may sit on **either axis** — as rows or as columns.
A two-subject comparison such as tmux vs herdr clears the bar when the counted
items are the behaviors compared (rows), with the two subjects as columns; it
does not clear the bar on the strength of the two subjects alone.

| Content | Form |
|---|---|
| Three or more parallel items, two or more shared attributes | Table |
| Ordered steps a reader performs in sequence | Numbered list |
| Constraints or requirements with no shared attribute axis | Bulleted list |
| One item, however many attributes | Prose |
| Rationale, caveats, and "why" for a decision | Prose, adjacent to the table it qualifies |

### Rule lists: which convert and which stay

A rule list becomes a table when the reader uses it for **lookup or decision** —
they arrive knowing one key and want the matching behavior, and the rules' order
is incidental.

A rule list stays a numbered list when the ordering is **genuinely
load-bearing** — an earlier rule preempts a later one, so renumbering would
change the outcome. When the outcomes of an ordered rule set are themselves
worth scanning, add a truth table alongside the numbered list rather than
replacing it.

### The two anti-rules

Both bind sitewide, including tables a given change never otherwise touches.

- **A table has two or more data rows.** One data row costs a header row and
  buys nothing over a sentence; write the sentence.
- **A table cell holds at most two sentences.** More than that defeats the
  vertical scan the table shape promises; move the overflow to prose beneath the
  table.

A verbatim quoted contract string — an error message, an output line — counts as
one unit toward the cell cap regardless of its internal sentence count. Contract
strings are preserved verbatim when they move into a cell.

### One owner per enumeration

An enumeration that would otherwise appear on more than one page gets exactly
one **owning page** carrying the table. Every other mention is a **link plus a
one-clause summary** — enough for the reader to know whether to follow it, never
a restatement of the owned attributes.

Two rules keep the non-owning mentions honest:

- **Echo rule.** A non-owning page may describe an owned enumeration in
  qualitative magnitude terms ("a receive operation bounds its native task lifetime"); it states no exact values.
- **Term tie-break.** When copies have drifted on what to call something, the
  term in the Core terms table of the concepts overview governs. The owning
  table and every linking mention adopt it.

### Rendering conventions

| Element | Convention |
|---|---|
| Literals — flags, values, env vars, error strings, field and column names | Code span |
| A cell with no applicable value | Em-dash (`—`), never an empty cell or "N/A" |
| Column alignment | Default left alignment; no alignment colons unless an adjacent existing table already uses them |
| Header wording | Noun phrase, sentence case, no trailing punctuation |
| Boolean-ish columns | `yes` / `no`, not `✓` / `✗` |
| A literal `\|` inside a cell | A raw `<code>` element using `&#124;` (plus `&lt;` / `&gt;` when the string also carries angle brackets) |

Use the raw `<code>` entity form above for literal pipes, as established by rendered site evidence. When changing a piped cell, inspect its rendered HTML: a successful build alone does not establish an intact cell.

## Coding-agent overlay

Write CAFleet base instructions so they read the same on every backend: family `SKILL.md` files, workflow bodies, roles, and shared reference pages prescribe neutral behavior and point to the relevant runtime overlay.

### Where backend specifics live

`skills/cafleet/reference/coding-agents.md` is the canonical backend reference. Each self-contained backend section contains these five subsections in order:

1. **Runtime bindings**: the five runtime placeholders for decision surface, permission flags, pane title, skill loader, and effort levels.
2. **Role defaults**: the concrete model and effort assignments for reviewers and other members; models come from the backend catalog (or are `—` when the backend has no maintained catalog) and effort matches its runtime capability.
3. **Model catalog**: exact spawn tokens, aliases, reviewed capability classes/order, prices, context notes, and official provenance.
4. **Note → applies at**: constraints bound to their tokens and consuming instructions.
5. **Role eligibility**: per-role `yes`/`no` verdicts for Director and Member, each with checked native version, source and date.

Backend-specific `####` subsections, such as a native reception recipe, may follow Note → applies at and precede Role eligibility. Add a backend by copying the Template in this rule and supplying all five subsections. Keep each section complete for its own backend, with shared policy and supervision linked to their authoritative homes. Model selection belongs to `skills/cafleet/roles/director.md` § Model selection. The `cafleet-model-list-refresh` skill exclusively maintains model catalogs, provenance/context notes, canonical Role defaults and shared freshness metadata from official sources, refreshing model data at least every 30 days. Runtime documentation maintenance owns Runtime bindings, bound runtime notes, Role eligibility and worked resolutions, and re-checks eligibility whenever a bundled profile boundary or `Evidence:` line changes; structural reorganization preserves the model freshness date.

### How the base and overlay connect

Each reader entry point places its runtime overlay first in its Required-reading block — as the first table row or the first prose reference — gated before action. Link `coding-agents.md#<name>`, substituting the reader's concrete backend anchor when used. The reader resolves the overlay with the procedure in `skills/cafleet/SKILL.md` § Resolve your overlay, which owns the subject selection, materialization, note application, emission self-check and documented neutral defaults. Report a missing required supported-backend section, malformed table, or broken reference as a documentation defect.

State shared operational facts in each home for its audience.

### Director verdict changes

To change a backend's Director verdict, a maintainer re-runs the Director requirements in `SPEC.md` §6.7 Role requirements for an installed native version: native goal lifecycle controls from official documentation or installed help, and streamed background output while supervision continues from the CAFleet stream probe defined there. Record the version, source and date in `SPEC.md` § Role eligibility and the backend's Role eligibility table, then add a native reception recipe or state the manual `poll` / finite `wait` reception limit.

### Template

Add a `## <name>` backend with every subsection below. Fill every value, keep runtime bindings as short noun phrases, and bind each caveat to its token and consuming instruction. Shared selection and supervision policy stays at its linked owner.

#### Runtime bindings

Supply all five tokens: `{decision_surface}` (recorded user-reaction surface; members relay), `{permission_flags}` (omitted launch-policy flags, the default mode and its `auto`/`manual` class, and explicit-selection behavior), `{pane_title}` (title mechanism or no analog), `{skill_loader}` (supported loader or complete absolute-path reads), and `{effort_levels}` (accepted values/flag or unsupported).

#### Role defaults

Set `{reviewer_model}` and `{other_model}` to catalog tokens or aliases. A backend without a maintained catalog sets them to `—`, meaning omit `--model` so the operator's configured default applies. Set their matching effort placeholders to a supported level, or to `—` when the backend does not support effort or has no maintained catalog.

#### Model catalog

List exact spawn tokens, valid aliases, reviewed most-to-least capability ordering, classes, and standard provider USD/MTok input/output prices. Include relevant context windows and availability constraints, required prefixes and official pricing/availability/configuration sources. Follow the freshness contract and [Director policy](../../skills/cafleet/roles/director.md#model-selection). A backend without a maintained catalog states "No maintained catalog", its `--model` pass-through rule and the source of the operator's configured default instead.

#### Note → applies at

Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind a `{permission_flags}` row stating the posture's denial semantics — what a member runs unprompted, what the harness denies, and when the member routes — and classifying every launch mode `auto` or `manual`, to the member command-execution, member-routing, Director-dispatch and supervision instructions that point at it.

#### Role eligibility

Use a `Role | Eligible | Evidence` table with one Director row and one Member row. Mark `yes` only when every Eligible-tier requirement in [Role requirements](../../skills/cafleet/reference/coding-agents.md#role-requirements) is established; otherwise `no` with "not established" or the observed failure. Each Evidence cell names the source, native version and `checked YYYY-MM-DD` for each requirement group in at most two sentences, and a Member row names its bundled profile instead of listing its capabilities.
