# Bash Tool — Member Behavior

This rule fires every time you reach for the Bash tool as a CAFleet team member. Read it before invoking Bash, before emitting any text that looks like a command result, and before responding to any "run X" request.

## The MUST rule

> **If you are a CAFleet member spawned by `cafleet member create`, your harness runs in `--permission-mode dontAsk`. Your Bash tool is ENABLED and permission prompts auto-resolve silently. Run cafleet (and any other shell command) directly via the Bash tool. No prefix, no Director routing, no operator prompts.**

## How to detect that you are a CAFleet member

Any of the following signals means you are a member subject to this rule:

- Your spawn prompt names a Director / `director_member_id` / refers to you as a "member" / "teammate" of a CAFleet team.
- The status line at the bottom of your pane shows `⏵⏵ don't ask on`.
- Your spawn prompt instructs you to wait for the Director's instructions via `cafleet ... message poll`.

## The owning protocols

- Member-side conduct — the run-commands-yourself default, the never-fabricate rules, denial handling, and where your ids come from: `skills/cafleet/roles/member.md`.
- The bash-via-Director fallback — the member-side reconsider-then-route protocol and the Director-side `prompt --shell → ping → ack` dispatch, serialization, and lookup boundary: `skills/cafleet/reference/prompt-routing.md`.
- Director keystrokes you may see land in your pane: a `cafleet member ping` (`Esc` → `cafleet message poll <your-member-id> — then resume your work if something was still running.` → `Enter`) re-poking a missed delivery, and a `cafleet member prompt --shell` staging a dispatched command's output for your next turn.
