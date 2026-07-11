---
icon: lucide/monitor
---

# Use the admin WebUI

The admin WebUI is a browser dashboard for watching and joining a fleet's
message traffic ([Overview](../concepts/overview.md)). It is the only
surface that needs a running server — CLI commands write to SQLite directly
and never require one — and it is read/write for messages only.

## Start the server

```bash
cafleet server
```

```
INFO:     Started server process [12345]
INFO:     Uvicorn running on http://127.0.0.1:8000 (Press CTRL+C to quit)
```

Open `http://127.0.0.1:8000`. Host/port overrides (`--host` / `--port` and
the matching env vars) are documented in
[CLI options](../spec/cli-options.md#cafleet-server).

Running from a source checkout instead of an installed wheel? `/` returns
404 until you build the UI with `mise //admin:build` — installed wheels
bundle the built UI.

## Pick a fleet

The first load lands on a fleet picker. Selecting a fleet opens the unified
timeline: a sidebar of the fleet's members, a center timeline of unicast and
broadcast messages, and a bottom input.

## Send as the Administrator

The bottom input parses `@<member> text` for unicast and `@all text` for
broadcast. Every send goes out as the fleet's built-in Administrator —
a write-only registry identity with no tmux pane — so you never register
yourself as a member to use the dashboard.

## Inspect history

Clicking a member in the sidebar opens its detail panel with Inbox / Sent
tabs. This works for deregistered members too — the WebUI is the only surface
that shows them ([Storage](../concepts/storage.md#no-physical-cleanup)).

## API contracts

The `/api/*` request/response shapes behind the dashboard are documented in
[WebUI API](../spec/webui-api.md).
