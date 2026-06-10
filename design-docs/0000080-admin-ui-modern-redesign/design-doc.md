# Admin WebUI Modern Redesign

**Status**: Approved
**Progress**: 10/24 tasks complete
**Last Updated**: 2026-06-10

## Overview

Redesign the admin WebUI (`admin/` — React 19 + Vite 8 + Tailwind CSS 4, served by the cafleet FastAPI server) into a visually elegant, modern interface with light/dark themes and the brand violet accent, while preserving every existing behavior. The redesign also adds a per-agent detail view backed by the existing-but-unused `/api/agents/{agent_id}/inbox` and `/sent` endpoints.

## Success Criteria

- [ ] Light and dark themes with a header toggle: explicit choice persisted in `localStorage`, default follows `prefers-color-scheme`, no flash of wrong theme on load
- [ ] Brand violet accent (derived from `admin/public/favicon.svg`) applied consistently: header wordmark, primary button, mention chips, focus rings, active states
- [ ] New agent detail view at `#/fleets/<fleetId>/agents/<agentId>` showing agent profile plus Inbox / Sent tabs fed by `GET /api/agents/{agent_id}/inbox` and `/sent`
- [ ] Every behavior in the *Preserved behaviors* table works exactly as before (verified manually)
- [ ] `mise //admin:lint` and `mise //admin:build` (which runs `tsc -b`) pass; light + dark screenshots captured for fleet picker, dashboard, and agent detail views
- [ ] Documentation updated in the same cycle: `docs/concepts/overview.md` § WebUI, `docs/spec/webui-api.md`, `docs/get-started/contributing.md` (only if commands/deps workflow changes)

---

## Background

The current UI is functional but visually plain: light-only gray Tailwind utilities, system fonts, no icons, no brand identity (the violet bolt favicon is ignored), `Loading...` text instead of loading states, and an `Updating…` italic label as the only liveness signal. Two screens exist — FleetPicker (`#/fleets`) and Dashboard (`#/fleets/<id>/agents` with agent sidebar, message timeline, composer). The backend already exposes per-agent inbox/sent endpoints that no UI consumes.

User decisions from the clarification round:

| Topic | Decision |
|---|---|
| Scope | Restyle + UX restructure; add the per-agent detail view |
| Aesthetic | Light + dark with toggle; violet accent from the favicon |
| Motion | Delegated to the Drafter — spec'd below as "tasteful middle ground" |
| Dependencies | Small runtime deps and a component library allowed; concrete proposals required |
| Testing | Lint + typecheck + build + manual visual verification (screenshots); no new test infra |
| Behavior | Preserve ALL existing behavior exactly |

---

## Specification

### 1. Dependencies

All added to `admin/package.json` `dependencies`. Procedure: edit `package.json`, then run plain `bun install` inside `admin/` to regenerate `bun.lock`. (`mise //admin:install` runs `bun install --frozen-lockfile` and cannot update the lockfile — it is only for frozen reinstalls after `bun.lock` is committed.)

| Package | Purpose | Rationale |
|---|---|---|
| `lucide-react` | Icons (send, refresh, sun/moon, x, chevron, inbox, alert-triangle, zap, …) | Tree-shakeable per-icon ESM imports (~1 kB each), consistent stroke style, de-facto standard with Tailwind |
| `radix-ui` | Accessible unstyled primitives: `Tooltip` (ack chips), `Tabs` (agent detail Inbox/Sent) | Unified single package; correct ARIA/focus behavior without hand-rolling; styled entirely with Tailwind classes |
| `@fontsource-variable/inter` | UI typeface, self-hosted | Bundled by Vite — no CDN, works on an offline localhost admin; variable font = one file for all weights |
| `@fontsource-variable/jetbrains-mono` | Monospace for ids and `cafleet …` command hints | Same self-hosting rationale; visually pairs with Inter |

Rejected alternatives (recorded for the Reviewer):

- **shadcn/ui** — vendors generated component code plus a CLI and `cva`/`clsx`/`tailwind-merge` utilities; disproportionate machinery for ~10 components. We style `radix-ui` primitives directly.
- **framer-motion** — ~40 kB for animation needs fully covered by Tailwind transitions + a few custom `@keyframes`.

### 2. Theme system

**Tokens** — `admin/src/index.css` becomes the single token source using Tailwind 4 CSS-first config:

```css
@import "tailwindcss";
@import "@fontsource-variable/inter";
@import "@fontsource-variable/jetbrains-mono";

@custom-variant dark (&:where(.dark, .dark *));

:root {
  --surface: …;          /* page background */
  --surface-raised: …;   /* cards, header, composer */
  --surface-hover: …;
  --border: …;
  --text: …;
  --text-muted: …;
  --text-faint: …;
  --accent: …;           /* violet, from favicon #7e14ff/#863bff */
  --accent-hover: …;
  --accent-soft: …;      /* tinted backgrounds: mention chips, selected rows */
  --success / -soft: …;  /* ack chips, active status dot */
  --danger / -soft: …;   /* errors, Administrator-missing banner */
}
.dark { /* same custom properties, dark values */ }

@theme inline {
  --color-surface: var(--surface);
  /* … register every semantic token (--color-<name>: var(--<name>)) so
       bg-surface, text-text-muted, border-border, bg-accent etc. exist
       as utilities … */
  --font-sans: "Inter Variable", system-ui, sans-serif;
  --font-mono: "JetBrains Mono Variable", ui-monospace, monospace;
}
```

The raw custom properties (`--surface`, …) deliberately use names distinct from the `--color-*` theme keys: a same-name pairing (`--color-surface: var(--color-surface)`) only works while `@theme inline` suppresses Tailwind's own variable emission and turns into a circular `var()` the moment `inline` is dropped or the pattern is copied into a non-inline `@theme`.

Components use only semantic utilities (`bg-surface`, `text-text-muted`, `bg-accent`, …) — no raw `gray-*`/`violet-*` classes in components, so dark mode needs no per-component `dark:` overrides except where a genuinely different treatment is wanted. Accent values are tuned per theme (darker violet on light, lighter violet on dark) for WCAG AA contrast on text/interactive elements.

**State** — new `admin/src/hooks/useTheme.ts`:

- `localStorage` key `cafleet-admin-theme`, values `"light" | "dark"`; key absent → follow `prefers-color-scheme` via `matchMedia` (including live changes through its `change` listener).
- Hook returns `{ theme, toggle }`; `toggle` sets the explicit opposite of the currently effective theme and persists it. Applying = adding/removing `dark` on `document.documentElement`.

**FOUC prevention** — inline `<script>` in `admin/index.html` `<head>` (before the module script) that reads the key / `matchMedia` and sets the `dark` class synchronously. Also set `<meta name="color-scheme" content="light dark">` so native scrollbars/form controls match.

### 3. Visual language

- **Typography**: Inter Variable for UI; JetBrains Mono for agent/fleet ids, timestamps in tooltips, and `cafleet …` command hints.
- **Surfaces**: flat `surface` page background; `surface-raised` cards/header/composer separated by 1px `border` and `shadow-sm`; `rounded-lg`/`rounded-xl` radii. No glassmorphism, no gradients except a subtle violet tint allowed on the brand wordmark.
- **Brand**: header shows a small bolt mark (inline SVG reusing the favicon silhouette as a single-color `currentColor` path — the multi-layer blurred favicon does not scale down legibly) + "CAFleet" wordmark; the active fleet renders as a breadcrumb (`Fleets / <label or id>`).
- **Agent identity**: new `admin/src/components/AgentAvatar.tsx` — circular avatar with the agent's first two characters, background from a deterministic 12-color palette (hash of `agent_id`; palette defined as tokens with light/dark variants). The built-in Administrator always gets the violet brand color and a `Zap` icon instead of initials. Used in sidebar, timeline, mention popover, and agent detail.
- **Status**: active = small green dot; deregistered = gray dot + dimmed row. Administrator gets an "Admin" badge (violet `accent-soft`).

### 4. Motion (Drafter's delegated decision: tasteful middle ground)

| Element | Treatment |
|---|---|
| Initial loads (fleet list, timeline, agent detail) | Skeleton placeholders instead of `Loading...` text |
| New timeline entries | One-time fade-and-rise-in on mount (~200 ms). React keys are stable task ids, so existing entries never remount/re-animate across polls |
| Liveness | `Updating…` text replaced by a compact "Live" indicator in the header: green dot with a gentle pulse; during an in-flight poll the dot swaps to a small spinning `RefreshCw` icon |
| Hover | Fleet cards and sidebar rows get background + ring transitions (~150 ms); theme switch transitions colors (~150 ms) |
| Accessibility | All animations behind `motion-safe:`; `prefers-reduced-motion` users get instant state changes |

No scroll-jacking, no parallax, no entrance animation replay on poll refreshes.

### 5. Layout and components

**App shell** — new `admin/src/components/AppHeader.tsx` shared by both screens: brand + breadcrumb left; Live indicator, "Sending as Administrator" note (dashboard only, when applicable), Refresh icon button, ThemeToggle right. "Back to Fleets" becomes the breadcrumb's `Fleets` link (still calls the existing `onBack` path so `setFleetId(null)` + hash navigation are unchanged).

**FleetPicker** — centered column (`max-w-2xl`): page title, fleet cards (label prominent; mono `#id` badge; metadata row "N agents · created <date>"; chevron). Hover = ring + slight elevation. Empty/error states keep the exact current copy (including the `cafleet fleet create` hint) restyled with an icon and code chip.

**Dashboard** — same three regions, restyled:

- *Sidebar* (`w-60`): "Active"/"Deregistered" group headers kept; rows become buttons with avatar + name + status dot (+ Admin badge). Clicking navigates to the agent detail route. Deregistered rows are also clickable (read-only history view) — this replaces the current `pointer-events-none`; their dimmed styling stays.
- *Timeline*: entries get avatar + sender name + recipient mention chips (`accent-soft`; broadcast recipients render as today — one chip per delivery row) + HH:MM time. Day-divider rows (e.g. "June 10, 2026") between calendar days. Canceled entries stay struck-through and dimmed. Ack pills keep the `[ack]` semantics restyled as green pills; the hover tooltip (recipient label + ISO `status_timestamp`) moves to `radix-ui` Tooltip with identical content. Grouping, sort keys, and near-bottom auto-scroll logic are untouched.
- *Composer* (`MessageInput`): rounded-xl focus-ring container; autogrow textarea unchanged; submit becomes an icon button (`Send`, accent background); hint row `Enter to send · Shift+Enter for newline`; mention popover restyled with avatars (selected row = `accent-soft`), keyboard handling (arrows / Enter / Tab / Esc / IME composing) byte-for-byte preserved. Administrator-missing banner keeps its exact text and `db init` guidance, restyled as a danger alert with `TriangleAlert` icon.

**Agent detail** — new `admin/src/components/AgentDetail.tsx`:

- Route: `#/fleets/<fleetId>/agents/<agentId>`. `App.tsx`'s `parseHash` gains the third segment; route type becomes `{ kind: "fleets" | "dashboard"; fleetId?: string; agentId?: string }`. Existing routes parse exactly as before.
- Rendering: right-hand panel (`w-96`, `border-l`, own scroll) inside the Dashboard layout — timeline stays visible and live next to it. Close button (`X`) and `Escape` navigate back to `#/fleets/<fleetId>/agents`. Escape precedence: the panel's document-level `keydown` listener ignores events with `defaultPrevented === true`; `MessageInput`'s mention-popover handler already calls `preventDefault` when it consumes Escape, so one keypress closes the popover and a second closes the panel.
- Content: header (avatar, name, status badge, Admin badge if applicable), description, mono `agent_id`, `registered_at`; below, `radix-ui` Tabs **Inbox** / **Sent**. Each tab is a compact list: direction line (`from <name>` / `to <name>`), HH:MM + date, status chip (`input_required` = "pending", `completed` = "acked", `canceled` = "canceled"), body (`whitespace-pre-wrap break-words`). API order (newest first) is kept. Empty tab → small empty state ("No messages").
- Broadcast rows: rendered ungrouped, exactly as the API returns them — one row per delivery, each with its own recipient and status chip. The per-recipient delivery lifecycle is precisely what this view adds over the timeline's grouped rendering, so no `origin_task_id` grouping here.
- Row cap: `/inbox` and `/sent` are unbounded on the backend (unlike `/timeline`'s 200-row cap). The UI slices client-side to the most recent 200 rows per tab and, when truncated, appends a muted footer note ("Showing the 200 most recent messages").
- Data: new `api.ts` functions reusing the `TimelineMessage` type (the backend formats all message endpoints through the same `_format_messages`, so the field set matches `/api/timeline`):

```ts
export async function fetchInbox(agentId: number): Promise<TimelineResponse> {
  return request<TimelineResponse>(`/agents/${agentId}/inbox`);
}
export async function fetchSent(agentId: number): Promise<TimelineResponse> {
  return request<TimelineResponse>(`/agents/${agentId}/sent`);
}
```

- Refresh: the panel refetches when Dashboard's existing `refreshKey` bumps (5 s poll / manual Refresh / post-send) — no second polling loop, mirroring `Timeline`'s in-flight-guard pattern.
- Errors/edge cases: an `agentId` not present in the loaded `agents` list → navigate back to `#/fleets/<fleetId>/agents` before any fetch (the membership check covers unknown and cross-fleet ids; no status-code inspection is needed and `api.ts`'s error shape stays unchanged). Any fetch failure → keep last-known lists (same swallow-and-retry policy as `Timeline`). Deregistered agents render normally with the "Deregistered" badge.

### 6. Preserved behaviors (verification checklist)

| Behavior | Where it lives | Redesign rule |
|---|---|---|
| Hash routes `#/fleets`, `#/fleets/<id>/agents` (+ unknown-fleet redirect to `#/fleets`) | `App.tsx` | Unchanged; one route added |
| 5 s polling cadence, shared in-flight guards, no pause on visibility change | `usePolling.ts`, `Timeline.tsx` | Untouched |
| @mention autocomplete: slugify rules, prefix matching, max 6 candidates, `@all` virtual entry, arrows/Enter/Tab/Esc, IME-composing guards, caret-inside-token rewrite, blur-delay close | `MessageInput.tsx` | Logic untouched; only classNames/markup styling change |
| Parse errors: `@all` + others rejected, multi-recipient unicast rejected, unknown/ambiguous mention, empty body | `MessageInput.tsx` `parseInput` | Untouched, same messages |
| Broadcast grouping by `origin_task_id`, min-`created_at` sort key | `Timeline.tsx`, `timeline.ts` | Untouched |
| Ack reaction chips + per-recipient tooltip (agent label incl. `(deregistered)` suffix, ISO timestamp) | `ReactionBar.tsx` | Same content; tooltip impl → radix |
| Near-bottom auto-scroll on new entries | `Timeline.tsx` `useLayoutEffect` | Untouched |
| Administrator-missing banner text + send disabled; `disabled` placeholder text | `Dashboard.tsx`, `MessageInput.tsx` | Same copy, restyled |
| Empty states' `cafleet …` CLI hints (exact commands) | `FleetPicker`, `Dashboard`, `Sidebar` | Same copy, restyled |
| Sending always as Administrator | `Dashboard.tsx` | Untouched |
| Build output `../cafleet/src/cafleet/webui`, `/` base, `/api` dev proxy | `vite.config.ts` | Untouched |
| FastAPI `SPAStaticFiles` serving + reserved `ui/`/`api/` prefixes | `cafleet/src/cafleet/server.py` | No backend change anywhere in this design |

The one deliberate behavior change (allowed by the approved UX-restructure scope): deregistered sidebar rows become clickable to open the read-only detail view.

### 7. File inventory

| File | Change |
|---|---|
| `admin/package.json`, `admin/bun.lock` | Add the four dependencies |
| `admin/index.html` | FOUC-prevention inline script, `color-scheme` meta |
| `admin/src/index.css` | Token system, dark variant, fonts, keyframes |
| `admin/src/hooks/useTheme.ts` | New |
| `admin/src/components/AppHeader.tsx` | New shared shell header |
| `admin/src/components/ThemeToggle.tsx` | New |
| `admin/src/components/AgentAvatar.tsx` | New (incl. deterministic color hash) |
| `admin/src/components/AgentDetail.tsx` | New |
| `admin/src/components/Skeleton.tsx`, `EmptyState.tsx` | New small shared components |
| `admin/src/App.tsx` | Route parsing for `agentId`; pass-through to Dashboard |
| `admin/src/api.ts` | `fetchInbox`, `fetchSent` |
| `admin/src/components/{FleetPicker,Dashboard,Sidebar,Timeline,TimelineMessage,ReactionBar,MessageInput}.tsx` | Restyle; Sidebar rows clickable; Dashboard hosts detail panel |
| `docs/concepts/overview.md` | § WebUI: theme toggle, agent detail view |
| `docs/spec/webui-api.md` | Note inbox/sent are consumed by the agent detail view; align inbox/sent example fields with `_format_messages` (they also return `status_timestamp` / `origin_task_id`) |
| `docs/get-started/contributing.md` | Only if the dependency workflow note is needed (bun.lock regeneration) |

`README.md` mentions the admin WebUI only as a one-line architecture fact — no feature list — so no README change is expected; verify during Step 1. No `skills/*/SKILL.md` documents the WebUI; none change. No Python, CLI, or schema changes.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (project rule: docs before code)

- [x] Update `docs/concepts/overview.md` § WebUI: light/dark theme toggle, agent detail view (Inbox/Sent), violet brand styling <!-- completed: 2026-06-10T09:31 -->
- [x] Update `docs/spec/webui-api.md`: mark inbox/sent as consumed by the agent detail view; align their response examples with the shared `_format_messages` field set <!-- completed: 2026-06-10T09:31 -->
- [x] Verify `README.md` and `skills/*/SKILL.md` need no changes (no CLI/API surface change); update `docs/get-started/contributing.md` only if a lockfile-regeneration note is warranted <!-- completed: 2026-06-10T09:31 -->

### Step 2: Foundation — dependencies, tokens, theme

- [x] Add `lucide-react`, `radix-ui`, `@fontsource-variable/inter`, `@fontsource-variable/jetbrains-mono` to `admin/package.json`; regenerate `bun.lock` <!-- completed: 2026-06-10T09:38 -->
- [x] Rewrite `admin/src/index.css`: semantic token custom properties for `:root` and `.dark`, `@custom-variant dark`, `@theme inline` registration, font imports, motion keyframes <!-- completed: 2026-06-10T09:38 -->
- [x] Add `useTheme.ts` (localStorage + matchMedia + html class) and `ThemeToggle.tsx` <!-- completed: 2026-06-10T09:38 -->
- [x] Add FOUC-prevention inline script and `color-scheme` meta to `admin/index.html` <!-- completed: 2026-06-10T09:38 -->
- [x] Add shared primitives: `Skeleton.tsx`, `EmptyState.tsx`, `AgentAvatar.tsx` (deterministic 12-color palette) <!-- completed: 2026-06-10T09:38 -->

### Step 3: App shell and FleetPicker

- [x] Build `AppHeader.tsx` (brand mark + wordmark, breadcrumb, Live indicator, Refresh, ThemeToggle) and wire into both screens <!-- completed: 2026-06-10T09:42 -->
- [x] Restyle `FleetPicker.tsx`: fleet cards, skeleton loading, restyled empty/error states with preserved copy <!-- completed: 2026-06-10T09:42 -->

### Step 4: Dashboard restyle

- [ ] Restyle `Sidebar.tsx`: avatar rows, status dots, Admin badge, clickable rows (incl. deregistered) navigating to the detail route <!-- completed: -->
- [ ] Restyle `Timeline.tsx` + `TimelineMessage.tsx`: avatars, mention chips, day dividers, entry mount animation, skeleton loading; grouping/scroll logic untouched <!-- completed: -->
- [ ] Migrate `ReactionBar.tsx` tooltip to radix Tooltip with identical content <!-- completed: -->
- [ ] Restyle `MessageInput.tsx`: composer container, icon send button, kbd hint, restyled mention popover with avatars; parsing/keyboard logic untouched <!-- completed: -->
- [ ] Restyle the Administrator-missing banner and "Sending as Administrator" note <!-- completed: -->

### Step 5: Agent detail view

- [ ] Extend `App.tsx` route parsing for `#/fleets/<id>/agents/<agentId>` (existing routes byte-compatible) <!-- completed: -->
- [ ] Add `fetchInbox` / `fetchSent` to `api.ts` <!-- completed: -->
- [ ] Build `AgentDetail.tsx`: profile header, Inbox/Sent radix Tabs, status chips, empty states, Esc/close navigation, refreshKey-driven refetch with in-flight guard <!-- completed: -->
- [ ] Handle edge cases: unknown/cross-fleet agentId → redirect to dashboard route; fetch-failure keeps last-known data <!-- completed: -->

### Step 6: Polish and verification

- [ ] Pass over all interactive elements: focus-visible rings (accent), `motion-safe:` guards, AA contrast check on both themes <!-- completed: -->
- [ ] Run `mise //admin:lint` and `mise //admin:build`; fix all findings <!-- completed: -->
- [ ] Manual verification of every row in the *Preserved behaviors* table against a seeded local fleet <!-- completed: -->
- [ ] Capture light + dark screenshots: fleet picker, dashboard (with messages incl. a broadcast + acks), agent detail (inbox + sent) <!-- completed: -->
- [ ] Rebuild via `mise //admin:build` and smoke-test `cafleet server` serving the new bundle at `/` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-10 | Initial draft |
| 2026-06-10 | Reviewer round 1: fixed lockfile-regeneration procedure; distinct raw/theme token names to avoid circular `var()`; Escape precedence (popover before panel); detail tabs render broadcast rows ungrouped with a client-side 200-row cap; agent-existence check via `agents`-list membership instead of 404 inspection |
