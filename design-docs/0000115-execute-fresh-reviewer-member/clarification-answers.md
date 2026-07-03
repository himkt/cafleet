User answers (relayed verbatim by the Director):

UNDERSTANDING TO CONFIRM: Correct. Proceed on this basis.

Q1 (Fate of push + PR creation): Keep push + PR. Keep pushing the branch and opening the PR after admin approval; drop only the @copilot review request and the Step 7 Copilot loop.

Q2 (Removal blast radius): Remove everywhere. Purge ALL Copilot machinery from the whole cafleet-design-doc skill family: coordination.md Copilot Routing + copilot marker role, create.md mentions, Copilot commit-message rows in director.md, skill-author SKILL.md reference.

Q3 (Reviewer's review scope): Read-execute. The Reviewer may run the test suite and lint tasks itself to verify claims, in addition to reading the diff and design doc.

Q4 (Loop + disagreement handling): OK as proposed. Marker-based routing, uncapped loop until Reviewer approval, Director arbitration with 3-round limit on disputes.

Q5 (Reviewer lifecycle after approval): Re-review first. Post-admin-feedback revisions go back through the Reviewer for re-approval before re-presenting to the admin — keeps the "Reviewer approves before admin sees it" invariant. The Reviewer stays alive until the Step 8 teardown.

Q6 (Concrete reviewer model values): User's own words: "Update the model list on reference to show the intelligence level of each model. And use best model among models for each coding agent (best on claude code, best on codex. best on opencode)". Interpretation guidance: the per-backend model tables in the reference docs should be annotated with each model's intelligence level, and the new reviewer-model token should resolve to the most intelligent model of each backend's table (claude: the best available on Claude Code; codex: the best available on Codex; opencode: the best available on OpenCode).

Q7 (Monitoring-member delta): Revert to conditional. Remove the execute-only unconditional idle-nudge delta entirely — its reason disappears with Copilot; the canonical conditional nudge suffices.
