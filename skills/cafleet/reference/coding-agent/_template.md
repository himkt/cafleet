# Overlay: <backend name>

Apply these deltas on top of the cafleet base. The base states each instruction in backend-neutral terms and points here; this overlay states how `<backend name>` realizes it. Fill every section with this backend's concrete delta. A backend that has "no analog" for a section states that *as* the section's content — never omit a section.

## 1. Decision surface

<How this backend solicits a recorded user reaction — approve / choose / confirm / continue-or-abort. Name this backend's interactive decision-prompt tool if it has one, or state the plain-operator-message-via-Director fallback. A fleet member never talks to the user: it sends its question to the Director, which relays it through this surface. Include the question-shape taxonomy and any pane-relay keystroke frame this backend supports.>

## 2. Monitor model

`--model <cheapest capable model for this backend>` — the value the Director substitutes into the `cafleet member create … --role monitor` spawn command.

## 3. Auto-approval / permission mode

<The exact spawn flags that put this backend in workspace-scoped auto-approval mode: the Bash tool is enabled and routine permission prompts auto-resolve.>

## 4. Background-task + task-list primitives

<This backend's background-run primitive and the matching stop primitive for long-lived work (e.g. the Slidev dev server), plus the task-list primitive used to coordinate parallel sub-work — or, if the backend has none, the cafleet-message coordination fallback.>

## 5. Pane discovery / pane title

`cafleet member list` (the `pane_id` column) is ground truth for locating a member's pane. <Note any `--name`-style pane-title analog this backend sets, or state that it has none.>

## 6. Skill-loading recipe

<The loader this backend uses to load the listed skills at startup — or, if it cannot load skills, the read-the-referenced-files-by-absolute-path fallback using the paths the spawn prompt provides.>
