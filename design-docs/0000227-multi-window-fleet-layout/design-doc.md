# Multi-window fleet layout

**Status**: Approved
**Progress**: 59/64 tasks complete
**Last Updated**: 2026-11-13

## Overview

A fleet spreads its member panes across several multiplexer windows (a tmux window or a herdr tab, both called "window"). Every `member create` takes the first free slot in a height-derived column layout, opening a named overflow window when every column is full. Each member's window is recorded in the registry and shown by `member create`, `member list` and `member show`.

## Success Criteria

- [ ] With the Director window full, the next `member create` opens a detached overflow window named `cafleet-<fleet_id>-<ordinal>` without stealing focus, on both tmux and herdr.
- [ ] Column capacity is `min(8, floor(window_height / 30))` and is evaluated at spawn time; existing members never move to another window.
- [ ] The Director window keeps the Director alone in its left half and stacks members in the right column; overflow windows fill the left column top to bottom, then the right column.
- [ ] After every member create and member delete, each column of the affected window has equal pane heights (tmux `select-layout -E`, herdr arithmetic equalization per column), best effort.
- [ ] Overflow-window ordinals only increase within a fleet; an overflow window closes when its last member leaves.
- [ ] Member panes carry the member name as their pane title (tmux) / pane label (herdr), best effort.
- [ ] `member list` text shows a `window` column; `member create` and `member show` text append `window=<name>`; the JSON `placement` object carries `mux_window_name`, `window_ordinal` and `column_side`.
- [ ] Concurrent `member create` and `member delete` calls in one fleet never choose the same slot, open two windows for one ordinal, or target a pane another operation is closing.
- [ ] An overflow window that can no longer be read fails `member create` with an error naming the window, its active members and the remedy.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass; the manual live checks in Step 13 pass on tmux 3.6a and herdr 0.9.5.

---

## Background

Today `member create` splits every member into the Director's window. tmux splits the Director window and reflows it with `select-layout main-vertical` (`src/multiplexer/tmux.rs`); herdr emulates main-vertical by splitting the Director rightward once, then splitting the highest-sorting member pane downward and equalizing the single right column (`src/multiplexer/herdr.rs`, SPEC §6.5 herdr column equalization). A large fleet therefore shrinks every pane until coding-agent TUIs become unusable.

The pending placement row records the Director's window at registration (`src/cli/member.rs`), and publication patches only `mux_pane_id` after the runner's `prepared` handshake (`src/runtime/lifecycle.rs`). `member list` and `member show` are registry reads with no multiplexer requirement, so any window name they show must be stored. Deletion (`end_member` in `src/cli/fleet.rs`) shuts the runner down, closes the pane, deregisters, then runs the optional layout restoration.

### Verified multiplexer behavior

Probed on isolated servers (`tmux -L cafleet-probe`, `herdr --session cafleet-probe`, and the Reviewer's `herdr --session rvprobe`), all removed afterwards.

| Behavior | tmux 3.6a | herdr 0.9.5 |
|---|---|---|
| Per-column equalization | `select-layout -E -t <pane>` spreads only that pane's column evenly; the other column is untouched | No reflow command; arithmetic `pane resize` per column (existing algorithm) |
| Splitting inside a column | `split-window -v -t <any pane of the column>` inserts into that column, even from a middle pane | `pane split --direction down` nests below the target pane; splitting the bottom pane keeps a right-leaning chain |
| Full-height column split | `split-window -h -f -l 50%` (right), `-h -f -b -l 50%` (left) | None: `pane split --direction right` splits only the target pane |
| Re-parenting a pane inside a tab | Not needed | `pane move` within one tab is a no-op (`"changed": false, "reason": "same_tab"`); a round trip through a temporary tab (`--new-tab`, then `--tab <tab> --split down\|right --target-pane <p>`) re-parents it, keeps its pane ID and terminal, and the emptied temporary tab closes itself (`closed_tab_id`) |
| Focus preservation | `new-window -d`, `split-window -d` | `tab create --no-focus`, `pane split --no-focus` and `pane move --no-focus` keep the focused tab; `pane swap` steals tab focus and is not used |
| New window | `new-window -d -n NAME -c CWD -P -F "#{window_id}\|#{pane_id}"`; `-n` turns `automatic-rename` off for that window only | `tab create --workspace W --cwd P --label NAME --no-focus` returns `result.tab.tab_id` and `result.root_pane.pane_id` |
| Window height | `display-message -p -t <window> "#{window_height}"`; a missing window prints an empty line and exits 0 | `pane layout --pane <p>` → `result.layout.area.height` |
| Window name read | `display-message -p -t <window> "#{window_name}"` | `tab get <tab>` → `result.tab.label` |
| Listing a window's panes | `list-panes -t <window> -F "#{pane_id}\|#{pane_left}"`; a missing window exits 1 with stderr `can't find window: <window>` | `pane layout --pane <p>` |
| Pane naming | `select-pane -t <pane> -T <title>` (a program may retitle it by escape sequence; visible when `pane-border-status` is on) | `pane rename <pane> <label>` sets `result.pane.label`, independent of `terminal_title` |
| Last pane leaves | The window closes natively | `pane close` of a tab's last pane closes the tab natively; `tab get <closed tab>` exits 1 with error code `tab_not_found` |
| A whole column closes | The other column widens to full width | The surviving column spans the full width with `splits: []` (no residual `right` split) |

---

## Specification

### Terms

| Term | Meaning |
|---|---|
| Window | A tmux window or a herdr tab. |
| Director window | The window recorded in the Director's placement at `fleet create`; ordinal `0`. |
| Overflow window | A window CAFleet opens for the fleet; ordinal `1` or greater. |
| Column | The `left` or `right` half of a window. The Director window's left column holds only the Director. |
| Slot | One member position in a column. |

### Constants

```rust
pub(crate) const MIN_PANE_HEIGHT: i64 = 30;
pub(crate) const MAX_PANES_PER_COLUMN: i64 = 8;
pub(crate) const LAYOUT_LOCK_WAIT: Duration = Duration::from_secs(60);
```

All three are fixed; no flag or configuration changes them. The capacity of a column in a window whose live height is `H` rows is:

```
capacity(H) = min(MAX_PANES_PER_COLUMN, floor(H / MIN_PANE_HEIGHT))
```

| Window height `H` | Capacity per column | Director window members | Overflow window members |
|---|---|---|---|
| < 30 | 0 | 0 (skipped) | 2 (one per column, by the overflow minimum) |
| 50 | 1 | 1 | 2 |
| 60 | 2 | 2 | 4 |
| 120 | 4 | 4 | 8 |
| ≥ 240 | 8 | 8 | 16 |

An overflow column always accepts at least one member (`max(1, capacity(H))`), so a new overflow window always makes progress. The Director window's right column uses `capacity(H)` unchanged and is skipped when it is 0. `H` is the tmux `#{window_height}` (which excludes the status line) or the herdr `layout.area.height`; the 30-row minimum is a per-slot budget that includes the tmux pane separator.

### Slot selection

Slot selection is a pure function in the new `src/multiplexer/layout.rs`:

```rust
pub(crate) enum ColumnSide { Left, Right }

pub(crate) struct WindowOccupancy {
    pub ordinal: i64,            // 0 = Director window
    pub window_id: String,
    pub window_name: String,
    pub height: i64,             // live rows
    pub left: Vec<String>,       // concrete member pane IDs, registry order
    pub right: Vec<String>,
    pub director_pane: Option<String>, // Some only for ordinal 0
}

pub(crate) enum SlotChoice {
    Existing { ordinal: i64, window_id: String, window_name: String, side: ColumnSide },
    NewWindow { ordinal: i64, window_name: String },
}

pub(crate) fn choose_slot(
    fleet_id: i64,
    windows: &[WindowOccupancy],   // sorted by ordinal ascending; windows[0].ordinal == 0
    last_window_ordinal: i64,
) -> SlotChoice
```

1. Scan windows by ascending ordinal. For ordinal 0, consider only the right column with capacity `capacity(height)`. For each overflow window, consider the left column, then the right column, each with capacity `max(1, capacity(height))`.
2. Return the first column whose member count is below its capacity (`Existing`).
3. Otherwise return `NewWindow { ordinal: last_window_ordinal + 1, window_name: "cafleet-<fleet_id>-<ordinal>" }`, whose first member takes the left column; the overflow minimum makes that slot always available, so a new window needs no height read.

`choose_slot` asserts that `windows[0].ordinal == 0` and that ordinals are strictly ascending, panicking on a violated invariant.

Occupancy comes from the registry: active members of the fleet whose placement is concrete (`mux_pane_id IS NOT NULL`), grouped by `window_ordinal` and `column_side`. The Director's own row (ordinal 0, `left`) is excluded from counts. A pending placement holds no slot: every creation holds the fleet layout lock until it completes or compensates, so a pending row seen under the lock belongs to a killed creation (§6.5 Termination during pane input), which `member delete` removes. An overflow ordinal with no concrete member is a closed window and is not scanned.

`height` is read live for each scanned window before registration through `window_height(window_id, anchors)`. The anchors are the Director pane for ordinal 0, and every concrete member pane of the window in registry order for an overflow window. tmux reads by window ID and ignores the anchors. herdr tries each anchor in order with `pane layout --pane <anchor>` and returns the first successful height. A `pane_not_found` reply, or another command failure, moves on to the next anchor, so one member whose pane died does not make a live tab unreadable. The read fails only when no anchor answers, carrying the last anchor's error as its cause.

- A failed Director-window read propagates the multiplexer error unchanged and registers nothing.
- A failed or unparsable overflow-window read (the user closed the window, or every member pane in it died while the members stayed registered) fails `member create`, registers nothing, and names the window, its stale members and the remedy: `Error: window <window-name> (ordinal <ordinal>) of fleet <fleet-id> cannot be read: <cause>. Its active members <member-ids> may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.` `<member-ids>` lists the window's active member IDs ascending, separated by `, `.

Policy decisions:

- **First free slot.** A slot freed by `member delete` is reused by the next creation, so a newer member can land in the Director window while overflow windows hold older members.
- **No migration.** Existing members never change window or column; capacity changes from a resized terminal affect only later placements. herdr re-parenting (below) moves a pane only visually inside its own window, keeping its pane ID, window and column.
- **Ordinals only increase.** `fleets.last_window_ordinal` is incremented in the registration transaction of every `NewWindow` choice and is never decremented, even when that creation later fails; a fleet's overflow windows can therefore skip a number.
- **Emptied overflow windows close.** When the last member of an overflow window is deleted the window closes natively on both backends. The Director window is never closed.

### Data model

`assets/migrations/V1__baseline.sql` is edited in place (`.claude/rules/code-quality.md` § Database schema); every version starts from a fresh database, so no migration exists.

```sql
CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT,
    last_window_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(last_window_ordinal >= 0)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT,
    mux_window_name TEXT,
    window_ordinal INTEGER NOT NULL CHECK(window_ordinal >= 0),
    column_side TEXT NOT NULL CHECK(column_side IN ('left','right')),
    mux_pane_id TEXT,
    backend TEXT NOT NULL,
    mux_server TEXT NOT NULL,
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK((mux_pane_id IS NULL) = (mux_window_id IS NULL)
          AND (mux_window_id IS NULL) = (mux_window_name IS NULL))
);
```

| Column | Semantics |
|---|---|
| `fleets.last_window_ordinal` | Highest overflow ordinal ever allocated for the fleet; `0` until the first overflow window. |
| `member_placements.mux_window_id` | Window of the concrete pane. Null exactly while creation is pending, because an overflow window's ID exists only after its creation. |
| `member_placements.mux_window_name` | Display name: `cafleet-<fleet_id>-<ordinal>` for an overflow window, the Director window's name read at `fleet create` for ordinal 0. Null exactly while pending. |
| `member_placements.window_ordinal` | Chosen at registration; `0` for the Director and Director-window members. |
| `member_placements.column_side` | Chosen at registration; `left` for the Director row, `right` for every Director-window member. |

Code changes that follow:

- `Placement` (`src/broker/mod.rs`) gains `mux_window_name: Option<String>`, `window_ordinal: i64`, `column_side: ColumnSide`; `mux_window_id` becomes `Option<String>`. `PLACEMENT_COLUMNS` and every `RETURNING`/`SELECT` list that projects placement columns add the new columns. A stored `column_side` outside the enum fails with `CafleetError::InvalidStoredValue`.
- `NewPlacement` (`src/broker/members.rs`) gains `window_ordinal` and `column_side` and drops `mux_window_id`; the Director bootstrap passes its window ID and name through a Director-specific constructor.
- Every construction of `Placement` or `NewPlacement`, production and test alike, supplies the new fields; every consumer of `PLACEMENT_COLUMNS` decodes them.
- Consumers that need a concrete window (both backends' `check_key_target`, fleet-delete revalidation in `src/broker/fleets.rs`, deletion's `close_pane`) read `mux_window_id` through `.expect("a concrete placement records its window")` after the existing concrete-pane check; the pane-lock key is unchanged (it does not include the window).
- Publication (`src/runtime/lifecycle.rs`) becomes `UPDATE member_placements SET mux_pane_id=?1, mux_window_id=?2, mux_window_name=?3 WHERE member_id=?4 AND mux_pane_id IS NULL`, still requiring exactly one changed row.

### Fleet layout lock

A fleet-scoped OS file lock serializes every operation that creates or closes a member pane, in the same family as the pane input lock (`src/pane_lock.rs`):

- Path: `<canonical database path>.layout-locks/<sha256_hex(fleet_id as decimal)>.lock`, created `0600`, directory created on demand. The OS releases it when its holder exits, including after `SIGKILL`.
- Acquisition retries every 50 ms within its own `LAYOUT_LOCK_WAIT` budget (60 seconds), separate from each command's operation deadline.
- **`member create`** acquires it after prompt validation and before resolving the Director placement. The 10-second creation deadline is captured after acquisition, so the native version probe, spawn, `prepared` handshake, publication and `start` keep their full budget. It holds the lock until creation returns: through `started`, or through compensation. Expiry exits 1 with `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not created.` and registers nothing.
- **`member delete`** acquires it after its pane lock and before any cleanup step (Hermes settlement, runner shutdown, pane close), and holds it through deregistration and layout restoration. The wait extends the deletion's operation deadline by its duration, as Hermes settlement does. Expiry exits 1 with `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not deleted.`, leaving every state unchanged.
- **`fleet delete`** acquires it once after all its pane locks and before the first member's cleanup, holding it through the last member's restoration, with the same deadline extension. Expiry exits 1 with `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; fleet was not deleted.`, leaving every state unchanged. `end_member` takes the held layout lock as an argument instead of acquiring it.

Lock order is always pane lock → layout lock. `member create` takes no pane lock, and a runner's own delivery takes only its pane lock, so no cycle exists.

Why creation holds the lock until it completes rather than releasing it right after the spawn:

1. Before publication the new pane is invisible to other creations: occupancy counts only concrete placements, and the pane ID is published only after the `prepared` handshake (§6.9 Runner boundary and creation transaction). A second creation in that window would miscount the column and, for an empty column, repeat the full-height split.
2. After publication, `start` can still fail, and its compensation closes the published pane. Without the lock, a concurrent creation may already target that pane (a tmux split anchor, a herdr bottom pane or a re-parenting move). Compensation would then have to re-acquire the lock, and an expired re-acquisition would leave an orphan pane or race the other creation. The `start` phase is short, so holding the lock through it costs little.

A creation holds the lock for at most its 10-second deadline plus compensation, so the 60-second wait admits several queued creations, such as a Director spawning a team back to back.

### Multiplexer interface

`MultiplexerContext` and the shared pane-context read stay unchanged, so `cafleet doctor` keeps its argv and failure modes. `fleet create` alone reads the Director window name, through a separate `window_name(window_id)` call made after the pane context succeeds:

| Backend | Window-name read | Failure |
|---|---|---|
| tmux | `display-message -p -t <window_id> "#{window_id}\|#{window_name}"`, split on the first `\|` so a name containing `\|` survives; an empty name is valid | A missing `\|` or a first field other than `<window_id>` (tmux prints an empty line for a missing window): `unexpected tmux display-message output: <quoted>` |
| herdr | `tab get <tab_id>` → `result.tab.label` | The command failure propagates (`tab_not_found`); a missing label fails with `herdr tab get missing 'label' field` |

A failed read fails `fleet create` exactly as a failed pane-context read does today, registering nothing. On success `fleet create` records the name as the Director row's `mux_window_name`.

`Multiplexer::split_window(reference, command) -> String` is replaced by:

```rust
pub(crate) struct ColumnTarget {
    pub window_id: String,
    pub side: ColumnSide,
    pub column_panes: Vec<String>,   // concrete member panes already in this column, registry order
    pub other_panes: Vec<String>,    // panes of the opposite column (the Director pane for ordinal 0)
}

pub(crate) enum PaneTarget {
    Column(ColumnTarget),
    NewWindow { session: String, window_name: String },
}

pub(crate) struct SpawnedPane { pub pane_id: String, pub window_id: String }

fn spawn_member_pane(
    &self,
    target: &PaneTarget,
    pane_name: &str,          // the member name
    cwd: &str,
    command: &[String],
) -> Result<SpawnedPane, MultiplexerError>;

fn window_height(&self, window_id: &str, anchors: &[String]) -> Result<i64, MultiplexerError>;
fn window_name(&self, window_id: &str) -> Result<String, MultiplexerError>;

fn kill_pane(&self, target_pane_id: &str, window_id: &str) -> Result<(), MultiplexerError>;
fn close_pane(&self, target_pane_id: &str, window_id: &str) -> Result<(), MultiplexerError>;
fn restore_after_close(&self) -> Result<(), MultiplexerError>;
```

Every close receives the window ID from its caller, so neither backend reads the pre-close window from the multiplexer; herdr's pre-close `pane get` read and its `closed_tab` field are removed.

| Close path | Call | Window ID source |
|---|---|---|
| Deletion (`DeletionHost::close_pane` in `src/cli/fleet.rs`) | `close_pane`, then `restore_after_close` after registry finalization | `placement.mux_window_id` |
| Creation compensation (`LaunchHost::close_pane`, `NativeLaunchHost` in `src/runtime/lifecycle.rs`) | `kill_pane`: close, then restore that window | The `SpawnedPane.window_id` returned by `split_runner`; `LaunchHost::close_pane` takes the `SpawnedPane` instead of a bare pane ID |
| Owned-pane rollback inside `spawn_member_pane` | `kill_pane` | The Column target's `window_id`, or the window ID parsed from the `new-window` / `tab create` reply |

`close_pane` closes the pane and records `window_id`; `restore_after_close` equalizes every column of the recorded window (§ tmux backend, § herdr backend). `kill_pane` runs the same close and restoration in one call; its restoration is best effort, and the close keeps today's pane-gone tolerance.

`window_height` parses strictly: tmux output that is not a positive integer (including the empty line a missing window prints) fails with `unexpected tmux display-message output: <quoted>`; herdr fails with `herdr pane layout missing 'area' field` or propagates the command error (`pane_not_found`).

`NativeLaunchHost::split_runner` and every other `LaunchHost` implementation return `SpawnedPane`, and `launch_registered` publishes both IDs plus the chosen window name. The pane-cleanup contract (§6.5 Pane ownership during creation) applies unchanged: a failed or malformed creation response is `PaneCleanup::Unknown`; once a pane ID is extracted the backend owns it and closes it on a later failure inside `spawn_member_pane`. Pane naming, equalization and herdr re-parenting are best effort: their failures never fail the spawn.

### tmux backend

Every argv starts with `tmux -S <mux_server>`.

| Target | Creation argv |
|---|---|
| Column, `column_panes` non-empty | `split-window -v -d -t <last of column_panes> -P -F "#{pane_id}" <command>...` |
| Column `right`, empty | `split-window -h -f -l 50% -d -t <first of other_panes> -P -F "#{pane_id}" <command>...` |
| Column `left`, empty (an overflow window whose left column emptied) | `split-window -h -f -b -l 50% -d -t <first of other_panes> -P -F "#{pane_id}" <command>...` |
| NewWindow | `new-window -d -t <session>: -n <window_name> -c <cwd> -P -F "#{window_id}\|#{pane_id}" <command>...` |

The window ID of a Column target is `target.window_id`; a NewWindow reply splits on the first `|` and requires both fields non-empty (`unexpected tmux new-window output: <quoted>` with `PaneCleanup::Unknown` otherwise). A split inherits the splitting pane's cwd as today; `new-window` receives `-c <cwd>` explicitly. `select-layout main-vertical` is removed.

After creation, in order: `select-pane -t <new pane> -T <member name>`, then equalize every column of the window.

**Equalize every column** (after creation and in `restore_after_close`): `list-panes -t <window> -F "#{pane_id}|#{pane_left}"`, group the panes by `pane_left`, and run `select-layout -E -t <first pane of the group>` for each group of 2 or more panes. A `list-panes` failure whose stderr contains `can't find window` (the closed pane was the window's last) means nothing to restore. Every step is best effort.

### herdr backend

Every argv starts with `herdr --session <mux_server>`. Every split, move and tab creation passes `--no-focus`; `pane swap` is never used. Column order (`L1..Lk`, `R1..Rm`, the bottom pane) always comes from a live `pane layout` read: the panes at the column's `rect.x`, sorted by `rect.y`. `ColumnTarget.column_panes` and `other_panes` only supply the anchor for that read.

| Target | Creation |
|---|---|
| Column, `column_panes` non-empty | Read `pane layout --pane <first of column_panes>`; `pane split <bottom pane of the column> --direction down --no-focus --cwd <cwd>` |
| Column `right`, empty, the other column holds 1 pane | `pane split <that pane> --direction right --no-focus --cwd <cwd>` (the Director window's first member, today's behavior) |
| Column `right`, empty, the other column holds 2 or more panes | `pane split L1 --direction right --no-focus --cwd <cwd>`, then Right re-parenting |
| Column `left`, empty | `pane split R1 --direction right --no-focus --cwd <cwd>` → `X`, then Left re-parenting |
| NewWindow | `tab create --workspace <session> --cwd <cwd> --label <window_name> --no-focus`; the root pane is the member pane |

After creation, in order:

1. `pane run <new pane> "<shell-quoted command>"`. On failure the backend closes the new pane (owned-pane rollback, as today) and returns the error; no re-parenting has run yet, so every other pane stays where it was.
2. `pane rename <new pane> <member name>`, best effort.
3. Re-parenting, when the target row requires it, best effort.
4. Equalize every column of the tab, best effort.

The member's agent starts at the pre-re-parenting size and receives a terminal resize when its pane moves or is equalized.

**Right re-parenting** (left column `L1..Lk`, k ≥ 2; the new pane `R` was split off `L1`). The tab is `down(right(L1, R), L2..Lk)`; the left chain is rebuilt under `L1`:

1. `pane move L2 --new-tab --no-focus` → temporary tab `T'`; for `i = 3..k`: `pane move Li --tab T' --split down --target-pane L(i-1) --no-focus`.
2. For `i = 2..k`: `pane move Li --tab <tab> --split down --target-pane L(i-1) --no-focus`. The last move empties `T'`, which herdr closes.

**Left re-parenting** (right column `R1..Rm`, m ≥ 1; the new pane `X` was split off `R1`). The tab is `down(right(R1, X), R2..Rm)`; the right chain moves to the right of `X` through one temporary tab:

1. `pane move R1 --new-tab --no-focus` → temporary tab `T'`; the tab is now `down(X, R2..Rm)`.
2. For `i = 2..m`: `pane move Ri --tab T' --split down --target-pane R(i-1) --no-focus`; the tab now holds `X` alone.
3. `pane move R1 --tab <tab> --split right --target-pane X --no-focus` (`X` left, `R1` right).
4. For `i = 2..m`: `pane move Ri --tab <tab> --split down --target-pane R(i-1) --no-focus`. The last move empties `T'`, which herdr closes (for m = 1, step 3 empties it).

Each move keeps the pane ID and terminal within one workspace. A failed move leaves the remaining panes where they are, emits the stderr diagnostic `layout re-parenting incomplete: <cause>`, and the spawn continues. While a pane sits in `T'`, a concurrent `member press` to it fails its `check_key_target` comparison with the recorded `mux_window_id` (`placement mismatch`); wrapped delivery revalidates only the registry placement, which re-parenting leaves unchanged.

**Equalize every column** generalizes § herdr column equalization from one right column to every column of a tab: group `layout.panes` by `rect.x`, sort each group by `rect.y`, and for each group of N ≥ 2 select the `down` splits whose `rect.x` and `rect.width` equal the group's, driving the right-leaning chain to ratios `1/(N-k)` with the existing arithmetic. A group whose `down` split count is not `N - 1` is left untouched. The Director's single-pane column has no group of 2, so no rule needs to identify the Director tab.

**Delete-time restoration** for the recorded window (anchor = first surviving pane in `pane list` whose `tab_id` equals it):

| Tab state after close | Action |
|---|---|
| No surviving pane | Nothing to do: herdr closed the tab with its last pane |
| One pane with exactly one residual `right` split | Restore that pane to full width (the existing Director rule) |
| Otherwise | Equalize every column |

### Window and pane naming

| Object | Name | Mechanism |
|---|---|---|
| Overflow window | `cafleet-<fleet_id>-<ordinal>` (e.g. `cafleet-3-1`) | tmux `new-window -n` (also turns `automatic-rename` off for that window); herdr `tab create --label` |
| Director window | Unchanged; its current name is recorded at `fleet create` | `window_name`, read by `fleet create` only (§ Multiplexer interface) |
| Member pane | The member name | tmux `select-pane -T`; herdr `pane rename` |

The recorded Director window name is a snapshot; renaming the window later does not update the registry. A tmux pane title is visible only when the user enables `pane-border-status`, and a coding agent may retitle its pane (Claude's and pi's `--name` already set a title). A herdr pane label is independent of the agent's terminal title. Naming failures never fail a spawn.

### `member create` flow

1. Validate arguments, backend, policy, evidence and prompt (unchanged).
2. Acquire the fleet layout lock within `LAYOUT_LOCK_WAIT`, then capture the 10-second creation deadline.
3. Read occupancy from the registry and each scanned window's live height; call `choose_slot`.
4. Register the member, pending placement (`window_ordinal`, `column_side`; null window ID/name/pane) and run in one transaction. For `NewWindow`, the same transaction runs `UPDATE fleets SET last_window_ordinal=?1 WHERE fleet_id=?2 AND last_window_ordinal=?3` with the new and read values. The layout lock makes a mismatch impossible, so a changed-row count other than 1 is an invariant violation: the transaction rolls back and creation fails with `fleet <fleet-id> window ordinal changed under the layout lock`.
5. Build the `PaneTarget` from the slot and spawn through `NativeLaunchHost`.
6. Publish pane ID, window ID and window name; continue the runner handshake and `start` as today.
7. Release the layout lock when creation returns, after `started` or after compensation.

Compensation order (§6.3 Creation failure compensation) is unchanged; closing an overflow window's only pane closes that window.

### Output

| Surface | Change |
|---|---|
| `member create` text | `ID NAME backend=AGENT pane=PANE window=WINDOW` plus the runtime line; `WINDOW` is `(pending)` while pending |
| `member list` text | A `window` column between `backend` and `pane_id`: `mux_window_name` left-aligned and padded to 13 columns like `name`, or `(pending)` |
| `member show` text | `ID NAME STATUS window=WINDOW`, then the runtime line for an ordinary member |
| JSON `placement` | Ordered keys `backend, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side, mux_pane_id, coding_agent, created_at`; `mux_window_id` and `mux_window_name` are null exactly when `mux_pane_id` is null |

`member list` text, with the row format `"  {:<9}  {:<13}  {:<8}  {backend:<8}  {window:<13}  {pane:<7}  {}"`:

```
2 members:
  member_id  name           kind      backend   window         pane_id  idle
  8          director       director  claude    main           %3       12s
  9          drafter        member    claude    cafleet-3-1    %12      5s
```

### Error strings

| Command | Condition | Error |
|---|---|---|
| `member create` | The fleet layout lock stays busy for `LAYOUT_LOCK_WAIT` | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not created.` |
| `member create` | An overflow window's height read fails or is unparsable | `Error: window <window-name> (ordinal <ordinal>) of fleet <fleet-id> cannot be read: <cause>. Its active members <member-ids> may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.` |
| `member create` | tmux `new-window` reply malformed | `unexpected tmux new-window output: <quoted>` (with `pane ID unknown; pane cleanup unconfirmed`) |
| `member create` | herdr `tab create` reply without `result.root_pane.pane_id` or `result.tab.tab_id` | `herdr tab create missing '<key>' field` (with `pane ID unknown; pane cleanup unconfirmed`) |
| `member create` | herdr re-parenting step failed (stderr diagnostic, exit unaffected) | `layout re-parenting incomplete: <cause>` |
| `member delete` | The fleet layout lock stays busy for `LAYOUT_LOCK_WAIT` | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not deleted.` |
| `fleet delete` | The fleet layout lock stays busy for `LAYOUT_LOCK_WAIT` | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; fleet was not deleted.` |
| `fleet create` | tmux window-name reply malformed or naming another window | `unexpected tmux display-message output: <quoted>` |
| `fleet create` | herdr `tab get` reply without a label | `herdr tab get missing 'label' field` |

### Out of scope

A user-configurable column bound or minimum height, a `member create --window` target, moving live members between windows, renaming windows after creation, and re-layout of windows the user rearranged by hand.

### Documentation surfaces

| Target | Change |
|---|---|
| `SPEC.md` §5.5 | Pending placement: `mux_window_id` and `mux_window_name` are null with `mux_pane_id` |
| `SPEC.md` §6.2 Members | Slot occupancy semantics and ordinal allocation |
| `SPEC.md` §6.3 `fleet create` | Reads the Director window name after the pane context and records it; its failure contract. `cafleet doctor` is unchanged |
| `SPEC.md` §6.3 Output shapes, Member list output, `member create`, `member show`, `member delete`, `fleet delete`, Error Messages | Output columns/keys, layout-lock acquisition, waits and busy outcomes, the unreadable-window error |
| `SPEC.md` §6.5 | Backend matrix, Multiplexer context (the split reference becomes the slot's `PaneTarget`), Backend operations rows (Window name, Split → Spawn member pane, Window height, Close), new `#### Fleet window layout` section (terms, constants, capacity table, slot selection, naming, layout lock), herdr column equalization generalized, herdr re-parenting, Delete-time pane layout |
| `SPEC.md` §6.5 Termination during pane input | `member create` span from layout-lock acquisition through completion or compensation; `member delete` / `fleet delete` spans include the layout lock |
| `SPEC.md` §6.7 Backend capabilities | CAFleet names every member pane on the multiplexer; the "Sets the pane title" cells point at `member list` `window` and `pane_id` |
| `SPEC.md` §6.9 Runner boundary and creation transaction | Publication patches window ID and name with the pane |
| `SPEC.md` §8 | `fleets` and `member_placements` DDL and the placement semantics row |
| `docs/docs/concepts/members.md` | New "Pane layout" section: Director window, overflow windows, capacity in user terms (30-row minimum, 8 per column, consequences for small terminals), naming, `window` in `member list`, the unreadable-window remedy; link to `SPEC.md` § Fleet window layout |
| `docs/docs/concepts/overview.md` | Core terms: `placement` row mentions the window; add a `window` term row |
| `docs/docs/concepts/coding-agents.md` | Replace "Only claude and pi panes display the member name …" with: herdr labels every member pane with its name, tmux titles it (an agent may retitle it), and `member list` shows each member's `window` and `pane_id` |
| `README.md` | No change expected; confirm the pitch still matches `docs/docs/index.md` |
| `assets/skills/cafleet/reference/cli.md` | `fleet create` (window name, failure), `member create` output line, `member list` / `member show` fields, `member delete` / `fleet delete` busy outcomes, split-reference sentence, new error rows |
| `assets/skills/cafleet/roles/director.md` | "Discover panes through `member list`'s recorded `pane_id`" also names `window` |
| `assets/skills/cafleet/reference/coding-agents.md` | No change: `{pane_title}` describes each agent's own title mechanism, which is unchanged |
| `.claude/rules/` | No change |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] Add `#### Fleet window layout` to §6.5 with terms, constants, capacity formula and table, slot selection, the unreadable-window error, policy decisions, naming and the fleet layout lock with its hold rationale <!-- completed: 2026-11-13T14:07 -->
- [x] Update §6.5 Backend matrix, Multiplexer context, Backend operations rows, Delete-time pane layout, and generalize herdr column equalization to every column with the re-parenting procedures <!-- completed: 2026-11-13T14:07 -->
- [x] Update §6.5 Termination during pane input spans for the layout lock <!-- completed: 2026-11-13T14:07 -->
- [x] Update §5.5, §6.2 Members and §6.9 Runner boundary and creation transaction for nullable window fields, occupancy and publication <!-- completed: 2026-11-13T14:07 -->
- [x] Update §6.3 `fleet create`, Output shapes, Member list output, `member create`, `member show`, `member delete`, `fleet delete` and Error Messages <!-- completed: 2026-11-13T14:07 -->
- [x] Update §6.7 Backend capabilities for multiplexer pane naming <!-- completed: 2026-11-13T14:07 -->
- [x] Update §8 DDL and the `member_placements` / `fleets` semantics rows <!-- completed: 2026-11-13T14:07 -->

### Step 2: User documentation

- [x] Add the "Pane layout" section to `docs/docs/concepts/members.md` <!-- completed: 2026-11-13T14:09 -->
- [x] Update the Core terms table in `docs/docs/concepts/overview.md` <!-- completed: 2026-11-13T14:09 -->
- [x] Update the pane-name sentence in `docs/docs/concepts/coding-agents.md` <!-- completed: 2026-11-13T14:09 -->
- [x] Confirm `README.md` needs no change <!-- completed: 2026-11-13T14:09 -->

### Step 3: Skills

- [x] Update `assets/skills/cafleet/reference/cli.md` (`fleet create`, `member create` output, `member list` / `member show`, `member delete` / `fleet delete` busy outcomes, split reference, error rows) <!-- completed: 2026-11-13T14:09 -->
- [x] Update the pane-discovery sentence in `assets/skills/cafleet/roles/director.md` <!-- completed: 2026-11-13T14:09 -->

### Step 4: Schema and registry

- [x] Edit `assets/migrations/V1__baseline.sql`: `fleets.last_window_ordinal`, nullable `mux_window_id`, `mux_window_name`, `window_ordinal`, `column_side`, the nullability CHECK <!-- completed: 2026-11-13T14:30 -->
- [x] Update `Placement`, `PLACEMENT_COLUMNS`, row decoding (typed `ColumnSide`, `InvalidStoredValue` on a bad value) in `src/broker/mod.rs`, and every `PLACEMENT_COLUMNS` consumer <!-- completed: 2026-11-13T14:30 -->
- [x] Update `NewPlacement`, `insert_placement_row` and `RETURNING` lists in `src/broker/members.rs`; add an occupancy query returning concrete placements by ordinal and side, and the active members of one ordinal <!-- completed: 2026-11-13T14:30 -->
- [x] Update every `Placement` / `NewPlacement` construction, production and test alike, to the new fields <!-- completed: 2026-11-13T14:40 -->
- [x] Record the Director row as ordinal 0, `left`, with its window ID and the name from `window_name` in `fleet create` (`src/cli/fleet.rs`, `src/broker/fleets.rs`), with a unit test in `src/cli/fleet.rs` that a failed window-name read registers nothing <!-- completed: 2026-11-13T14:30 -->
- [x] Adapt `mux_window_id` consumers (both backends' `check_key_target`, fleet-delete revalidation) to `Option<String>` with `.expect` after the concrete-pane check <!-- completed: 2026-11-13T14:30 -->
- [x] Unit tests in `src/db.rs`: the baseline CHECK rejects a placement whose pane, window ID and window name are not all null or all set <!-- completed: 2026-11-13T14:30 -->
- [x] Unit tests in `src/broker/members.rs`: the occupancy query groups concrete placements by ordinal and side and excludes the Director and pending rows; a bad stored `column_side` fails with `InvalidStoredValue` <!-- completed: 2026-11-13T14:30 -->

### Step 5: Slot selection

- [x] Create `src/multiplexer/layout.rs` with `MIN_PANE_HEIGHT`, `MAX_PANES_PER_COLUMN`, `ColumnSide`, `capacity`, `WindowOccupancy`, `SlotChoice`, `choose_slot` <!-- completed: 2026-11-13T14:30 -->
- [x] Unit tests in `src/multiplexer/layout.rs`: capacity table rows; Director window fill; Director right capacity 0 skipped; overflow left-then-right; new window ordinal = last + 1 with `cafleet-<fleet>-<n>` name; freed Director slot reused before overflow; emptied left column refilled first; invariant panics <!-- completed: 2026-11-13T14:30 -->

### Step 6: Fleet layout lock

- [x] Add the fleet layout lock (path, `0600`, `LAYOUT_LOCK_WAIT`, retry every 50 ms) beside the pane lock in `src/pane_lock.rs` <!-- completed: 2026-11-13T14:30 -->
- [x] Unit tests in `src/pane_lock.rs`: a second acquisition blocks until release; expiry reports busy; distinct fleets do not contend <!-- completed: 2026-11-13T14:30 -->

### Step 7: Multiplexer interface

- [x] Replace `split_window` with `spawn_member_pane`; add `window_height`, `window_name`, `PaneTarget`, `ColumnTarget`, `SpawnedPane`; change `kill_pane` and `close_pane` to take the window ID in `src/multiplexer/mod.rs` <!-- completed: 2026-11-13T14:40 -->
- [x] Update every `LaunchHost` implementation, production and test alike, so `split_runner` returns `SpawnedPane` <!-- completed: 2026-11-13T14:40 -->
- [x] Update every `LaunchHost::close_pane` implementation, production and test alike, to take the `SpawnedPane` and pass its window ID to `kill_pane` <!-- completed: 2026-11-13T14:40 -->

### Step 8: tmux backend

- [x] Implement the four creation argvs, `new-window` reply parsing, `select-pane -T`, and remove `main-vertical` in `src/multiplexer/tmux.rs` <!-- completed: 2026-11-13T14:40 -->
- [x] Implement `window_name` (`#{window_id}|#{window_name}`, first field must match) and `window_height` <!-- completed: 2026-11-13T14:40 -->
- [x] Implement equalize-every-column (`list-panes` + `select-layout -E` per column) after creation and in `restore_after_close`, tolerating `can't find window` <!-- completed: 2026-11-13T15:08 -->
- [x] Unit tests in `src/multiplexer/tmux.rs` (FakeRunner argv): each creation target; naming and equalization errors ignored; malformed `new-window` reply is `PaneCleanup::Unknown`; empty and non-integer height replies fail; `window_name` rejects an empty or other-window reply and keeps a name containing `|`; restoration of a closed window does nothing; compensation `kill_pane("%7", "@1")` runs exactly `kill-pane -t %7`, `list-panes -t @1 -F "#{pane_id}|#{pane_left}"`, then `select-layout -E -t <first pane>` per column of 2 or more panes <!-- completed: 2026-11-13T14:40 -->

### Step 9: herdr backend

- [x] Implement column append (bottom pane from the live layout), first-right split, `tab create` new window, then `pane run`, `pane rename` in that order in `src/multiplexer/herdr.rs` <!-- completed: 2026-11-13T14:40 -->
- [x] Implement Right and Left re-parenting, each through one temporary tab with `--no-focus` moves, after `pane run` and `pane rename` <!-- completed: 2026-11-13T14:40 -->
- [x] Generalize `right_column` / `equalize_column` to every column (group by `rect.x`, match split `rect.x` and `rect.width`) <!-- completed: 2026-11-13T14:40 -->
- [x] Replace the `closed_tab` pre-close `pane get` read with the window ID passed to `close_pane`; implement the three-row delete-time restoration <!-- completed: 2026-11-13T14:40 -->
- [x] Implement `window_name` (`tab get` label) and `window_height` (`layout.area.height`, trying each anchor in order) <!-- completed: 2026-11-13T14:40 -->
- [x] Unit tests in `src/multiplexer/herdr.rs`: each creation target argv; re-parenting argv for k = 2 and k = 4 and for m = 1 and m = 3; a `pane run` failure closes only the new pane and runs no move; a failed move emits the diagnostic and keeps the spawn; two-column equalization; restoration per tab state; `tab create` missing fields are `PaneCleanup::Unknown`; `window_height` with a first anchor failing `pane_not_found` and a second answering returns the second's height, and fails with the last error only when every anchor fails; `window_name` reads the `tab get` label and rejects a reply without one <!-- completed: 2026-11-13T14:40 -->

### Step 10: `member create` wiring

- [x] Acquire the layout lock before resolving the Director placement and capture the creation deadline after it; read occupancy and heights; call `choose_slot`; register with ordinal/side and the ordinal update in `src/cli/member.rs` and `src/runtime/lifecycle.rs` <!-- completed: 2026-11-13T14:59 -->
- [x] Publish pane ID, window ID and window name together; release the lock when creation returns <!-- completed: 2026-11-13T14:59 -->
- [x] Unit tests in `src/runtime/lifecycle.rs`: publication updates all three fields; compensation passes the spawned window ID to `close_pane`; compensation after a new-window spawn failure leaves the ordinal consumed <!-- completed: 2026-11-13T15:08 -->
- [x] Unit tests in `src/cli/member.rs`: a busy layout lock fails with the create busy string and registers nothing; an unreadable overflow window fails with the unreadable-window string listing its members and registers nothing; a changed `last_window_ordinal` rolls back with the invariant message <!-- completed: 2026-11-13T15:08 -->

### Step 11: Deletion

- [x] Acquire the layout lock in `member delete` after the pane lock and before cleanup, extending the deletion deadline by the wait, in `src/cli/member.rs` <!-- completed: 2026-11-13T14:59 -->
- [x] Acquire the layout lock once in `fleet delete` after its pane locks; pass it to `end_member`; pass `placement.mux_window_id` to `close_pane` in `src/cli/fleet.rs` <!-- completed: 2026-11-13T14:59 -->
- [x] Unit tests in `src/cli/member.rs`: a busy layout lock fails `member delete` with its busy string and leaves the run, pane and row unchanged <!-- completed: 2026-11-13T15:08 -->
- [x] Unit tests in `src/cli/fleet.rs`: a busy layout lock fails `fleet delete` with its busy string and leaves every member unchanged; `close_pane` receives the recorded window ID <!-- completed: 2026-11-13T15:08 -->

### Step 12: Output

- [x] Update `format_member`, `format_member_list`, `format_member_detail` in `src/output/text.rs` and `placement` in `src/output/json.rs` <!-- completed: 2026-11-13T14:59 -->
- [x] Unit tests in `src/output/text.rs`: the example `member list` rows above and a `(pending)` window <!-- completed: 2026-11-13T15:08 -->
- [x] Unit tests in `src/output/json.rs`: placement key order and nulls <!-- completed: 2026-11-13T15:08 -->

### Step 13: Live verification

- [x] tmux 3.6a: create a fleet in a ≥ 120-row window and spawn enough members to open two overflow windows; confirm names, focus, per-column equal heights, `member list` windows <!-- completed: 2026-11-13T15:19 -->
- [x] tmux: delete members to empty an overflow left column, then create one member and confirm the left column is recreated; delete the rest and confirm the window closes and the next window takes a higher ordinal <!-- completed: 2026-11-13T15:19 -->
- [x] herdr 0.9.5: repeat the two tmux checks, confirming re-parenting keeps pane IDs and the focused tab <!-- completed: 2026-11-13T15:19 -->
- [x] Close an overflow window by hand and confirm the next `member create` reports the unreadable-window error naming its members <!-- completed: 2026-11-13T15:19 -->
- [x] Run three `member create` calls concurrently, then one `member delete` concurrently with a `member create`, on each backend; confirm distinct slots, no busy error and an intact layout <!-- completed: 2026-11-13T15:19 -->

### Step 14: Validation

- [x] `mise //:test` <!-- completed: 2026-11-13T15:15 -->
- [x] `mise //:lint` <!-- completed: 2026-11-13T15:13 -->
- [x] `mise //docs:build` and inspect any changed table with a literal pipe in the rendered HTML <!-- completed: 2026-11-13T15:13 -->
- [ ] Reinstall the CLI through its mise task <!-- completed: -->

### Step 15: Deleting a member whose herdr pane was closed by hand

The unreadable-window remedy (`cafleet member delete MEMBER_ID`) works on both backends. Closing a herdr tab by hand kills the pane shell; the runner is reparented, shuts down and exits, and stays an unreaped zombie when the reparenting process never reaps. A recorded process that has exited but is not yet reaped (state `Z`; macOS `SZOMB`) is terminated evidence wherever recovery or deletion judges liveness: the run owner's process evidence, the Hermes runner-exit wait, and recovery's descendant-group checks, which count only non-zombie group members. `member delete` and `fleet delete` then complete as for any exited runner; their command facts are unchanged.

- [x] Diagnose the deletion/recovery path that records `terminal restoration requires host recovery: EIO` and propose the exact rule in `COMMENT(programmer)` at this step before changing code <!-- completed: 2026-11-13T23:40 -->
- [x] Update `SPEC.md` (recovery/deletion contract) and, if it changes a command fact, `assets/skills/cafleet/reference/cli.md` <!-- completed: 2026-11-13T23:41 -->
- [ ] Implement the rule so `member delete` and `fleet delete` complete for a member whose herdr pane is gone <!-- completed: -->
- [ ] Render the unreadable-window `<cause>` on one line (herdr's `\nstderr:` separator) <!-- completed: -->
- [ ] Unit tests for the rule in the file that owns it, and for the one-line cause in `src/cli/member.rs` <!-- completed: -->
- [ ] herdr 0.9.5 live: close an overflow tab by hand, then `member delete` each listed member and confirm the next `member create` succeeds <!-- completed: -->

COMMENT(programmer): Diagnosis. The `EIO` diagnostic is not what blocks deletion: recovery's `perform` (`src/runtime/recovery.rs`) rewrites `cleanup_state='complete'` over any runner diagnostic once it reaches its end. The block is upstream, in liveness. herdr runs the runner as a child of the pane's `/bin/sh`. Closing the tab kills that shell, so the runner is reparented to PID 1, receives SIGHUP, shuts down (recording the `EIO` restoration failure) and exits. On this host PID 1 is `jupyter-lab`, which never reaps, so the runner stays a zombie: `ps` shows `[cafleet] <defunct>` entries with PPID 1 at the Step 13 live-check times. A zombie has released its run lock, but `NativeRecoveryHost::process_evidence` compares only `/proc/<pid>/stat` identity (`ProcessIdentity::read`), so it reports the zombie owner `Alive`, `recover` returns `LiveOrUnreachable`, and `end_member` fails with `member recovery remains incomplete; inspect with cafleet doctor`. No owner transfer happens, so every retry sees the same zombie. tmux reaps its pane processes, which is why only herdr failed. A second instance of the gap: `recover_descendants` (`src/runtime/process.rs`) fails with `descendant ownership cannot be anchored to the recorded child generation` whenever any process remains in the child's group, including unreaped zombies. The runner's own `survivors_with` already ignores those. Proposed rule (SPEC §6.9 Member PTY runtime, recovery liveness): "A recorded process that has exited but is not yet reaped (state `Z`; macOS `SZOMB`) is terminated evidence wherever recovery or deletion judges liveness: the run owner's process evidence, the Hermes runner-exit wait (`await_runner_exit` in `src/cli/fleet.rs`), and recovery's descendant-group checks, which count only non-zombie group members." Implementation: `ProcessIdentity::is_alive()` (identity match and not stopped, the existing private `running`) used by `process_evidence` and `await_runner_exit`, plus a zombie filter on the two `group_members(group)?.is_empty()` checks in `recover_descendants`. No pane-absence probe and no special case for the restoration diagnostic are needed: herdr `close_pane` already tolerates `pane_not_found`, and the closed tab's restoration is best-effort. `member delete` / `fleet delete` then complete as for any exited runner, and their command facts and `cli.md` do not change. Please confirm the rule, or ask instead for the pane-absence rule the step text describes.


---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-13 | Initial draft |
| 2026-11-13 | Review round 1: unreadable-window error; layout lock spans deletion and the whole creation with its own 60-second wait; restoration interface; Director window name read with the pane context; one-tab Left re-parenting; herdr step ordering and live column order; verified herdr tab closure; complete documentation surfaces |
| 2026-11-13 | Review round 2: herdr height read tries every member anchor; window name read only by `fleet create` (doctor unchanged); every close receives its window ID, including creation compensation |
| 2026-11-13 | Execution: Step 15 added after live verification showed the unreadable-window remedy failing on herdr (user-approved scope extension) |
