# Redundancy audit brief (shared by every auditor)

Repository: /development-pvc/work/himkt/cafleet (Rust CLI under cafleet/, docs site under docs/, embedded skills under skills/, presets/, project rules under .claude/).

## Goal

The user wants the repository to carry no waste. Audit your assigned slice exhaustively and report every concrete opportunity to remove or simplify code, tests, fixtures, and documentation WITHOUT changing the shipped specification (CLI behavior, flags, outputs, error strings, JSON key order, SQLite schema/migrations semantics, skill-visible contracts). Your findings feed a refactoring/cleanup design doc.

The user's explicit requirements:

1. Test-only files and folders must go. In this Rust project every test lives as a unit test inside the implementation file it exercises (a #[cfg(test)] mod tests in the same .rs file). Report every test-only file/folder in or touching your slice (e.g. *_tests.rs, runner_tests/, contract_tests/, test fixture directories, test-support modules) and say exactly which implementation file(s) each test should move into. Note shared test helpers and how they should be placed (e.g. a #[cfg(test)] helper inside the implementation module that owns the type). Note fixtures that tests load (JSON etc.) and whether they are also runtime data (include_str! / bundled) or test-only.
2. Redundant design must be simplified while the specification stays identical: duplicated logic, parallel near-identical types/enums/functions, needless abstraction layers or traits with one implementor, wrappers that only forward, dead or unreachable code, unused pub items, overly defensive duplicate validation, redundant tests that assert the same behavior twice, oversized test scaffolding, duplicated documentation that restates another page, stale or contradictory docs.

## How to work

- Read-only on the repository. Write ONLY your findings file (path in your spawn prompt). Do not edit, format, build-in-place, or commit anything else.
- You may run read-only commands (grep, git ls-files, git log, wc, cargo metadata). Prefer the Read/Grep tools. Build/test commands are unnecessary; if you want a usage check, use grep across the whole repo (callers may live outside your slice).
- Verify every finding with evidence (file:line, grep counts of callers). Do not report speculative items; mark uncertain ones as "needs-verification" with what would confirm them.
- Follow /development-pvc/work/himkt/cafleet/CLAUDE.md and .claude/rules/*.md (code-quality, documentation-maintenance, database-migrations) — proposals must comply with them.
- Prioritize by impact (lines removed / concepts removed / maintenance burden).

## Findings file format (Markdown, English)

```
# Audit: <slice name>

## Summary
<3-6 sentences: biggest wins, total estimated LOC removable>

## Test placement inventory
| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |

## Findings
### <SLICE-ID>-<NN>: <short title>
- Category: test-placement | dead-code | duplication | over-abstraction | redundant-test | doc-duplication | stale-doc | other
- Location: path:line[-line]
- Evidence: <what you observed, caller counts, cross-references>
- Proposal: <concrete simplification>
- Spec impact: none — <why the shipped contract is unchanged>
- Risk: low|medium|high — <why>
- Estimated delta: -<N> lines
```

Omit the Test placement inventory table rows if your slice has none (write "None in slice."). When done, send the Director one message: complete (audit) — <findings file path> — <N findings, ~M lines removable>.
