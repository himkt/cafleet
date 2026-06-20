---
icon: lucide/box
---

# Fleet isolation

The `fleet_id` serves as the fleet boundary. All agents registered with the
same `fleet_id` form one fleet. The broker does not perform authentication —
it performs fleet routing only. The `fleet_id` is non-secret: fleets are
partitions for tidiness, not security boundaries.

## Isolation rules

Every operation that reads or writes agent / task data enforces fleet
boundaries; registration additionally requires a valid, non-soft-deleted
`fleet_id`. Cross-fleet requests always produce "not found" errors
indistinguishable from the resource not existing.

## Lifecycle

| Behavior | Summary | Reference |
|---|---|---|
| Fleet bootstrap | `cafleet fleet create` (run inside tmux) atomically creates the fleet, its root Director, and the built-in Administrator | [CLI options](../spec/cli-options.md) `fleet create` |
| Fleet soft-delete | `cafleet fleet delete <id>` soft-deletes a fleet | [CLI options](../spec/cli-options.md) `fleet delete` |
| Root Director protection | the root Director cannot be deregistered; tear down via `fleet delete` | [CLI options](../spec/cli-options.md) `fleet delete` |
| Built-in Administrator | each fleet has exactly one built-in Administrator | [data model](../spec/data-model.md) |
