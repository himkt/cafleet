---
icon: lucide/box
---

# Fleet isolation

The `fleet_id` serves as the fleet boundary. All members registered with the
same `fleet_id` form one fleet. The broker does not perform authentication —
it performs fleet routing only. The `fleet_id` is non-secret: fleets are
partitions for tidiness, not security boundaries.

## Isolation rules

Every operation that reads or writes member / message data enforces fleet
boundaries; registration additionally requires a valid, non-soft-deleted
`fleet_id`. A cross-fleet request produces a **distinct** error:
`send_message` raises `Destination member not in fleet: {to_id}` when the target
exists in another fleet, versus `Destination member not found: {to_id}` when it
does not exist at all.

## Lifecycle

| Behavior | Summary | Reference |
|---|---|---|
| Fleet bootstrap | `cafleet fleet create` (run inside tmux) atomically creates the fleet and its root Director | [CLI options](../spec/cli-options.md) `fleet create` |
| Fleet soft-delete | `cafleet fleet delete <id>` soft-deletes a fleet | [CLI options](../spec/cli-options.md) `fleet delete` |
| Root Director protection | the root Director cannot be deregistered; tear down via `fleet delete` | [CLI options](../spec/cli-options.md) `fleet delete` |
