---
icon: lucide/code
---

# broker

The data-access layer every CLI command and the WebUI share: all agent,
fleet, and message operations against SQLite live here. Read this page to
embed the broker from Python or to change any persisted behavior. Like
every API page, it is for contributors changing cafleet and embedders
driving it from Python; CLI users find the command surface in
[CLI options](../spec/cli-options.md).

## Package layout

`cafleet.broker` is a package split by domain:

| Submodule | Contents |
|---|---|
| `broker/fleets.py` | fleet CRUD (`create_fleet`, `list_fleets`, `get_fleet`, `delete_fleet`) |
| `broker/agents.py` | agent registry + placement (`register_agent`, `deregister_agent`, `verify_agent_fleet`, …) |
| `broker/members.py` | member roster + activity proxies (`list_members`, `list_members_with_activity`) |
| `broker/messaging.py` | `send_message`, `broadcast_message`, `poll_tasks`, `ack_task`, `cancel_task` + inline-preview notification |
| `broker/queries.py` | read-only task queries (`list_inbox`, `list_sent`, `list_timeline`, `get_task`) |
| `broker/_shared.py` | private cross-submodule helpers and the `read_session` / `write_session` context managers |

## Re-export contract

`broker/__init__.py` re-exports the full public API with `__all__`, so
consumers import the package and use attribute access — `from cafleet
import broker` then `broker.send_message(...)` — never a submodule
directly. The package attribute is also the supported patch seam for
tests; the single DB seam is
`cafleet.broker._shared.get_sync_sessionmaker`.

::: cafleet.broker
