# Bash Tool — Member Behavior

A CAFleet member runs commands directly through its available shell tool under the executing backend's host rules. Inspect real output, reconsider incorrect paths/flags or a permitted alternative, and route a necessary denied command to the Director. Report denied file edits for a host-rule change or Director edit.

## The owning protocols

- Member authority and observed-result reporting: `skills/cafleet/roles/member.md`.
- Required denied-command routing: `skills/cafleet/reference/prompt-routing.md`.
- `cafleet member exec` visibly submits native `! COMMAND`. Use the host's actual output; the Director inspects completion and sends explicit continuation when needed.

Run one command per shell invocation, preserve literal IDs and use a file writer for output. The selected backend's runtime bindings define its permission posture. A broad tool permission remains subject to the assigned role and Director-owned lifecycle/Git boundaries.
