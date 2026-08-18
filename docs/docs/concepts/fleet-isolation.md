# Fleet isolation

The `fleet_id` serves as the fleet boundary. All members registered with the
same `fleet_id` form one fleet. The broker does not perform authentication —
it performs fleet routing only. The `fleet_id` is non-secret: fleets are
partitions for tidiness, not security boundaries.

## Isolation rules

The fleet boundary is derived from the subject rows themselves — a member row
names its fleet, so nothing needs to restate it; registration requires a
valid, non-soft-deleted
`fleet_id`. A cross-fleet request produces a **distinct** error:
`send_message` raises `members {from} and {to} are not in the same fleet.`
when sender and recipient
exist in different fleets, versus `Destination member not found: {to_id}`
when the recipient does not exist at all.

## Lifecycle

| Behavior | Summary | Reference |
|---|---|---|
| Fleet bootstrap | `cafleet fleet create` (run inside a tmux or herdr session) atomically creates the fleet, its root Director, and its monitor member | [CLI options](../spec/cli-options.md) `fleet create` |
| Fleet soft-delete | `cafleet fleet delete <id>` soft-deletes a fleet | [CLI options](../spec/cli-options.md) `fleet delete` |
| Root Director protection | the root Director cannot be deregistered; tear down via `fleet delete` | [CLI options](../spec/cli-options.md) `fleet delete` |
