# Upload skills artifacts on release

**Status**: Complete
**Progress**: 3/3 tasks complete
**Last Updated**: 2026-06-22

## Overview

Attach a downloadable `cafleet-skills-v<version>.zip` asset to every published GitHub Release so the cafleet skills can be obtained without cloning the repository. The packaging runs as a new, independent job in `.github/workflows/publish.yml`.

## Success Criteria

- [x] A new `upload-skills` job in `publish.yml` is triggered by the existing `release: published` event.
- [x] On a published release, a `cafleet-skills-v<tag>.zip` asset appears on that release's page.
- [x] The archive contains only the three skill directories under `skills/` (`cafleet`, `cafleet-design-doc`, `cafleet-research`); no `.claude-plugin/`, `.codex-plugin/`, or other manifest files.
- [x] The new job runs in parallel with the existing `publish` job (no `needs:`); a failure in either job does not block the other.
- [x] The new job declares its own `permissions: contents: write`; the existing `publish` job remains `contents: read`.
- [x] The asset is produced for all `release: published` events, including pre-releases.

---

## Background

`.github/workflows/publish.yml` triggers on `release: published` and runs a single `publish` job (`contents: read`, `id-token: write`) that calls `mise //cafleet:publish` (builds admin assets, builds the wheel, runs `uv publish`).

The three skills live under `skills/` (`cafleet`, `cafleet-design-doc`, `cafleet-research`). Today the only way to obtain them is to clone the repository (installation runs `gh skill install ./ --from-local` via `mise //:skill-install`). Publishing a self-contained skills archive on the Release page makes preparing the skills easier for downstream consumers.

Release tags in this repository are bare semantic versions without a `v` prefix (e.g. `0.10.0`), so `github.event.release.tag_name` resolves to a value like `0.12.2`. The requested filename embeds an explicit `v`, yielding `cafleet-skills-v0.12.2.zip`.

---

## Specification

### Delivery surface

The asset is a **permanent, public Release-page asset**, attached via the GitHub CLI (`gh release upload`). This is deliberately not `actions/upload-artifact`, which only attaches ephemeral, retention-limited files to the workflow run behind the Actions tab.

### Archive content and naming

| Property | Value |
|----------|-------|
| Contents | The `skills/` directory and its three skill subdirectories (`cafleet`, `cafleet-design-doc`, `cafleet-research`) |
| Format | `.zip` |
| Filename | `cafleet-skills-v${{ github.event.release.tag_name }}.zip` |

`zip -r "cafleet-skills-v<tag>.zip" skills` run from the repository root yields an archive whose entries are prefixed with `skills/`, so the archive unpacks to a single top-level `skills/` folder.

Excluding the manifests is **structural, not a filter**. `.claude-plugin/`, `.codex-plugin/`, `marketplace.json`, and `plugin.json` all live at the repository root as siblings of `skills/`; the three directories under `skills/` contain only `SKILL.md` files plus skill subdirectories, with no manifest of any kind. Packaging the `skills/` directory alone therefore excludes every manifest purely by scope, so success criterion #3 holds by construction. No `-x` exclude patterns are needed — do not add any.

### Job structure and permissions

The new `upload-skills` job runs **independently and in parallel** with `publish` — no `needs:` between them. A PyPI publish failure must not block the skills upload, and vice versa.

The new job declares its own job-level `permissions: contents: write` (required by `gh release upload` to attach an asset). The existing `publish` job is unchanged and keeps `contents: read` + `id-token: write`. `gh` authenticates with `GH_TOKEN: ${{ github.token }}`; `gh` and `zip` are preinstalled on GitHub-hosted `ubuntu-24.04` runners, so this job needs neither `mise` nor any extra setup.

### Releases covered

No `if:` filter is applied. The job inherits the workflow's `release: published` trigger, which fires for stable releases and pre-releases alike — matching the existing `publish` job's coverage.

### Idempotency

The upload uses `gh release upload ... --clobber` so a re-run of the workflow (or a manually re-published release) overwrites an existing same-named asset instead of failing on a name collision.

### Resulting workflow

```yaml
name: Publish

on:
  release:
    types:
      - published

jobs:
  publish:
    runs-on: ubuntu-24.04
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
      - uses: jdx/mise-action@1648a7812b9aeae629881980618f079932869151 # v4.0.1
        with:
          experimental: true
      - run: mise //cafleet:publish

  upload-skills:
    runs-on: ubuntu-24.04
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
      - name: Package skills directory
        run: zip -r "cafleet-skills-v${{ github.event.release.tag_name }}.zip" skills
      - name: Upload skills archive to the release
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh release upload "${{ github.event.release.tag_name }}" "cafleet-skills-v${{ github.event.release.tag_name }}.zip" --clobber
```

### Documentation surface

No README, `docs/`, or `SKILL.md` changes are made — a deliberate scoping decision, not an oversight. Per the clarification round, consumer-side skill installation will be **automated**: an out-of-scope mechanism fetches the Release-page archive directly, so consumers are not expected to manually discover and download it, and there is no manual acquisition path to document. This change also adds no CLI, API, or architectural surface. Discoverability documentation is intentionally deferred until (and only if) a manual install path is later introduced. The change is scoped entirely to the CI workflow.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Add the `upload-skills` job to `publish.yml`

- [x] Add the `upload-skills` job exactly as shown in *Resulting workflow*: `contents: write`, pinned `actions/checkout`, the `zip -r` packaging step, and the `gh release upload --clobber` step with `GH_TOKEN: ${{ github.token }}`. <!-- completed: 2026-06-22T11:45 -->
- [x] Confirm the existing `publish` job is untouched (still `contents: read` + `id-token: write`, no `needs:` added). <!-- completed: 2026-06-22T11:45 -->

### Step 2: Validate the workflow

- [x] Parse the workflow to confirm it is well-formed YAML. `actionlint` is not part of the repo toolchain (no `mise` task wraps it), so run the one-off parse `uv run --with pyyaml python -c "import yaml; yaml.safe_load(open('.github/workflows/publish.yml'))"` — it exits non-zero on a syntax error. Then confirm by inspection that the `zip` target is `skills` and the asset filename is `cafleet-skills-v${{ github.event.release.tag_name }}.zip`. <!-- completed: 2026-06-22T11:48 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-22 | Initial draft |
| 2026-06-22 | Implemented; collapsed the upload step's `run:` to a single line per Administrator PR review on #144 |
| 2026-06-22 | Marked complete |
