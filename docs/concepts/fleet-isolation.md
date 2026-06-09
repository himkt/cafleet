---
icon: lucide/box
---

# Fleet isolation

The `fleet_id` serves as the fleet boundary. Fleets are created via
`cafleet fleet create`. All agents registered with the same `fleet_id`
form one fleet. The broker does not perform authentication — it performs
fleet routing only.

The `fleet_id` is a non-secret fleet identifier. Fleets are partitions
for tidiness, not security boundaries.

## Registration

Registration requires a valid, non-soft-deleted `fleet_id`. Fleets are
created via `cafleet fleet create` before any members can be spawned.

## Isolation rules

Every operation that reads or writes agent / task data enforces fleet
boundaries. Cross-fleet requests always produce "not found" errors
indistinguishable from the resource not existing.

## Fleet bootstrap

`cafleet fleet create` must be run inside a tmux session and atomically
creates the fleet, its root Director, and the built-in Administrator in one
all-or-nothing transaction — see [CLI options](../spec/cli-options.md)
`fleet create`.

## Fleet soft-delete

`cafleet fleet delete <id>` soft-deletes a fleet — see
[CLI options](../spec/cli-options.md) `fleet delete` for the observable
behavior.

## Root Director protection

The root Director cannot be deregistered; use `cafleet fleet delete` to tear
down a fleet.

## Built-in Administrator agent

Each fleet has exactly one built-in Administrator — see
[data model](../spec/data-model.md) for its definition and protections.
