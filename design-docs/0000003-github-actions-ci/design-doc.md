# GitHub Actions CI Pipeline

**Status**: Complete
**Progress**: 6/6 tasks complete
**Last Updated**: 2026-03-29

## Overview

Add a GitHub Actions CI workflow to the Hikyaku monorepo that runs linting (`ruff check`) and tests (`pytest`) for both packages (`registry/` and `client/`) on every push to `main` and pull request.

## Success Criteria

- [ ] `ruff check` runs against the entire codebase and fails the build on violations
- [ ] Registry tests (`cd registry && uv run pytest tests/ -v`) pass in CI
- [ ] Client tests (`cd client && uv run pytest tests/ -v`) pass in CI
- [ ] Lint and test jobs run on every push to `main` and every pull request
- [ ] Registry and client tests run as independent parallel jobs with separate pass/fail reporting

---

## Background

The project currently has no CI pipeline. Tests are run locally by developers. As the project grows, automated testing on every PR and push to `main` is needed to catch regressions early. All tests are fully offline — registry tests use `fakeredis` and client tests use mocked `httpx` — so no external services are required.

---

## Specification

### Workflow File

Path: `.github/workflows/ci.yml`

### Trigger Events

```yaml
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
```

No scheduled runs or manual dispatch.

### Job Structure

Three jobs, with test jobs running in parallel:

| Job | Depends On | Purpose |
|---|---|---|
| `lint` | — | Run `ruff check .` on the entire codebase |
| `test-registry` | — | Run `pytest` in `registry/` |
| `test-client` | — | Run `pytest` in `client/` |

All three jobs run in parallel (no inter-job dependencies).

### Common Job Setup

Each job uses the same setup steps:

| Step | Action / Command |
|---|---|
| Checkout | `actions/checkout@v6` |
| Install uv | `astral-sh/setup-uv@v7` with `enable-cache: true` (caches `~/.cache/uv`) |
| Install Python | `uv python install 3.13` (uv manages the Python installation). 3.13 is the latest stable release, matching the development environment; the project requires >=3.12. |
| Install dependencies | `uv sync` (from project root; installs all workspace deps including dev group) |

Runner: `ubuntu-latest`

**Note on uv caching**: `astral-sh/setup-uv@v7` has built-in caching support via the `enable-cache` option. This caches the uv dependency cache directory (`~/.cache/uv`), avoiding redundant downloads across runs.

### Lint Job

```yaml
lint:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - uses: astral-sh/setup-uv@v7
      with:
        enable-cache: true
    - run: uv python install 3.13
    - run: uv sync
    - run: uv run ruff check .
```

`ruff` is not currently listed as a project dependency. It must be added to the root `pyproject.toml` dev dependency group:

```toml
[dependency-groups]
dev = [
    # ... existing deps ...
    "ruff>=0.11.0",
]
```

No `ruff.toml` or `[tool.ruff]` configuration exists. `ruff check` will use defaults initially. Custom rules can be added later as needed.

### Test Jobs

Each test job uses `working-directory` to run pytest inside the package directory:

```yaml
test-registry:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - uses: astral-sh/setup-uv@v7
      with:
        enable-cache: true
    - run: uv python install 3.13
    - run: uv sync
    - run: uv run pytest tests/ -v
      working-directory: registry

test-client:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - uses: astral-sh/setup-uv@v7
      with:
        enable-cache: true
    - run: uv python install 3.13
    - run: uv sync
    - run: uv run pytest tests/ -v
      working-directory: client
```

### Branch Protection (Recommendation)

Outside the scope of this workflow file, but recommended: configure GitHub branch protection on `main` to require all three CI jobs (`lint`, `test-registry`, `test-client`) to pass before merging PRs.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Add ruff to Dev Dependencies

- [x] Add `ruff>=0.11.0` to `[dependency-groups] dev` in root `pyproject.toml` <!-- completed: 2026-03-29T01:48 -->
- [x] Run `uv sync` to update `uv.lock` <!-- completed: 2026-03-29T01:48 -->

### Step 2: Create CI Workflow

- [x] Create `.github/workflows/ci.yml` with the three-job structure (lint, test-registry, test-client) as specified above <!-- completed: 2026-03-29T01:49 -->

### Step 3: Fix Lint Violations

- [x] Run `ruff check .` locally and fix any violations <!-- completed: 2026-03-29T01:51 -->

### Step 4: Verify

- [x] Push a branch and confirm all three CI jobs pass on the resulting PR <!-- completed: 2026-03-29T01:58 -->
- [x] Verify that lint failures and test failures are reported independently per job <!-- completed: 2026-03-29T01:58 -->

---

## Changelog (optional -- spec revisions only, not implementation progress)

| Date | Changes |
|------|---------|
| 2026-03-28 | Initial draft |
| 2026-03-29 | Updated actions/checkout v4→v6, astral-sh/setup-uv v6→v7 |
| 2026-03-29 | Approved |
| 2026-03-29 | Complete |
