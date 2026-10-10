# Bash Tool — Member Behavior

This rule fires every time you reach for the Bash tool as a CAFleet team member. Read it before invoking Bash, before emitting any text that looks like a command result, and before responding to any "run X" request.

## The MUST rule

> **If you are a CAFleet member spawned by `cafleet member create`, your harness runs in `--permission-mode dontAsk`. Your Bash tool is ENABLED, your broker commands are allowed from spawn, and permission prompts auto-resolve silently. Run every command your harness allows directly via the Bash tool, with no prefix and no operator prompt, and route a command it denies to the Director with `cafleet message send`.**

## How to detect that you are a CAFleet member

Any of the following signals means you are a member subject to this rule:

- Your spawn prompt names a Director / `director_member_id` / refers to you as a "member" / "teammate" of a CAFleet team.
- The status line at the bottom of your pane shows `⏵⏵ don't ask on`.
- Your spawn prompt instructs you to wait for the Director's instructions via `cafleet ... message poll`.

## The owning protocols

- Member-side conduct — run what your harness allows and route the rest, the never-fabricate rules, and where your ids come from: `skills/cafleet/roles/member.md`.
- Command routing — the member-side reconsider-then-route protocol and the Director-side `cafleet member exec` dispatch, completion notice, and targeting boundary: `skills/cafleet/reference/prompt-routing.md`.
- Keystrokes you may see land in your pane: a `cafleet member ping` (`Esc` → `cafleet message poll <your-member-id> — then resume your work if something was still running.` → `Enter`) re-poking a quiet pane, and a `! cafleet member exec-run <exec-id>` line running a command the Director dispatched for you with `cafleet member exec`.
