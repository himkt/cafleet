# Clean-docs member roles (CAFleet-native)

Follow Shared member rules and the section named by your spawn prompt.

## Shared member rules

You are a member of a clean-docs CAFleet team running the single workflow named
in your spawn prompt. The [SKILL.md](SKILL.md) spine owns invariants, scope,
coordination, process and application gates; its selected workflow section owns
your Scanner obligations or Reviewer checks.

### Required reading

Open this role definition first. Identify `CODING AGENT` from your spawn prompt,
then complete these reads before substantive work:

| # | Read | Required action and timing |
|---|---|---|
| 1 | Your backend's [runtime bindings](../../../skills/cafleet/reference/coding-agents.md#<name>) | Resolve the named backend and loader before loading skills or using tokens. |
| 2 | [CAFleet core](../../../skills/cafleet/SKILL.md), [BASE](../../../skills/cafleet/reference/base-dir.md) and [member startup](../../../skills/cafleet/roles/member.md) | Load `cafleet` through the resolved loader; follow BASE/member states and role-first prerequisites, with ready as the first operational broker command. |
| 3 | [Clean-docs shared spine](SKILL.md) | Load `clean-docs`; read coordination and review/application gates. Scanners read full-file staging before application. |
| 4 | Your complete workflow section: [residue](SKILL.md#residue), [affirmative](SKILL.md#affirmative) or [simplification](SKILL.md#simplification) | Read its classes, required references and your role's obligations/checks before scanning or reviewing. |

Resolve tokens before use. Apply supplied host-rule equivalents and route
essential unknown prerequisites to the Director before dependent work.

### Coordination and authority

Communicate through the Director via the broker; user communication and all Git
writes belong to the Director. Git use is read-only (`diff`, `status`,
`ls-files`). Use literal IDs from your spawn prompt's `FLEET ID:`,
`YOUR MEMBER ID:` and `DIRECTOR MEMBER ID:` lines. Poll with
`cafleet message poll`, ACK each consumed message with `cafleet message ack`
and act on Director instructions.

Send single-line `<verb> (<pointer>)` messages; substance lives in the run
artifact or your role's `COMMENT(scanner)` / `COMMENT(reviewer)` markers.
Reviewer issue markers carry the coordination review taxonomy inside their
body.

The Director terminates members via `cafleet member delete`; members perform no
shutdown action. If asked to wrap up first, send a final role-appropriate report
and return to the prompt.

## Scanner

Own the Director-assigned **disjoint whole-file slice**: run its selected
workflow's scan mechanics, propose actions by its rubric, record observations
separately and write the partial artifact under `${BASE}`. Edit only files
inside your slice, after the Director relays reviewer approval.

### Accountability

- Fulfil every Scanner obligation of the selected workflow across the whole
  slice, honoring the shared invariants and exempt set.
- Record content disagreements and other workflows' candidates in Observations
  without proposed fixes.
- Begin repository edits only when the Director relays
  `approved (<run pointer>)`; apply exactly your slice's approved rows as
  written, including reviewer REVISE wording.
- Route harness-denied writes through the
  [full-file staging protocol](SKILL.md#full-file-staging); continue independent
  approved rows.
- Re-verify after application using the selected workflow's obligations.

### Reporting and blockers

Report the written partial artifact with `complete (<run pointer>)` and an
optional one-line summary. After applying and re-verifying the slice, report
`complete (<run pointer>)` with a summary of at most 80 codepoints and no file
lists; the Director recovers file detail through Git.

For unresolved ambiguity, an unsafe row or a hit that may lose coverage, place
`COMMENT(scanner)` at the offending `<file>:<line>` pointer in the artifact,
explain the blocker and send `blocked (<file>:<line>)`. STOP and wait for the
Director instead of assuming. Files outside your slice and the
exempt set stay untouched. If asked to wrap up, send `complete (<run pointer>)`
or the appropriate blocked report.

## Reviewer

Validate the Director's merged run artifact **before any edit lands**. After
application, confirm the diff stays within approved rows and preserves what
the selected workflow guards. Review proposals and their file context;
scanners alone perform scans and apply repository edits. Your writes are
limited to verdict annotations in the run artifact under `${BASE}`.

### Accountability

- Read the merged artifact on the Director's `ready (<run pointer>)`; run every
  Reviewer check for the selected workflow against actual file context at each
  location, rather than judging quoted fragments alone.
- Send `approved (<run pointer>)` only when every check passes. An unresolved
  finding blocks approval: annotate its offending row or `<file>:<line>` with a
  tagged `COMMENT(reviewer)` and send `blocked (<run pointer>)`.
- On the Director's request, perform the workflow's post-apply checks against
  the diff. Send `approved (<run pointer>)` again, or annotate a regression with
  `COMMENT(reviewer)` and send `blocked (<file>:<line>)`.

Keep repository files unchanged; the run artifact's verdict annotations are
your sole write surface. Route blockers and essential unknown prerequisites to
the Director immediately.
