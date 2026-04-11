# Registry-Bundled WebUI Assets

**Status**: Complete
**Progress**: 15/15 tasks complete
**Last Updated**: 2026-04-11

## Overview

Move the admin SPA build output from `admin/dist/` into the registry Python package (`registry/src/hikyaku_registry/webui/`) and ship it inside the `hikyaku-registry` wheel, so a single `pip install hikyaku-registry` produces a runnable broker that serves `/ui/` without any external file lookup.

## Success Criteria

- [x] `mise //admin:build` writes the SPA to `registry/src/hikyaku_registry/webui/index.html` (and sibling assets)
- [x] `mise //registry:dev` serves the built SPA at `http://localhost:8000/ui/` with no extra steps (verified in-process via `test_default_mount_serves_spa` integration test)
- [x] `uv build` (run after `mise //admin:build`) produces a wheel whose contents include `hikyaku_registry/webui/index.html`
- [x] `create_app()`'s default `webui_dist_dir` resolves to a path inside the installed package, not a sibling repo directory
- [x] `ARCHITECTURE.md`, `README.md`, and `.claude/rules/commands.md` document the new layout and the manual two-step build order
- [x] `admin/dist/` is no longer produced by the build (Vite `outDir` redirected away from `admin/`)
- [x] `.gitignore` entry for `admin/dist/` is replaced with the new `registry/src/hikyaku_registry/webui/` path

---

## Background

The admin SPA currently builds to `admin/dist/` at the repo root. The registry server reaches outside its own package to load it:

```python
# registry/src/hikyaku_registry/main.py:331-334
if webui_dist_dir is None:
    webui_dist_dir = str(
        Path(__file__).resolve().parent.parent.parent.parent / "admin" / "dist"
    )
```

This works in a source checkout but breaks for any installed wheel — `Path(__file__).parent.parent.parent.parent` lands somewhere in `site-packages`, not at a repo root that contains `admin/`. The registry wheel's `[tool.hatch.build.targets.wheel].include` only ships `alembic.ini` and `alembic/**/*`; nothing under `admin/` is bundled. The result: `pip install hikyaku-registry` produces a server that can never serve the WebUI.

The user's stated intent is "registry と一緒に web server としてホストしてほしい" — the WebUI must travel with the registry process as a single shippable unit.

---

## Specification

### Target layout

```
registry/
  src/hikyaku_registry/
    webui/                      # NEW — Vite build output (gitignored)
      index.html
      assets/
        index-<hash>.js
        index-<hash>.css
      ...
    main.py                     # default webui_dist_dir resolves here
    alembic/
    ...
admin/
  vite.config.ts                # outDir points OUT of admin/ into registry package
  src/                          # SPA sources unchanged
  (no dist/ directory)          # admin/dist/ is permanently retired
```

### Vite build config

`admin/vite.config.ts` adds an explicit `build` block:

```ts
export default defineConfig({
  plugins: [
    react(),
    babel({ presets: [reactCompilerPreset()] }),
    tailwindcss(),
  ],
  build: {
    outDir: '../registry/src/hikyaku_registry/webui',
    emptyOutDir: true,
  },
  server: {
    proxy: {
      '/ui/api': {
        target: 'http://localhost:8000',
        changeOrigin: true,
      },
    },
  },
})
```

`emptyOutDir: true` is required because Vite refuses by default to clear a directory outside the project root. The dev server (`mise //admin:dev`) is unaffected — it does not write to disk.

### Runtime path resolution

`create_app()` in `registry/src/hikyaku_registry/main.py` is updated to default to a package-relative path via a small helper, so the resolution logic is independently unit-testable:

```python
def _default_webui_dist_dir() -> Path:
    return Path(__file__).resolve().parent / "webui"


def create_app(...) -> FastAPI:
    ...
    if webui_dist_dir is None:
        webui_dist_dir = str(_default_webui_dist_dir())
    dist_path = Path(webui_dist_dir)
    if dist_path.exists():
        app.mount("/ui", SPAStaticFiles(directory=str(dist_path)), name="webui")
```

The existing `dist_path.exists()` guard is preserved: if a contributor has not yet run `mise //admin:build`, the server starts cleanly and `/ui/` simply 404s. The JSON-RPC and Registry REST surfaces remain fully functional. Tests already inject `webui_dist_dir` via the `mounted_app` fixture (`registry/tests/test_webui_mount.py:36`), so they are unaffected by the default change. The new helper exists specifically so a regression test can assert the default path stays inside the installed package without spinning up a full app.

### Wheel packaging

`registry/pyproject.toml` extends `[tool.hatch.build.targets.wheel].include`:

```toml
[tool.hatch.build.targets.wheel]
packages = ["src/hikyaku_registry"]
include = [
  "src/hikyaku_registry/alembic.ini",
  "src/hikyaku_registry/alembic/**/*",
  "src/hikyaku_registry/webui/**/*",
]
```

Wheel-only. The sdist is intentionally NOT extended — `pip install hikyaku-registry --no-binary` is out of scope.

### Build orchestration — explicitly NO coupling

`mise //registry:dev` and `mise //registry:test` are **not** modified. Backend-only contributors are not forced to install bun. The two-step manual order is documented:

1. `mise //admin:build` — produces `registry/src/hikyaku_registry/webui/`
2. `mise //registry:dev` — serves the result at `/ui/`

If step 1 is skipped, the server still runs; only `/ui/` 404s.

For wheel builds, the same manual order applies: run `mise //admin:build` before `uv build`. There is no hatch build hook enforcing this — documentation is the only enforcement mechanism. **Ownership**: the release maintainer is responsible for running `mise //admin:build` and the wheel-listing verification (`unzip -l dist/*.whl | grep webui/index.html`) before every `uv build`. This procedure is documented in the README's "Build the WebUI" subsection (added in Step 1) so there is exactly one canonical home for the manual order. There is no separate release runbook in this repo, and one is not introduced here.

### Verification scope & residual risk

Acceptance is verified at three levels — `mise //admin:build` produces the expected file, `mise //registry:dev` serves it locally, and `unzip -l dist/*.whl` confirms the wheel archive contains it. A fresh-venv `pip install dist/*.whl` smoke test (which would prove that the package-relative `Path(__file__).resolve().parent / "webui"` resolves correctly from a real `site-packages` layout) is **intentionally out of scope** per user decision. The wheel file listing proves the asset is in the archive but does NOT prove runtime path resolution from an installed package. This residual risk is accepted because:

- The `Path(__file__).resolve().parent / "webui"` pattern is the same pattern used today by `alembic.ini` and `alembic/**/*` (also bundled via `[tool.hatch.build.targets.wheel].include`), and that bundling is known to work for `hikyaku-registry db init`.
- The project has no release workflow yet (no `.github/workflows/release.*`), so adding a fresh-venv install test would be process overhead with no automation surface to wire it into.
- If the install path ever becomes load-bearing (e.g. when a release workflow is introduced), the missing smoke test should be added at that time, not pre-emptively here.

### .gitignore

Replace the `admin/dist/` line with the new package-internal location:

```gitignore
# WebUI SPA build output and dependencies
registry/src/hikyaku_registry/webui/
admin/node_modules/
.vite
```

Note: `admin/dist/` is removed entirely, since Vite no longer writes there.

### Affected files

| File | Change |
|---|---|
| `admin/vite.config.ts` | Add `build.outDir` + `build.emptyOutDir` |
| `registry/src/hikyaku_registry/main.py` | Default `webui_dist_dir` to `Path(__file__).resolve().parent / "webui"` |
| `registry/pyproject.toml` | Add `src/hikyaku_registry/webui/**/*` to wheel include |
| `.gitignore` | Replace `admin/dist/` with `registry/src/hikyaku_registry/webui/` |
| `ARCHITECTURE.md` | Update "WebUI / Static serving" line (currently `admin/dist/`) |
| `README.md` | Rewrite line 246 prose + add new "Build the WebUI" subsection in the Development section |
| `.claude/rules/commands.md` | Add a one-line note: "`//registry:dev` serves `/ui/` only after `//admin:build` has been run" |

Files verified to need NO change:
- `.claude/skills/hikyaku/SKILL.md` (does not reference `admin/dist`)
- `docs/spec/*.md` (no `admin/dist` references)
- `registry/tests/test_webui_mount.py` (tests inject `webui_dist_dir` explicitly)
- No `plugins/` directory exists in this repo

### Alternatives considered

| Alternative | Why rejected |
|---|---|
| Keep `admin/dist/`, add a copy step to mirror it into the package | Two source-of-truth directories; copy step is invisible failure surface; doesn't simplify anything |
| Couple `//registry:dev` to `//admin:build` via mise `depends` | Forces bun on backend-only contributors; rebuilds on every dev start; rejected by user |
| Custom hatch build hook to invoke `bun run build` from `uv build` | Adds bun dependency to every wheel build environment, including CI lint jobs; rejected for simplicity |
| Ship webui in sdist as well | `pip install --no-binary` is not a supported install path for this project; out of scope |
| Use directory name `static/` or `_webui/` instead of `webui/` | `webui/` matches existing terminology in codebase (`webui_router`, `webui_dist_dir`, `webui_api.py`); rename adds churn for no gain |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates (BEFORE any code)

Per `.claude/rules/design-doc-numbering.md`, documentation is updated first.

- [x] Update `ARCHITECTURE.md` line 218: replace `\`StaticFiles\` mount at \`/ui\` serves \`admin/dist/\` (production build)` with the new bundled-package location and a note that `mise //admin:build` is required before `mise //registry:dev` serves it <!-- completed: 2026-04-11T00:00 -->
- [x] Update `README.md` line 246: rewrite "It is built from `admin/`…" to state that the build output is bundled inside `registry/src/hikyaku_registry/webui/` and ships in the `hikyaku-registry` wheel <!-- completed: 2026-04-11T00:00 -->
- [x] Update `README.md` Development section (around lines 282-300): add a "Build the WebUI" subsection that explicitly shows the manual two-command order — `mise //admin:build` first, then `mise //registry:dev` — and notes that the release maintainer must also run `mise //admin:build` before any `uv build` <!-- completed: 2026-04-11T00:00 -->
- [x] Update `.claude/rules/commands.md` to add a one-line note under "Start broker server": `//registry:dev` serves `/ui/` only after `//admin:build` has been run <!-- completed: 2026-04-11T00:00 -->
- [x] Use the Grep tool with pattern `admin/dist` and paths `.claude/skills/` and `docs/` to confirm no SKILL.md or spec doc still references `admin/dist`. If any hit is found, replace it with `registry/src/hikyaku_registry/webui/` in the same edit. <!-- completed: 2026-04-11T00:00 -->

### Step 2: Vite config

- [x] Edit `admin/vite.config.ts`: add `build: { outDir: '../registry/src/hikyaku_registry/webui', emptyOutDir: true }` <!-- completed: 2026-04-11T00:00 -->
- [x] Run `mise //admin:build` and verify `registry/src/hikyaku_registry/webui/index.html` exists <!-- completed: 2026-04-11T00:00 -->

### Step 3: Runtime default

- [x] Edit `registry/src/hikyaku_registry/main.py`: add a module-level helper `def _default_webui_dist_dir() -> Path: return Path(__file__).resolve().parent / "webui"` and replace the existing `webui_dist_dir = str(Path(__file__).resolve().parent.parent.parent.parent / "admin" / "dist")` block in `create_app()` with `webui_dist_dir = str(_default_webui_dist_dir())` (still gated on `if webui_dist_dir is None`). <!-- completed: 2026-04-11T00:00 -->
- [x] Add a regression test in `registry/tests/test_webui_mount.py`: new class `TestDefaultWebuiDistDir` with one test `test_default_points_inside_package` that imports `hikyaku_registry` and `_default_webui_dist_dir` from `hikyaku_registry.main`, then asserts `_default_webui_dist_dir() == Path(hikyaku_registry.__file__).resolve().parent / "webui"`. This protects against future package-layout refactors silently breaking the install-time path. <!-- completed: 2026-04-11T00:00 -->
- [x] Add a Tester-owned integration test in `registry/tests/test_webui_mount.py` that constructs `create_app()` with NO `webui_dist_dir` argument (exercising the default path), uses `httpx.AsyncClient` + `ASGITransport` to `GET /ui/`, and asserts status 200 + `text/html` + body starts with `<!doctype html>`. Guard with `pytest.skip` if `<package>/webui/index.html` does not exist so CI environments without a prior `mise //admin:build` do not fail. <!-- completed: 2026-04-11T00:00 -->
- [x] Run `mise //registry:test` to confirm the existing `test_webui_mount.py` suite still passes (the fixture injects `webui_dist_dir` so the default change is invisible to existing tests) <!-- completed: 2026-04-11T00:00 -->

### Step 4: Wheel packaging

- [x] Edit `registry/pyproject.toml`: add `"src/hikyaku_registry/webui/**/*"` to `[tool.hatch.build.targets.wheel].include` <!-- completed: 2026-04-11T00:00 -->
- [x] From the project root, run `mise //admin:build` followed by `uv build --package hikyaku-registry` and verify `unzip -l dist/hikyaku_registry-*.whl` lists `hikyaku_registry/webui/index.html` <!-- completed: 2026-04-11T00:00 -->

### Step 5: .gitignore + contributor migration note

- [x] Update `.gitignore`: replace `admin/dist/` with `registry/src/hikyaku_registry/webui/` <!-- completed: 2026-04-11T00:00 -->
- [x] Add a one-line contributor migration note in the README's "Build the WebUI" subsection (added in Step 1): "Existing checkouts pulled from before this change may have a stale `admin/dist/` directory; run `rm -rf admin/dist` once after pulling. The directory is no longer produced by `mise //admin:build`." <!-- completed: 2026-04-11T00:00 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-11 | Initial draft |
| 2026-04-11 | Reviewer pass 1 revisions: fixed task counter (12→15 — split README task added 1, regression-test task added 1, original count was actually 13); rewrote README task to spell out line 246 + Development subsection literally (B2); added "Verification scope & residual risk" subsection acknowledging the omitted fresh-venv smoke test (B3); converted Step 5 hygiene note into a concrete README contributor-migration task (I1); added regression-test task in Step 3 for the default `webui_dist_dir` (I2); named the release maintainer as owner of the manual `mise //admin:build` step in the Build orchestration subsection (I3); split Success Criterion #6 into two checkboxes (I4) |
| 2026-04-11 | User approved; Status → Approved; Affected files README cell tightened |
| 2026-04-11 | Implementation complete on branch `feat/registry-bundled-webui`. 7 commits: 0ee435c (docs), d02f285 (vite outDir), d4f0760 (regression test), c5ff94b (main.py helper), c2ac006 (integration test), 3a22dee (wheel include), 3bf9b2b (gitignore). 15/15 tasks, 339 tests pass. Verifier confirmed ALL CRITERIA SATISFIED. Step 3 task #3 was pivoted mid-execution from a curl-based live-server check to an in-process `httpx.AsyncClient` + `ASGITransport` integration test after user interjection. Status → Complete. |
