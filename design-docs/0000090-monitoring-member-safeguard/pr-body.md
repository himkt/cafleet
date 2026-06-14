Implements design doc `0000090-monitoring-member-safeguard`.

Closes the `cafleet monitor` safety hole where a bare `Enter` could confirm a pending permission prompt, and narrows the heartbeat from "ping every member" to an `Esc`-safeguarded loop targeting only the Director and one dedicated monitoring member.

- `Esc`-first keystrokes (`send_poll_trigger`, new `send_wake_trigger`); `member ping` inherits it
- Enrollment restricted to Director + monitoring member (`--role monitor`, `cafleet.kind == "monitoring-member"`, one per fleet); ordinary members are never pinged
- `send_resume_trigger` removed entirely
- Loop selects the keystroke by role; `monitor status` labels the monitoring member; Alembic 0003 prunes legacy rows
- Docs/README/SKILLs updated; mise lint/format/typecheck/test green (825 tests)
