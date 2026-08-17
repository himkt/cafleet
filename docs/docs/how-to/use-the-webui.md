# Use the admin WebUI

The admin WebUI is a browser dashboard for watching and joining a fleet's
message traffic ([Overview](../concepts/overview.md)). It is the only
surface that needs a running server — CLI commands write to SQLite directly
and never require one. Its writes are messages and the monitor controls
(the wake interval and the forced wake); everything else is read-only.

## Start the server

```bash
cafleet server
```

Open `http://127.0.0.1:8000` (press ++ctrl+c++ to stop the server). Host/port
overrides (`--host` / `--port` and the matching env vars) are documented in
[CLI options](../spec/cli-options.md#cafleet-server).

The built UI is embedded in the `cafleet` binary at build time, so the served
dashboard always matches the binary — there is nothing extra to build or
deploy at runtime.

## Pick a fleet

The first load lands on a fleet picker. Selecting a fleet opens the unified
timeline: a sidebar of the fleet's members, a center timeline of unicast and
broadcast messages, and a bottom input.

## Send as the root Director

The bottom input parses `@<member> text` for unicast and `@all text` for
broadcast. Every send goes out as the fleet's root Director — the member
`cafleet fleet create` registered for the fleet — so you never register
yourself as a member to use the dashboard.

## Inspect history

Clicking a member in the sidebar opens its detail panel with Inbox / Sent
tabs. This works for deregistered members too — the WebUI is the only surface
that shows them ([Storage](../concepts/storage.md#no-physical-cleanup)).

## Control the monitor

The header shows a monitor indicator — running or stopped — for the selected
fleet's monitor loop ([Monitoring](../concepts/monitoring.md)). Clicking it
opens a popover with the two monitor controls:

- **Wake interval** — edits the running loop's wake cadence; the change
  takes effect within one scan tick, and `0` disables the scheduled wake
  while the loop keeps running.
- **Wake now** — requests an immediate wake outside the schedule; the loop
  delivers it within one scan tick, even when the interval is `0` or the
  next scheduled wake is not yet due.

Both controls are disabled while the monitor is stopped: the interval is
re-stamped from the CLI/env when the monitor starts, and a wake request
needs a running loop to deliver it.

## API contracts

The `/api/*` request/response shapes behind the dashboard are documented in
[WebUI API](../spec/webui-api.md).
