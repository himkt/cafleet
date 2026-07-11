---
icon: lucide/code
---

# broker

The data-access layer every CLI command shares: all member,
fleet, and message operations against SQLite live here, and any change to
persisted behavior lands here.

## Package layout

`cafleet.broker` is a package split by domain:

| Submodule | Contents |
|---|---|
| `broker/fleets.py` | fleet CRUD (`create_fleet`, `list_fleets`, `get_fleet`, `delete_fleet`) |
| `broker/members.py` | the member registry — registration + placement (`register_member`, `deregister_member`, `get_member`, `verify_member_fleet`, …) plus the roster + activity proxies (`list_members`) |
| `broker/messaging.py` | `send_message`, `broadcast_message`, `poll_tasks`, `ack_task`, `cancel_task`, `get_task` + inline-preview notification |
| `broker/monitor.py` | monitor runtime, enrollment, and ping records (`find_monitoring_member`, `claim_monitor_runtime`, `heartbeat_monitor_runtime`, `clear_monitor_runtime`, `read_monitor_runtime`, `monitor_is_live`, `monitor_runtime_payload`, `record_pings`, `list_monitor_targets`, `get_monitor_config`, `update_monitor_config`) |
| `broker/skill_installs.py` | skill-install audit rows (`skill_installs_table_exists`, `list_skill_installs`, `record_skill_install`) |
| `broker/_shared.py` | private cross-submodule helpers and the `read_session` / `write_session` context managers |

## Re-export contract

`broker/__init__.py` re-exports the public API of every submodule **except
`skill_installs.py`** with `__all__`: import the package and use attribute
access (`from cafleet import broker` then `broker.send_message(...)`). The
`skill_installs.py` helpers are the deliberate exception — they are imported
directly from the submodule (`from cafleet.broker.skill_installs import …`).
The package attribute is the supported test patch seam; the single DB seam is
`cafleet.broker._shared.get_sync_sessionmaker`.

::: cafleet.broker
