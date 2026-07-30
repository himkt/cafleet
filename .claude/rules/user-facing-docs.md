# User-Facing Docs

`docs/` is the user- and operator-facing documentation site. Its pages explain
what the tool does and how to use it — in terms of behavior, concepts, and
user-visible artifacts, never in terms of where the implementation lives.

## Describe behavior, not code locations

- Name components by concept ("the broker", "the HTTP server", "the
  supervision protocol"), anchored to the Core terms table in the concepts
  overview — not by source file or module path.
- When a page needs to say where a protocol or catalog is defined, attribute
  it to the shipping artifact the reader has ("part of the cafleet skill",
  "bundled with every deployed skill replica") without the repo-internal file
  path.
- When removing a path mention, replace it with a self-contained explanation
  of the behavior or a link to the docs page that owns the fact — never a bare
  deletion that leaves the sentence emptier.

## Exemptions — where a concrete path is the content

| Surface | Why paths are legitimate there |
|---|---|
| `docs/contributing.md` | Contributor instructions: paths stay where the path IS the instruction (project-structure table, dependency-edit recipes, design-doc layout). Incidental implementation pointers are still rewritten. |
| Path-as-contract mentions in `docs/spec/*` | Paths that are part of a specified contract (release-archive preset paths and their install targets, the `SPEC.md` DDL source of truth). Incidental pointers are still rewritten. |
| User-machine paths | `~/...` install and runtime locations describe the reader's own machine. |
| Deliverable locations in the user's repo | e.g. `design-docs/NNNNNNN-<slug>/design-doc.md` as a workflow output layout. |
