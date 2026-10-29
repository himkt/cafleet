---
name: update-readme
description: >-
  Reconcile README.md with the documentation site and SPEC.md with the shipped
  binary and code when those maintained surfaces change.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash
---

# Update README and SPEC

Read the documentation-site concepts and navigation, the shipped CLI behavior and code, and the current maintained targets. Apply the task directly when it needs one session; use CAFleet for independent delegated review when the task calls for it. An assigned member performs its own scoped work within the existing team.

## README structure

Retain the title/demo/pitch, Install, Documentation and License surfaces. Align the pitch with the site hero, installation with Quickstart and documentation links with navigation; the Specification link points at `SPEC.md`. Keep detailed CLI/architecture/development content in the documentation site.

## SPEC maintenance

SPEC owns every contract and is the self-contained reimplementation specification. Reconcile it with the shipped binary and code: current CLI options/defaults, environment, schema, native input, finite receiving, authored errors, JSON order, text layouts, runtime, multiplexer and backend behavior. Preserve its numbered structure and required detail, update the affected contracts together, remove obsolete concepts completely and keep the description affirmative.

Retain exact current argv, SQL and error blocks, and reference bundled preset files by path. Verify `§` cross-references against the section headings. Report the focused validation performed and material limitations. Write all maintained artifacts in English.
