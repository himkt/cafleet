## Questions

1. [Section: Overview] Confirm the primary intent of 'cafleet setup': is it strictly to collapse 'gh skill install (per agent) + cafleet db init' into one end-user command, with no change to what those steps do? | Options: A) Yes, pure consolidation of existing onboarding B) Also intended to become the canonical/only documented user path C) Broader scope (also future config bootstrap)
2. [Section: Overview] Should 'cafleet setup' be safe/recommended to run from inside the repo working tree, or is it strictly an end-user (installed-CLI) command? | Options: A) End-user only; contributors use mise //:skill-install B) Both, but in-repo it still pulls Release skills (may shadow working-tree skills) C) Detect in-repo and refuse/warn
3. [Section: Success Criteria] 'fully idempotent' is asserted, but latest-stable can change between runs. Is idempotency scoped to a fixed resolved release (same release -> same tree), not across time? | Options: A) Per-resolved-release only (reword criterion) B) Guarantee identical across time C) Leave as-is
4. [Section: Background] Should the skills version be pinned to the installed 'cafleet' CLI version instead of latest-stable, to avoid CLI/skills version skew? | Options: A) Always latest stable (current) B) Default to the running CLI version C) Latest, but warn on mismatch
5. [Section: Command surface] When '--agent' repeats the same value, should setup dedupe silently or error? | Options: A) Dedupe silently B) Error on duplicate
6. [Section: Command surface] Is '--agent' allowed alongside the default (both halves) so it scopes only the skills targets while DB still migrates, or does '--agent' imply skills-only? | Options: A) Allowed; DB still runs B) '--agent' implies --skills-only C) Error unless combined with --skills-only
7. [Section: Command surface] Should a '--dry-run' flag exist to preview resolved targets + release + asset without writing? | Options: A) No B) Yes, add --dry-run
8. [Section: Command surface] Should '--release-tag' accept both 'v'-prefixed and bare tags from the user (doc only normalizes the resolved tag_name)? | Options: A) Accept both forms B) Bare semver only C) Whatever GitHub resolves
9. [Section: Source constants] Should 'HTTP_TIMEOUT' (30s, fixed) be overridable via env/flag for slow networks or large assets? | Options: A) Fixed 30s B) Env var override C) CLI flag override
10. [Section: Source constants] Should 'AGENT_SKILLS_DIRS' paths be overridable (env) for non-standard agent installs, or are the three hardcoded paths final? | Options: A) Hardcoded final B) Allow env override
11. [Section: Skills half] Auto-detect treats an agent as present if its home OR skills dir exists. If only the home exists (no skills dir), should setup create the skills dir or skip until explicitly targeted? | Options: A) Create skills dir (current) B) Require explicit --agent
12. [Section: Skills half] When '--agent X' names an agent whose home does not exist, should setup create the home/skills tree anyway (mkdir parents) or error? | Options: A) Create anyway (current implication) B) Error: home not found
13. [Section: Skills half] Should setup honor a GITHUB_TOKEN (if set) to lift the 60/hr anonymous API limit, or stay strictly unauthenticated? | Options: A) Strictly anonymous B) Use GITHUB_TOKEN when present
14. [Section: Skills half] Should the zip be validated against path-traversal / zip-slip ('..', absolute members) before extraction, even though it is the project's own release asset? | Options: A) Yes, reject traversal B) Trust own release archive
15. [Section: Skills half] If the archive's 'skills/' contains directories beyond SKILL_DIRS, are extras silently ignored (only the three installed) or treated as malformed? | Options: A) Ignore extras (install only the three) B) Treat extras as malformed
16. [Section: Skills half] Replace semantics ('rmtree' then 'copytree') destroy any local user edits in target skill dirs. Replace silently, back up first, or confirm? | Options: A) Replace silently B) Back up before replace C) Warn/confirm
17. [Section: Skills half] Confirm the ordering guarantee: download+extract+validate fully completes BEFORE any rmtree, so a network/archive failure never leaves a target with skills removed-but-not-replaced. | Options: A) Yes, that is the guarantee B) No, per-target download is acceptable
18. [Section: Skills half] How should symlinks inside the archive be handled on copy? | Options: A) Copy as symlinks B) Dereference/copy targets C) Reject archives containing symlinks
19. [Section: Skills half] Is the report line format ('agent: installed ... (vX) -> path') normative (tests assert it) or illustrative? | Options: A) Illustrative B) Normative/asserted
20. [Section: Install per target] On mid-loop partial failure, should the raised error enumerate which agents succeeded and which remain, or name only the failed target? | Options: A) Name failed target only (current) B) List succeeded + failed + remaining
21. [Section: Database half] Does the database half target strictly the same settings-derived DB path as 'db init', with no setup-specific override? | Options: A) Same as db init, no override B) Allow a --db-path/env override
22. [Section: Independence and exit status] When both halves run, is the skills-then-DB order significant, or are the halves order-independent? | Options: A) Order-independent (either order fine) B) Skills must precede DB C) DB should precede skills
23. [Section: Independence and exit status] On partial success (one half ok, one failed, exit 1), should the successful half's output clearly state it succeeded despite the non-zero exit? | Options: A) Yes, explicit per-half success line B) Only the failure summary matters
24. [Section: Independence and exit status] If the skills half fails, should the DB half still run by default (current), or short-circuit? | Options: A) Always attempt both (current) B) Stop after first half failure
25. [Section: Error handling] GitHub 403 rate-limit / secondary-rate-limit is not in the table. Surface it as a distinct message or fold it into the generic network error? | Options: A) Distinct rate-limit message (mention GITHUB_TOKEN) B) Fold into network-error path
26. [Section: Error handling] HTTP errors other than 404 (500/502 from the API) — distinct message or network-error path? | Options: A) Network-error path B) Distinct server-error message
27. [Section: Error handling] A corrupt/truncated download (zip CRC/BadZipFile) — map to 'release asset is malformed' or a distinct download-integrity error? | Options: A) 'release asset is malformed' B) Distinct download/integrity error
28. [Section: Error handling] Should the download verify integrity (Content-Length match, or a checksum asset if the release publishes one)? | Options: A) No verification B) Verify Content-Length C) Verify checksum when available
29. [Section: Documentation surface] The contributor path keeps 'gh skill install' / 'mise //:skill-install'. Should that local-source install also be reachable via 'cafleet setup' (e.g. a --local/source mode), or stay entirely separate? | Options: A) Stay separate B) Add a local-source mode to setup
30. [Section: Documentation surface] Should 'docs/spec/cli-options.md' document setup's exit codes explicitly (0 success, 1 half failure, 2 usage error)? | Options: A) Yes, document exit codes B) Flags only
31. [Section: Step 2: Refactor the database init code path for reuse] Should 'run_db_init()' return a status (already-at-head vs migrated) so setup can report it, or stay '-> None'? | Options: A) Keep '-> None' (current) B) Return a status the caller can echo
32. [Section: Step 4: Tests] Should tests assert the specific 'predates the skills-archive feature' wording for a missing-asset (pre-0000108) release, or just assert the failure? | Options: A) Assert the specific wording B) Assert failure only
33. [Section: Step 4: Tests] Is the 'browser_download_url' redirect (GitHub -> S3) out of scope for tests (fully monkeypatched), or should redirect-following be exercised? | Options: A) Out of scope; monkeypatch the URL B) Add a redirect-following test
34. [Section: Skills half] Should setup emit progress for a multi-MB asset download, or download silently? | Options: A) Silent download B) Print a brief progress/'downloading...' line
35. [Section: Command surface] Should '--agent'/'--release-tag' combined with '--skills-only' be explicitly valid (they apply to the skills half), confirming the validation only rejects combos with '--db-only'? | Options: A) Yes, valid with --skills-only B) Reconsider validation matrix

## Answers

### Round 1 (Questions 1-4)

1. A) Pure consolidation of existing onboarding; no behavior change to the underlying steps.
2. A) End-user (installed-CLI) command only; contributors use mise //:skill-install.
3. A) Per-resolved-release idempotency — reword the Success Criteria wording.
4. B) Pin the skills version to the running CLI version so skills match the installed CLI (CHANGE from latest-stable default).

### Round 2 (Questions 5-8)

5. A) Dedupe repeated --agent values silently.
6. A) --agent scopes the skills targets only; the DB half still runs by default.
7. A) No --dry-run flag.
8. Free-text: no need for --release-tag at all — derive the version to install from the installed CLI version. (Combined with Q4: DROP --release-tag and the latest-stable lookup; always fetch cafleet-skills-v<cli_version>.zip from the release tag matching the installed CLI version.)

### Round 3 (Questions 9-12)

9. A) Fixed 30s HTTP timeout; no override.
10. A) Hardcoded final target paths; no env override.
11. A) Create the skills dir when only the home exists (current).
12. A) Create the home/skills tree anyway for an explicit --agent (current).

### Round 4 (Questions 13-16)

13. A) Strictly anonymous — user notes no token is needed to download the public .zip asset.
14. A) Yes — reject path-traversal / zip-slip members before extraction (CHANGE: add the check).
15. B) Treat directories beyond the three SKILL_DIRS as malformed (CHANGE: stricter validation, reject extras).
16. A) Replace silently (current).

### Round 5 (Questions 17-20)

17. A) Yes — make the download+extract+validate-before-any-rmtree ordering an explicit stated guarantee.
18. Free-text: no special symlink handling — that check is unnecessary; rely on shutil defaults (do NOT add a symlink-specific check). (zip-slip traversal check from Q14 still applies.)
19. A) Report line format is illustrative, not asserted (the doc already uses "e.g.").
20. A) Name the failed target only (current).

### Round 6 (Questions 21-24)

21. A) Same settings-derived DB path as db init; no override.
22. A) Halves are order-independent.
23. A) Explicit per-half success line even on non-zero overall exit (current).
24. A) Always attempt both halves (current).

### Round 7 (Questions 25-28)

25. A) Fold 403 rate-limit into the network-error path (no token, so no GITHUB_TOKEN mention).
26. A) Fold non-404 HTTP errors (5xx) into the network-error path.
27. A) Map BadZipFile / truncated download to the existing 'release asset is malformed' message.
28. A) No download-integrity verification (BadZipFile catch suffices; no checksum asset today).

### Round 8 (Questions 29-32)

29. A) Keep the contributor local-source install entirely separate (no --local mode in setup).
30. Free-text: do NOT add an explicit exit-codes table; just show the appropriate error message at runtime.
31. A) Keep run_db_init() -> None (current).
32. Free-text: do NOT handle the "predates the skills-archive feature" case — 0000108 and this feature publish at the same time, so every published CLI version has the asset. (CHANGE: REMOVE the "Asset availability precondition" section, the tailored predates error wording, and its test; simplify missing-asset to a generic message.)

### Round 9 (Questions 33-35)

33. A) Redirect-following out of scope for tests; monkeypatch the URL/response.
34. A) Silent download; no progress line.
35. Free-text: --skills-only is not needed — keep the CLI as simple as possible. (CHANGE: DROP --skills-only; and since --db-only duplicates the existing `cafleet db init`, drop it too. See the follow-up decision below for the final flag set.)

### Follow-up (flag surface)

Resolved: keep ONLY --agent. Final surface = `cafleet setup [--agent claude|codex|opencode ...]` (repeatable, deduped). Always runs both halves (skills + db). For db-only, use the existing `cafleet db init`. DROP --release-tag, --skills-only, --db-only. The skills version is derived from the installed CLI version (importlib.metadata.version("cafleet")); fetch GET /repos/himkt/cafleet/releases/tags/<cli_version> and asset cafleet-skills-v<cli_version>.zip. No latest-stable lookup.
