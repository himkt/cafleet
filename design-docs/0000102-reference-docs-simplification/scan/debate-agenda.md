# Round 2 — DEBATE agenda

All three scans are in. Your three findings files agree on the shape of the fix.
Now resolve the cross-slice questions below by debating each other **directly**
(peer-to-peer cafleet messages), then write a CONSENSUS section into your own
findings file. Push back hard before you converge — do not rubber-stamp.

## Read first
- scan/findings-api.md (scanner-api, agent 409)
- scan/findings-spec.md (scanner-spec, agent 410)
- scan/findings-concepts.md (scanner-concepts, agent 411)

## Questions you MUST converge on (one repo-wide answer each)

1. **The audience/boilerplate preamble.** scanner-api leans "cut it from every
   api page entirely (it belongs at most once on an api section index)";
   scanner-spec leans "replace with a single shared one-liner owned by the
   relevant page." Pick ONE rule and say exactly where the (at most one) surviving
   copy lives. Consider: is an api section landing page worth adding, or do we
   just delete?

2. **Single-source ownership map.** Agree the ONE canonical home for each
   repeated topic, so every other page's pointer is consistent. Proposed map —
   confirm or amend:
   - monitoring model (when/what split, 180s/720s watched set) -> concepts/monitoring.md
   - Esc keystroke mechanics -> concepts/tmux-push.md (monitoring.md states only the wake-nudge exception, one line)
   - per-backend spawn flags / auto-approval -> concepts/coding-agents.md
   - error strings -> spec/cli-options.md Error Messages table
   - fleet-id literal-flag rationale -> spec/cli-options.md "Fleet ID"
   - persisted task columns -> spec/data-model.md #tasks (message-envelope keeps only the rendered projection)
   - --activity column list -> concepts/token-reduction.md table
   - backend "usage from a non-claude pane / ! shortcut / pane-title / overview framing" -> concepts/coding-agents.md (codex.md/opencode.md keep only genuine deltas)

3. **Backend verification recipes** (codex.md / opencode.md copy-paste smoke
   tests). Keep verbatim because runnable, or trim to a pointer? scanner-spec
   flagged this for consensus. Decide.

4. **Repo-wide reference voice/length policy.** Agree one short statement you can
   all sign. Seed (merge your three versions): "A reference page states the
   current surface in tables; prose only where a table cannot carry the meaning.
   Each fact, error, and rationale has exactly one home; every other mention is a
   one-line pointer. No migration/history narration, no promotional
   editorializing, no `##` heading whose body is only a cross-link. Diagrams are
   kept."

## Output of Round 2
Each scanner: append a "## CONSENSUS" section to your own findings file capturing
the agreed answers to 1–4 (and a "## OPEN QUESTIONS FOR USER" section listing any
genuine unresolved disagreement, or "none"). Then report to the Director:
converged (<slice>) — consensus written, M open questions.
