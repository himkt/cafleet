# Documentation Tables

Rules for choosing between a table, a list, and prose in the documentation, and
for keeping the resulting tables consistent. These apply to every page under
`docs/`, and to any user-facing prose that enumerates parallel items.

## The tabulate rule

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

## Rule lists: which convert and which stay

A rule list becomes a table when the reader uses it for **lookup or decision** —
they arrive knowing one key and want the matching behavior, and the rules' order
is incidental.

A rule list stays a numbered list when the ordering is **genuinely
load-bearing** — an earlier rule preempts a later one, so renumbering would
change the outcome. When the outcomes of an ordered rule set are themselves
worth scanning, add a truth table alongside the numbered list rather than
replacing it.

## The two anti-rules

Both bind sitewide, including tables a given change never otherwise touches.

- **A table has two or more data rows.** One data row costs a header row and
  buys nothing over a sentence; write the sentence.
- **A table cell holds at most two sentences.** More than that defeats the
  vertical scan the table shape promises; move the overflow to prose beneath the
  table.

A verbatim quoted contract string — an error message, an output line — counts as
one unit toward the cell cap regardless of its internal sentence count. Contract
strings are preserved verbatim when they move into a cell.

## One owner per enumeration

An enumeration that would otherwise appear on more than one page gets exactly
one **owning page** carrying the table. Every other mention is a **link plus a
one-clause summary** — enough for the reader to know whether to follow it, never
a restatement of the owned attributes.

Two rules keep the non-owning mentions honest:

- **Echo rule.** A non-owning page may describe an owned enumeration in
  qualitative magnitude terms ("the stall-check fires several times more often
  than a member's own ping interval"); it states no exact values.
- **Term tie-break.** When copies have drifted on what to call something, the
  term in the Core terms table of the concepts overview governs. The owning
  table and every linking mention adopt it.

## Rendering conventions

| Element | Convention |
|---|---|
| Literals — flags, values, env vars, error strings, field and column names | Code span |
| A cell with no applicable value | Em-dash (`—`), never an empty cell or "N/A" |
| Column alignment | Default left alignment; no alignment colons unless an adjacent existing table already uses them |
| Header wording | Noun phrase, sentence case, no trailing punctuation |
| Boolean-ish columns | `yes` / `no`, not `✓` / `✗` |
| A literal `\|` inside a cell | A raw `<code>` element using `&#124;` (plus `&lt;` / `&gt;` when the string also carries angle brackets) |

A `\|` escape does not work inside a code span — Python-Markdown leaves the
backslash visible in the rendered page. Verify any piped cell by inspecting the
rendered HTML: the docs build renders a broken cell without error, so a passing
build is not evidence the cell is intact.

## Scope

Applies whenever you author or revise a page under `docs/`, and whenever a
change adds an enumeration to user-facing prose.

Two neighbouring rules govern what may go *in* a cell and when the docs must be
updated at all:

- `user-facing-docs.md` — whether a repository file path may appear on the page,
  and the exempt surfaces where it may.
- `documentation-maintenance.md` — the documentation-first implementation order
  and the first-class documentation targets.
