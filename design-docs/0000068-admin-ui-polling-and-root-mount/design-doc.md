# Admin WebUI: auto-refresh, latest-first sessions, and root mount

**Status**: Approved
**Progress**: 20/20 tasks complete
**Last Updated**: 2026-05-23


## Overview

Three coordinated improvements to the admin WebUI ship in one design and one PR: (1) the three primary views (Sessions, Dashboard, Timeline) refresh themselves every 5 seconds with a subtle in-flight indicator, (2) the sessions list is server-sorted newest-first, and (3) the SPA and its API move from `/ui` and `/ui/api/*` to `/` and `/api/*` as a hard break with no redirect.

## Success Criteria

- [x] `http://<host>:<port>/` serves the admin SPA `index.html`. `/ui/` and any other path starting with `ui/` returns HTTP 404 with no `index.html` body. Known API routes live under `/api/*`; unknown paths starting with `api/` return HTTP 404, NOT the SPA `index.html`.

- [x] `broker.list_sessions()` returns rows ordered by `created_at DESC, session_id ASC`. The frontend renders that order verbatim with no client-side re-sort.
- [x] `SessionPicker`, `Dashboard` (agent list), and `Timeline` auto-refresh every 5 s. A small "Updating…" indicator is visible during each in-flight poll. The manual "Refresh" button on `Dashboard` is retained and still works alongside polling.
- [x] Timeline auto-scroll follows the tail only when the user is already near the bottom; if the user scrolled up, new messages arriving via a poll do not yank focus.
- [x] Polling continues regardless of `document.hidden` (no visibility-aware pause).
- [x] Polling errors are silently swallowed; the next tick re-attempts. If a tick fires while the previous request is still pending, the new tick is skipped (no concurrent requests, no queue).
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` pass. `mise //admin:build` produces a bundle that loads under base `/`.
- [x] No `/ui` or `/ui/api` reference remains in tracked source, tests, or current documentation (`README.md`, `ARCHITECTURE.md`, `CONTRIBUTING.md`, `docs/spec/*`, `.claude/rules/commands.md`, every `skills/*/SKILL.md`). Verified by `rg -n '/ui(/|\b)'` returning only this design doc and prior design-doc entries.

---

## Background

### Current shape

The admin is a React 19 + Vite SPA whose build output is copied into `cafleet/src/cafleet/webui/` and mounted by FastAPI at `/ui`:

| Concern | File:line | Current value |
|---|---|---|
| SPA mount | `cafleet/src/cafleet/server.py:46-50` | `app.mount("/ui", SPAStaticFiles(...))` |
| API router prefix | `cafleet/src/cafleet/webui_api.py:10` | `APIRouter(prefix="/ui/api")` |
| Vite base | `admin/vite.config.ts:8` | `base: '/ui/'` |
| Vite dev proxy | `admin/vite.config.ts:18-25` | proxies `/ui/api` → `http://localhost:8000` |
| Fetch prefix | `admin/src/api.ts:29` | `` fetch(`/ui/api${path}`, ...) `` |
| Sessions sort | `cafleet/src/cafleet/broker.py:224` | `.order_by(Session.created_at)` (ASC) |

There is no auto-refresh anywhere. `Dashboard.tsx` has a manual "Refresh" button that re-runs `getAgents()`/`fetchTimeline()` on click. `SessionPicker.tsx` calls `listSessions()` once on mount. `Timeline.tsx` re-fetches only when its `refreshKey` prop changes (driven by the manual refresh button and `MessageInput`'s `onSent` callback). Stale data persists until the user interacts.

`broker.list_sessions()` sorts ASC by `created_at`, so the oldest session is on top — the opposite of what an operator returning to `/sessions` wants.

The `admin/` package has **no test runner** — only `dev`, `build`, and `lint` scripts. This design intentionally does not change that; no Vitest, no Jest, no follow-up doc is planned to add one. Verification of frontend changes is `mise //admin:build` + `mise //admin:lint` + manual inspection of the running SPA.

### Why the three changes ship together

Each is small but they overlap on the same files (`admin/src/api.ts`, `admin/src/components/*.tsx`, `admin/vite.config.ts`, `cafleet/src/cafleet/server.py`, `cafleet/src/cafleet/webui_api.py`, and the doc set). Splitting into three docs would force a mid-flight state where docs and code disagree about the mount path, plus a second editing pass over `SessionPicker.tsx`. One design, one branch, one PR.

### Why hard break for `/ui → /`

The repository's removal rule (`~/.claude/rules/removal.md`) requires that the codebase read as if the removed surface never existed. A `RedirectResponse` from `/ui/*` to `/*` would be deprecation residue. The admin runs only on `127.0.0.1:8000` (per `settings.broker_host` default); broken bookmarks are local. The design doc itself is the historical record for anyone landing on a stale `/ui/...` URL.

The SPA-fallback behavior of `SPAStaticFiles` makes a naive `app.mount("/", ...)` swallow `/ui/...` and unknown `/api/...` paths into `index.html` with HTTP 200 (because `SPAStaticFiles.get_response` falls back to `index.html` on any 404 inside the mounted directory). The hard break therefore requires a small `SPAStaticFiles` change to re-raise 404 for the `ui/` and `api/` prefixes — see §Part A Backend.

### Why polling, not SSE/WebSocket

The admin uses raw `fetch` with no `react-query`/`swr`/axios. Subscribing to a stream would require new infrastructure for no proportional gain at this data volume (one session list, one agent list, a few dozen-to-low-hundred timeline rows on a local SQLite-backed server). SSE remains a reasonable future change; explicitly out of scope here.

### Why polling does NOT pause on `document.hidden`

The operator wants the dashboard to be current the moment they tab back to it, with no first-after-resume staleness window. The trade-off — polling fires forever against `127.0.0.1` even when nobody is watching — is negligible at one tick per 5 s against a local SQLite backend.

---

## Specification

### Part A — Move SPA and API from `/ui` to `/`

#### Backend

`cafleet/src/cafleet/server.py`:

- In `create_app()`, change `app.mount("/ui", ...)` to `app.mount("/", ...)`.
- Update the missing-bundle warning text from `/ui/ will return 404` to `/ will return 404`.
- Update the module docstring from `(``/ui/``) and its ``/ui/api/*`` endpoints` to `(``/``) and its ``/api/*`` endpoints`.
- Leave the call order `app.include_router(webui_router)` **before** `app.mount("/", ...)` unchanged. Add a short single-line comment immediately above the mount call: `# include_router MUST precede this mount so API 404s are not swallowed by the SPA fallback.` No automated regression test guards this — the comment is the safeguard.
- Modify `SPAStaticFiles.get_response` so the `index.html` fallback only fires when the requested path is neither `ui` / `ui/...` nor `api` / `api/...`. Implementation:

  ```python
  class SPAStaticFiles(StaticFiles):
      """StaticFiles subclass that falls back to index.html for SPA routing.

      Re-raises 404 for paths under the reserved ``ui/`` and ``api/`` prefixes
      so they never silently resolve to ``index.html``:
      - ``ui/...`` — removed surface; explicit 404 keeps the removal hard.
      - ``api/...`` — backend surface; unknown API paths must return JSON 404,
        not HTML, so client error paths (``resp.json()``) stay valid.
      """

      _RESERVED_PREFIXES = ("ui", "api")

      async def get_response(self, path, scope):
          try:
              return await super().get_response(path, scope)
          except StarletteHTTPException as e:
              if e.status_code != 404:
                  raise
              first = path.split("/", 1)[0]
              if first in self._RESERVED_PREFIXES:
                  raise
              return await super().get_response("index.html", scope)
  ```

  The `path` argument is the URL path with the mount prefix stripped, per Starlette `StaticFiles` convention. With the mount at `/`, `GET /ui/foo` becomes `path="ui/foo"` and `GET /api/foo` becomes `path="api/foo"` — so `path.split("/", 1)[0]` is `"ui"` or `"api"` respectively. A bare `GET /ui` also yields `path="ui"` and matches.

  Known API routes (`/api/sessions`, `/api/agents`, …) are served by the FastAPI router before reaching the mount, so they are unaffected. Only unknown `api/...` paths reach `SPAStaticFiles` and now get the desired JSON 404 from Starlette rather than the HTML SPA shell.

`cafleet/src/cafleet/webui_api.py`:

- Change `APIRouter(prefix="/ui/api")` to `APIRouter(prefix="/api")`.
- Update the module docstring to reference the new prefix.

#### Frontend

`admin/vite.config.ts`:

- Change `base: '/ui/'` to `base: '/'`.
- Change the dev proxy key from `'/ui/api'` to `'/api'`. Keep `target: 'http://localhost:8000'` and `changeOrigin: true`.

`admin/src/api.ts`:

- Change the fetch template from `` `/ui/api${path}` `` to `` `/api${path}` ``.

#### Tests

`cafleet/tests/test_server_cli.py:126`:

- Update the assertion `assert "/ui/" in captured.err` to `assert "/" in captured.err`. (Trivial — the warning now says `/ will return 404`.)

If a more discriminating assertion is wanted, use `assert "warning: admin WebUI is not built. / will return 404." in captured.err` — exact-substring matches the new warning message.

#### Documentation (current-state surfaces, updated in the doc-first commit)

| File | Change |
|---|---|
| `README.md` | No `/ui` references exist today (verified by `grep -n '/ui' README.md`). No change needed unless §A introduces a new mention. |
| `ARCHITECTURE.md` | Lines 12, 65, 282, 320, 323, 326: every `/ui/` and `/ui/api/*` → `/` and `/api/*`. Adjust the prose so the path naturally reads "served as a SPA at `/`" rather than "at `/`/". The "Run 'mise //admin:build' before `cafleet server`" guidance stays — only the path string changes. |
| `CONTRIBUTING.md` | Line 33: `(required before /ui/ is served)` → `(required before / is served)`. |
| `docs/spec/webui-api.md` | Header `Base path: /ui/api` → `Base path: /api`. Six endpoint headings: `GET/POST /ui/api/...` → `/api/...`. The 409 paragraph mentions `POST /ui/api/messages/send` — update to `POST /api/messages/send`. |
| `docs/spec/data-model.md` | Line 147: `/ui/api/*` → `/api/*`. |
| `docs/spec/cli-options.md` | Line 119: `/ui/api/*` → `/api/*`. Line 284: `/ui/` → `/` and `/ui/api/*` → `/api/*`. Line 305: warning text inside backticks → matches the new server warning. |
| `.claude/rules/commands.md` | Line 14: `Serves /ui/ only after mise //admin:build` → `Serves / only after mise //admin:build`. |
| `skills/*/SKILL.md` | No references today (verified by `grep -rln '/ui' skills`). No change needed. |
| `researches/recent-llm-studies/*.md` | Gitignored per project rule — do not modify. |
| Prior `design-docs/*/design-doc.md` | Historical record; leave untouched. |

#### Wheel pipeline (verify-only)

`cafleet/mise.toml` `[tasks.publish]` already chains `//admin:install` → `//admin:build` → `//cafleet:build` → `uv publish` (verified at `cafleet/mise.toml:35-41`). The admin bundle inside `cafleet/src/cafleet/webui/` is rebuilt with the new `base: '/'` before the wheel is built, so the published wheel ships a working SPA. **No change needed.** This bullet exists in the doc so a reader does not wonder.

### Part B — Sort sessions latest-first

`cafleet/src/cafleet/broker.py:224` (`list_sessions`):

```python
.order_by(Session.created_at.desc(), Session.session_id.asc())
```

`session_id ASC` is the tiebreaker. `created_at` is microsecond-precision ISO, but `_now_iso()` is called once per `create_session()` and three back-to-back creates on a fast box can land in the same microsecond. The UUID-based tiebreaker is **stable** but **not** in creation order — UUID v4 is random. The tiebreaker therefore guarantees a single deterministic order for any given (created_at, session_id) input, but a test that creates several sessions in the same microsecond cannot assert "C, B, A from creation order" alone. The test below sidesteps the clock-resolution flakiness by stubbing `_now_iso()` to return distinct timestamps.

The frontend (`admin/src/components/SessionPicker.tsx`) renders sessions in the order returned by `listSessions()` with no `Array.prototype.sort` — verified by reading the component. No change needed there.

#### Test

`cafleet/tests/test_broker_registry.py` (alongside the existing `test_list_sessions__*` tests):

```python
def test_list_sessions__newest_first_by_created_at_desc(monkeypatch):
    # Force distinct, ascending timestamps so the assertion does not depend
    # on microsecond clock resolution. Iterator yields one per create_session() call.
    timestamps = iter([
        "2026-05-23T00:00:01.000000+00:00",
        "2026-05-23T00:00:02.000000+00:00",
        "2026-05-23T00:00:03.000000+00:00",
    ])
    monkeypatch.setattr(broker, "_now_iso", lambda: next(timestamps))

    _create_session(label="a")  # 00:00:01
    _create_session(label="b")  # 00:00:02
    _create_session(label="c")  # 00:00:03

    rows = broker.list_sessions()
    labels = [row["label"] for row in rows]

    assert labels == ["c", "b", "a"]
```

`monkeypatch` is a built-in pytest fixture; no new import is needed. Helper `_create_session` is already in `cafleet/tests/_broker_helpers.py` and used by the surrounding tests in the same file. `broker._now_iso()` is consumed inside `create_session()` (broker.py:122) for the `created_at` column under test; the stub returns a fresh value on each call.

### Part C — Polling

#### Hook: `admin/src/hooks/usePolling.ts` (NEW)

A small reusable hook plus the shared interval constant:

```ts
import { useEffect, useRef } from "react";

export const POLL_INTERVAL_MS = 5000;

/**
 * Calls `callback` every `intervalMs`. Skips a tick if the previous call is
 * still pending (in-flight guard). Always polls — does NOT pause on
 * document visibility changes. Errors thrown by `callback` are swallowed
 * (caller is responsible for surfacing them through component state).
 */
export function usePolling(
  callback: () => Promise<void>,
  intervalMs: number,
): void {
  const savedCallback = useRef(callback);
  const inFlight = useRef(false);

  useEffect(() => {
    savedCallback.current = callback;
  }, [callback]);

  useEffect(() => {
    const tick = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        await savedCallback.current();
      } catch {
        /* swallow — next tick re-attempts */
      } finally {
        inFlight.current = false;
      }
    };

    const timer = setInterval(() => {
      void tick();
    }, intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
}
```

Notes:

- `savedCallback` lets callers pass a fresh closure each render without resetting the interval.
- `inFlight` guard ensures no two requests to the same endpoint are in flight from this hook. Concurrency between *different* hooks (e.g. agents + timeline on `Dashboard`) is unaffected and intentional.
- No `visibilitychange` listener — polling fires regardless of tab visibility.
- No retry policy — silent swallow, next tick re-attempts.

#### Indicator pattern

Each component that uses the hook also tracks `isPolling: boolean` for its subtle "Updating…" indicator. The pattern is:

```tsx
const [isPolling, setIsPolling] = useState(false);

const poll = useCallback(async () => {
  setIsPolling(true);
  try {
    await loadX();
  } finally {
    setIsPolling(false);
  }
}, []);

usePolling(poll, POLL_INTERVAL_MS);
```

Indicator markup (consistent across the three views):

```tsx
{isPolling && (
  <span className="text-xs text-gray-400 italic">Updating…</span>
)}
```

Placement:

| View | Indicator location |
|---|---|
| `SessionPicker` | Next to the "Select a Session" heading inside the white card header row. |
| `Dashboard` | In the right-side header cluster, immediately before the manual "Refresh" button. |
| `Timeline` | Floating top-right of the timeline scroll container, `absolute top-2 right-2`, so it does not shift content. |

#### Wiring

`admin/src/components/SessionPicker.tsx`:

- Extract the inline `useEffect` body into a named `loadSessions` callback.
- Replace the single-shot `useEffect` with: an initial `useEffect(() => { void loadSessions(); }, [])` for first paint, plus `usePolling(loadSessions, POLL_INTERVAL_MS)` for subsequent ticks. (The hook fires its first tick after `intervalMs`, not immediately; the initial `useEffect` handles paint-zero.)
- Show `isPolling` indicator next to the heading.

`admin/src/components/Dashboard.tsx`:

- The existing `refreshAll` callback already does the right thing (fetches agents + bumps `refreshKey`). Wrap it once: `usePolling(refreshAll, POLL_INTERVAL_MS)`.
- Track `isPolling` inside `refreshAll` and show the indicator next to the manual Refresh button. **Keep the manual Refresh button.**
- Bumping `refreshKey` on every poll forces `Timeline` to refetch; this is the existing manual-refresh path and stays as is. `Timeline` additionally adds its own internal poll for the case where `Dashboard` re-renders without a `refreshKey` bump — see below.

`admin/src/components/Timeline.tsx`:

- Extract the inline `useEffect` body into a named `loadTimeline` callback that takes nothing and returns `Promise<void>`.
- Replace the existing `useEffect(..., [refreshKey])` with two effects:
  1. `useEffect(() => { void loadTimeline(); }, [refreshKey])` — preserves manual-refresh and post-send behavior.
  2. `usePolling(loadTimeline, POLL_INTERVAL_MS)` — self-driven 5 s tick.
- Track `isPolling` and render the floating indicator.
- Replace the unconditional `bottomRef.current?.scrollIntoView(...)` effect with a **follow-tail** version. The decision must compare against the scroll height from the *previous* render — by the time `useLayoutEffect` fires after a render driven by `[entries]`, React has already committed the new rows to the DOM, so `el.scrollHeight` already includes them. Comparing post-commit `scrollHeight` against unchanged `scrollTop` overcounts distance-from-bottom by exactly the height of the new rows and skips the auto-scroll when the user is at the bottom. The fix is to remember the *prior* `scrollHeight` in a ref:

  ```tsx
  const scrollerRef = useRef<HTMLDivElement>(null);
  const prevScrollHeightRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const NEAR_BOTTOM_PX = 80;
    const prev = prevScrollHeightRef.current;
    const wasNearBottom =
      prev === null ? true : prev - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
    if (wasNearBottom) {
      bottomRef.current?.scrollIntoView({ behavior: "auto" });
    }
    prevScrollHeightRef.current = el.scrollHeight;
  }, [entries]);
  ```

  Attach `ref={scrollerRef}` to the existing `<div className="flex-1 overflow-y-auto">`. The 80 px threshold is roughly one or two message rows in the current `TimelineMessage` layout — small enough that the first poll after the user scrolls up no longer satisfies "near bottom" and the follow-tail freezes.

  On the very first load (`entries` goes from `[]` to populated), `prevScrollHeightRef.current` is `null`, so `wasNearBottom` short-circuits to `true` and the first render snaps to the bottom — preserving today's "land at the latest message" UX.


- `useLayoutEffect` is imported from `react` alongside the existing `useEffect`/`useRef`/`useState` imports.

#### What polling does NOT change

- No new HTTP endpoints. Polling re-uses `listSessions()`, `getAgents()`, `fetchTimeline()`.
- No request deduplication across components (e.g. two open Dashboard tabs each poll independently). At this volume on `127.0.0.1`, not worth solving.
- The manual `Refresh` button on `Dashboard` stays. The `MessageInput` `onSent` callback still calls `refreshAll` so newly-sent messages appear immediately, not on the next tick.
- No frontend test infrastructure is added. Verification of Part C is `mise //admin:build` + manual inspection of the running SPA.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation-first per `.claude/rules/design-doc-numbering.md`. Then Part A, Part B, Part C, all on a single branch as a single PR. Each step ends green: `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format`, and (for steps touching `admin/`) `mise //admin:build` and `mise //admin:lint`.

### Step 1: Documentation update (aspirational, single commit)

- [x] Update `ARCHITECTURE.md`: every `/ui/` → `/`, every `/ui/api/*` → `/api/*` (lines 12, 65, 282, 320, 323, 326). Add a sentence in the WebUI section noting auto-refresh every 5 s and the SessionPicker's newest-first ordering. <!-- completed: 2026-05-23T08:20 -->
- [x] Update `CONTRIBUTING.md` line 33: `(required before /ui/ is served)` → `(required before / is served)`. <!-- completed: 2026-05-23T08:20 -->
- [x] Update `docs/spec/webui-api.md`: replace the `Base path: /ui/api` header with `Base path: /api`; rewrite the six endpoint headings (`GET /ui/api/sessions`, `GET /ui/api/agents`, …) to use `/api/...`; update the 409-reservation paragraph's `POST /ui/api/messages/send` reference. Add a sentence noting `GET /api/sessions` returns rows newest-first by `created_at DESC, session_id ASC`. <!-- completed: 2026-05-23T08:20 -->
- [x] Update `docs/spec/data-model.md` line 147: `/ui/api/*` → `/api/*`. <!-- completed: 2026-05-23T08:21 -->
- [x] Update `docs/spec/cli-options.md` lines 119, 284, 305: `/ui/api/*` → `/api/*`, `/ui/` → `/`, and rewrite the warning-text backtick to `warning: admin WebUI is not built. / will return 404. Run 'mise //admin:build'.` to match the post-change server output. <!-- completed: 2026-05-23T08:21 -->
- [x] Update `.claude/rules/commands.md` line 14: `Serves /ui/ only after mise //admin:build` → `Serves / only after mise //admin:build`. <!-- completed: 2026-05-23T08:28 -->

- [x] Verify `README.md` and every `skills/*/SKILL.md` contain zero `/ui` references — `grep -n '/ui' README.md` and `grep -rln '/ui' skills/` must return nothing. If any reference exists, update it in this step. <!-- completed: 2026-05-23T08:24 -->

### Step 2: Part A — `/ui → /` cut-over

- [x] In `cafleet/src/cafleet/server.py`: change `app.mount("/ui", ...)` to `app.mount("/", ...)`, update the missing-bundle warning to `/ will return 404`, update the module docstring, add the mount-order comment above the mount call, and replace `SPAStaticFiles.get_response` with the reserved-prefix re-raise variant from §Part A Backend. <!-- completed: 2026-05-23T08:35 -->
- [x] In `cafleet/src/cafleet/webui_api.py`: change `APIRouter(prefix="/ui/api")` to `APIRouter(prefix="/api")`, update the module docstring. <!-- completed: 2026-05-23T08:35 -->
- [x] In `admin/vite.config.ts`: `base: '/'` and proxy key `'/api'`. <!-- completed: 2026-05-23T08:35 -->
- [x] In `admin/src/api.ts`: fetch template `` `/api${path}` ``. <!-- completed: 2026-05-23T08:35 -->
- [x] In `cafleet/tests/test_server_cli.py:126`: update the assertion to match the new warning text. Run `mise //cafleet:test`. <!-- completed: 2026-05-23T08:36 -->
- [x] Run `mise //admin:build`, then `mise //cafleet:dev` (or `cafleet server`), and manually load `http://127.0.0.1:8000/` to confirm the SPA renders. Verify with `curl -sI` (or browser DevTools) that: `GET /ui/` returns 404, `GET /ui/foo` returns 404, `GET /api/sessions` returns JSON 200, and `GET /api/does-not-exist` returns JSON 404 (NOT HTML). <!-- completed: 2026-05-23T08:40 by Verifier via agent-browser --> <!-- NOTE(programmer): admin:build ran cleanly (root-relative asset paths confirmed); runtime SPA + curl/route smoke routed to Verifier via agent-browser. `mise //cafleet:dev` is running in the background on http://127.0.0.1:8000 for the Verifier to probe. -->


### Step 3: Part B — sessions sorted newest-first

- [x] In `cafleet/src/cafleet/broker.py:224`: `.order_by(Session.created_at.desc(), Session.session_id.asc())`. <!-- completed: 2026-05-23T08:42 -->
- [x] In `cafleet/tests/test_broker_registry.py`: add `test_list_sessions__newest_first_by_created_at_desc` exactly as specified in §Part B. Run `mise //cafleet:test`. <!-- completed: 2026-05-23T08:42 -->

### Step 4: Part C — polling hook and wiring

- [x] Create `admin/src/hooks/usePolling.ts` with `POLL_INTERVAL_MS = 5000` and the hook implementation specified in §Part C. <!-- completed: 2026-05-23T08:47 -->
- [x] Wire `SessionPicker.tsx`: extract `loadSessions`, keep the initial `useEffect`, add `usePolling(loadSessions, POLL_INTERVAL_MS)`, add `isPolling` indicator next to the "Select a Session" heading. <!-- completed: 2026-05-23T08:47 -->
- [x] Wire `Dashboard.tsx`: add `usePolling(refreshAll, POLL_INTERVAL_MS)`, track `isPolling` inside `refreshAll`, render the indicator immediately before the existing manual Refresh button (button stays). <!-- completed: 2026-05-23T08:47 -->
- [x] Wire `Timeline.tsx`: extract `loadTimeline`, add the `refreshKey`-driven `useEffect` and `usePolling(loadTimeline, POLL_INTERVAL_MS)` side by side, switch the auto-scroll effect to the `prevScrollHeightRef` follow-tail pattern with 80 px threshold from §Part C, render the floating indicator. <!-- completed: 2026-05-23T08:47 -->
- [x] Run `mise //admin:build` and `mise //admin:lint`, then load the SPA against a session with a few messages. Verify: the agent list refreshes when an external `cafleet agent register` happens; the timeline appends new messages when an external `cafleet message send` happens; the "Updating…" indicator appears briefly each ~5 s; the manual Refresh button still works; scrolling up in Timeline freezes follow-tail until the user scrolls back near the bottom; switching tabs (hiding the page) does NOT pause polling — verified by watching the server log. <!-- completed: 2026-05-23T08:56 by Verifier; the scrolled-up follow-tail branch is code-spec PASS but not runtime-probed (DOM scroll-set denied to Verifier); recommend operator spot-check --> <!-- NOTE(programmer): mise //admin:build (28 modules → webui/) and mise //admin:lint both passed cleanly. Runtime SPA verification (polling indicator, manual Refresh, follow-tail under user scroll, no-pause-on-hidden) routed to Verifier via agent-browser per the precedent set in Step 2.6. `mise //cafleet:dev` is still running in the background on http://127.0.0.1:8000. -->


---

## Risks

- **Mount order regression.** If a future contributor reorders `create_app()` so the SPA mount precedes `include_router`, the FastAPI router never sees `/api/*` requests because the catch-all mount intercepts them first. The SPAStaticFiles re-raise from §Part A means unknown `api/...` will 404 cleanly, but known routes will also 404 — silently breaking the whole admin. No automated test guards this; the inline comment above the mount call is the only safeguard. If this ever bites, add a server test asserting `GET /api/agents` returns JSON 400 (the expected "X-Session-Id header required" response from `get_webui_session`) rather than 404.

- **Polling against a paused backend.** If the FastAPI process pauses (long DB lock, debugger breakpoint), each component's in-flight guard ensures no second request stacks up. When the server unblocks, all three guards release in order. No mutex or queue needed.
- **Stale build directory.** A `cafleet/src/cafleet/webui/` built before this change still references `/ui/` asset paths and 404s under the new mount. Mitigation: Step 2 ends with `mise //admin:build`; `.gitignore` already excludes the bundle so no stale assets land in git. The wheel pipeline (`mise //cafleet:publish`) rebuilds the bundle before packing, so published wheels are correct.
- **Follow-tail edge cases.** A user who scrolls up exactly within the 80 px threshold may still get yanked. 80 px is roughly one to two message rows in the current TimelineMessage layout and matches the convention used by chat UIs that follow-tail (Discord, Slack web). Tunable later if a user reports it.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-23 | Initial draft. |
