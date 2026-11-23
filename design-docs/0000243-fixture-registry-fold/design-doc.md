# Fold the fixture registry into per-backend profile files

**Status**: Approved
**Progress**: 15/30 tasks complete
**Last Updated**: 2026-11-23

## Overview

Replace `assets/profiles/bundled.json` (one profile per backend plus a 526-entry `fixtures[]` registry) and `assets/profiles/native/<backend>.json` (the captured screens) with one self-contained file per backend, `assets/profiles/<backend>.json`, that holds the backend's profile and its screens. Each screen carries its own `native_version` and optional `composer_evidence`, so the redundant registry, its `sha256` digests and the dual-write between two files disappear.

## Success Criteria

- [ ] `assets/profiles/bundled.json` and `assets/profiles/native/` no longer exist; `assets/profiles/<backend>.json` exists for every backend that has a bundled profile today, holding exactly `schema_version`, `backend`, `profile` and `screens`.
- [ ] Every one of today's 526 registry entries appears as exactly one screen, carrying the entry's `native_version` and (for the 80 entries that have it) its `composer_evidence`; no screen carries `id`, `kind`, `backend`, `source_ref` or `sha256`.
- [ ] The production binary embeds every profile file and builds the bundled catalog from them, decoding each profile while skipping the `screens` contents.
- [ ] The bundled-catalog guard validates every screen of every profile file (exact field set, geometry, masking, cell runs, composer evidence), fixture-id uniqueness across files, rejection of repeated object keys in a profile file's text, and that every `fixture_ids` value names a screen of the same profile file.
- [ ] No catalog JSON, production or test, contains a fixture registry; `FixtureKind`, `source_ref` and `sha256` handling are gone from the code.
- [ ] `SPEC.md`, `docs/docs/contributing.md` and `.claude/rules/code-quality.md` describe the per-backend layout; no living document mentions `bundled.json`, `native/<backend>.json`, `fixtures[]`, the fixture registry, fixture digests or screen-key ordering.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

| Fact | Today |
|---|---|
| `bundled.json` | `schema_version`, `fixtures` (526 entries, ~180 KB) and `profiles` (one per backend, ~160 KB); embedded in the production binary with `include_str!` |
| Production use of `fixtures` | None: `RawCatalog` requires the key but reads it as `IgnoredAny` (`src/coding_agent/catalog.rs:166-175`) |
| `native/<backend>.json` | `schema_version`, `backend`, `provenance` (`native`), `screens` (~3 MB total); `include_bytes!` only inside `#[cfg(test)] mod test_support` (`catalog.rs:1306`), although `SPEC.md:279/286` say the binary carries them |
| Registry fields | `kind` (always `native`), `backend` and `source_ref` (derivable from the file), `rows`/`cols` (duplicated in the screen), `sha256` (computed from the screen); only `native_version` and `composer_evidence` carry unique data |
| Maintenance cost | Each new capture is written twice (screen + registry entry), its digest recomputed, and screen keys kept in registry order (`SPEC.md:2799`) |
| Uncited screens | 79 screens are cited by no `fixture_ids`; tests use them (classification of every catalogued screen) |
| Test catalogs | Test code builds and reads catalogs as `serde_json::Value` in the `{schema_version, fixtures, profiles}` shape, with synthetic `kind: "synthetic"` registry entries; the inventory below lists every file |

Every file whose tests index the catalog shape (`["profiles"]`, `["fixtures"]`, `"/profiles/…"`, `"/fixtures/…"` pointers) or call the bundled-data helpers. `src/coding_agent/mod.rs` only calls `single_profiles` (`mod.rs:1420`) and needs no edit beyond the helper's new return shape.

| File | Catalog-shape use |
|---|---|
| `src/coding_agent/catalog.rs` | `test_support` helpers (`synthetic_definitions`, `single_profiles`, `fixture_at`, `bundled_file`, `bundled_fixture`, `bundled_definitions`, `implicit_pattern_classes`) and most `mod tests` cases |
| `src/coding_agent/detector.rs` | Catalog builders editing `["profiles"]` (`:814`, `:2446`, `:3195`–`:4508`), `bundled_definitions()["profiles"]` iterations (`:2618`, `:4537`), `suggestion_fixtures` reading `backend`/`native_version` from registry entries (`:2658`) |
| `src/runtime/input.rs` | Synthetic entries pushed into `["fixtures"]` and profile edits (`:5917`–`:12169`); `backend` and `native_version` read from a bundled registry entry (`:5965`–`:5984`) |
| `src/runtime/service.rs` | Synthetic entries and profile edits (`:2589`–`:2614`, `:5227`); `native_screen` reads `backend`, `native_version`, `rows`, `cols` from a registry entry (`:3804`–`:3815`) |
| `src/runtime/observation.rs` | `definition["profiles"][0]` edits (`:8291`, `:8667`–`:8674`); `bundled_fixture` replays (`:10307`–`:10480`) |
| `src/cli/member.rs` | `catalog["profiles"][0]` edit (`:2018`) |

The user chose co-location over a slimmer binary: the release binary now embeds the screens too. Simulating § Conversion gives 3,219,397 bytes of profile files against 338,759 bytes for `bundled.json`, so the embedded data grows by 2,880,638 bytes. The gain is a single source of truth per backend and one write per capture.

---

## Specification

### Decisions

| Topic | Decision | Rationale |
|---|---|---|
| File layout | `assets/profiles/<backend>.json`, one per backend with a bundled profile; `bundled.json` and `native/` are deleted | One file per backend makes "at most one profile per backend" structural and keeps a profile beside the screens it cites |
| `sha256` | Dropped | The digest bound a registry entry to a screen in another file; with one file there is nothing to bind. Privacy review stays enforced by `masked_rows` (`reason: reviewed_privacy`, masked lines empty) |
| `native_version` | Explicit required screen field, canonical SemVer | Fixture ids are not a contract ("existing fixture IDs retain their patterns"), so the version is not derived from the id |
| `composer_evidence` | Optional screen field, same record schema and rules as today | Unique measured data; its semantics are unchanged |
| `kind` / file `provenance` | Removed | Every screen in a profile file is a native capture; "continuations cite native fixtures" and "bundled fixtures require native provenance" become structural |
| Screen key order | No ordering rule; the conversion keeps today's order | Zero churn; nothing reads order |
| Uncited screens | Allowed | 79 exist and serve tests |
| `schema_version` | `1` | The file is new, embedded and read only by this binary; nothing reads an older shape |
| Synthetic test fixtures | Test catalogs use the same per-backend file shape with stub screens; no test-only registry | One catalog shape everywhere; the synthetic/native distinction becomes "which entry point validates" (below) |
| Conversion | One-off, uncommitted script under BASE `.scripts/` | The fold happens once; the capture-to-fixture conversion writes the new shape afterwards |
| Catalog load cost | Accepted: `BUNDLED` initialization tokenizes about 3.2 MB of JSON once per process that loads the catalog, instead of 0.34 MB | `IgnoredAny` still scans every screen byte, but decoding straight from text builds no value tree for them, and the cost is paid once per process in the `LazyLock` |
| Duplicate JSON keys | The bundled guard rejects a repeated object key at any level of a profile file, screen ids included | `serde_json` keeps the last value of a repeated key, so a screen written twice in one file would otherwise collapse silently; today the registry's `unique` check catches it |

### Profile file format

```json
{
  "schema_version": 1,
  "backend": "claude",
  "profile": {
    "rules": [ ... ],
    "capabilities": { ... }
  },
  "screens": {
    "claude-2.1.302-default-permission-1": {
      "native_version": "2.1.302",
      "rows": 36,
      "cols": 120,
      "alternate_screen": false,
      "screen_seq": 18158,
      "cursor_row": 22,
      "cursor_col": 120,
      "lines": [ ... ],
      "masked_rows": [ ... ]
    },
    "codex-0.167.0-composer-exec-...-herdr-119-25": {
      "native_version": "0.167.0",
      "composer_evidence": [
        {"operation": "exec", "multiplexer": "herdr", "kind": "framing", "literal_framing": "bracketed_paste"},
        {"operation": "exec", "multiplexer": "herdr", "kind": "post_submit"}
      ],
      "rows": 40,
      ...
    }
  }
}
```

| Top-level field | Requirement |
|---|---|
| `schema_version` | `1` |
| `backend` | The backend named by the file name (`claude.json` → `claude`) |
| `profile` | Exactly `rules` and `capabilities`, with today's profile semantics; the profile's backend is the file's `backend` |
| `screens` | Object mapping each fixture id to its screen object; every screen is a native capture |

Screen object fields are today's fields plus two:

| Field | Requirement |
|---|---|
| `native_version` | Required; the capture's native version, parsed with the native version probe's SemVer rule (§ Native version probe); bounds nothing |
| `composer_evidence` | Optional; a nonempty array of today's records (`operation`, `multiplexer`, `kind` plus kind-specific fields); no duplicate `(operation,multiplexer,kind)` on one screen |
| `rows`, `cols`, `alternate_screen`, `screen_seq`, `cursor_row`, `cursor_col`, `lines`, `masked_rows`, `cell_backgrounds`, `cell_styles` | Unchanged, except `rows`/`cols` no longer compare against a registry entry |

Key order written by the conversion: top level `schema_version, backend, profile, screens`; inside a screen `native_version`, then `composer_evidence` when present, then the existing fields in their current order. Files use 2-space pretty JSON with one array element per line, non-ASCII unescaped and a trailing newline (today's formatting).

### Production loading (`src/coding_agent/catalog.rs`)

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfileFile {
    schema_version: u64,
    backend: CodingAgent,
    profile: RawProfile,          // RawProfile loses its `backend` field
    #[serde(rename = "screens")]
    _screens: IgnoredAny,
}

fn bundled_profile_text(agent: CodingAgent) -> Option<&'static str> {
    match agent {
        CodingAgent::Claude => Some(include_str!("../../assets/profiles/claude.json")),
        // one arm per backend; exhaustive, so each backend has at most one file
    }
}
```

- `RawCatalog` and `BUNDLED_DEFINITIONS` are deleted. `BUNDLED` iterates `CodingAgent::all()`, deserializes each `bundled_profile_text` directly from text with `serde_json::from_str::<RawProfileFile>` (no intermediate `Value` tree for the 3 MB of screens), and compiles it with `decode_file(backend, raw)`.
- `decode_file` requires `schema_version == 1` (`unsupported profile schema`) and `raw.backend == backend` (`profile file backend differs from its file`), then compiles the profile with the file's backend. `compile_profile` takes the backend as a parameter.
- The "more than one profile for one backend" check and its error are deleted: the exhaustive match and the object-keyed test catalogs make duplicates unrepresentable.
- `fixture_ids` values stay `IgnoredAny` in production decoding; the comment above the raw types is rewritten to say that `screens` and every `fixture_ids` value are provenance the guard checks.
- Load cost: `BUNDLED` initialization tokenizes about 3.2 MB of JSON per process instead of 0.34 MB. This one-time `LazyLock` cost is accepted (§ Decisions).

### Provenance validation and the bundled guard (`catalog.rs` `test_support`)

The fixture index is built from the profile files: each screen key is a fixture id whose backend is its file's backend.

| Entry point | Used by | Behavior |
|---|---|---|
| `ProfileCatalog::from_files(&Value)` | Tests, guard | `#[cfg(test)] impl` beside `ProfileCatalog`. Requires a JSON object mapping backend name (`unknown backend` otherwise) to a profile file; each value goes through `RawProfileFile::deserialize` and `decode_file` |
| `validate_provenance(&Value)` | `load`, guard | Reads each profile and, per screen, only `native_version` and `composer_evidence` (other screen keys are ignored here). Rules: fixture ids unique across files; every `fixture_ids` value nonempty; every cited id is a screen key of the same file; executable composer plans keep today's empty-composer and evidence-coverage rules; evidence records keep today's nonempty/no-duplicate/plan-match rules |
| `with_synthetic_composer_evidence(&Value)` | `load` only | For every composer plan, derive evidence records from the plan for each cited screen that lacks `composer_evidence` (today's derivation, keyed by screen instead of `kind: synthetic`) |
| `load(&Value)` | Test catalogs | `from_files` + `validate_provenance(with_synthetic_composer_evidence(..))` |
| `ProfileCatalog::from_bundled_files(&Value)` | Bundled guard | `from_files` + `validate_provenance` without decoration + `verify_bundled_screen` on every screen; sets `capture_origin: "bundled_unknown"` |
| `parse_bundled_text(&str) -> Result<Value>` | `bundled_files`, guard tests | Parses a profile file's text into a `Value` through a recursive deserializer whose map visitor fails on a repeated key at any object level (`duplicate object key <KEY>`), so a screen id written twice in one file is rejected instead of collapsing |
| `bundled_files() -> Value` | Tests | Object mapping every backend with a `bundled_profile_text` to `parse_bundled_text` of it (`expect` on failure, since the guard test reports the error); replaces `bundled_definitions()` |
| `implicit_pattern_classes(&Value)` | Pattern-syntax guard and its tests (`catalog.rs:4143`–`:4260`) | Takes the object-keyed files; walks each file's `profile`, probes with `RawProfile::deserialize` on that object and names the backend from the file's `backend`; pointers are relative to `profile`, so `pattern_location` keeps reading `rules/<index>` |

Every iteration over bundled profiles, including `detector.rs:2618` and `detector.rs:4537`, walks `bundled_files()` entries and reads `file["profile"]` with the backend from `file["backend"]`.

`verify_bundled_screen` keeps today's `verify_native_screen` checks minus the digest and the registry geometry comparison, and its exact field set becomes `native_version` + `SCREEN_FIELDS` + optional `composer_evidence`, `cell_backgrounds`, `cell_styles`. `verify_bundled_fixtures`, `verify_native_fixture_file`, `BUNDLED_FIXTURES`, the `include_bytes!` constants, `FixtureKind`, `ProvenanceFixture._source_ref`/`sha256`/`kind`, the lowercase-digest check and the "continuation cites native fixtures" check are deleted. Bundling a stub screen fails the exact field check.

Guard and loader errors (all prefixed `invalid native profile: `):

| Condition | Error |
|---|---|
| A profile file's `schema_version` differs from `1` | `unsupported profile schema` |
| A profile file's `backend` differs from its file name or test-catalog key | `profile file backend differs from its file` |
| A fixture id is a screen key in two profile files | `duplicate fixture id <ID>` |
| An object in a profile file's text repeats a key (a screen id written twice included) | `duplicate object key <KEY>` |
| A `fixture_ids` value names no screen of its own profile file | `<BACKEND> cites unknown fixture <ID>` |
| A `fixture_ids` value is empty | `capability provenance required` (unchanged) |
| A bundled screen's fields differ from the screen field set | `exact required object fields expected` (unchanged) |
| A pattern-bearing field uses a construct outside the pattern syntax | Unchanged row |

These errors disappear: `bundled fixture bytes are not embedded`, `bundled fixture file identity differs from its catalog entry`, `bundled fixture screen is missing`, `bundled fixture file holds an uncited screen`, `bundled fixture digest mismatch for …`, `bundled fixture identity differs from its catalog entry`, `bundled fixtures require native provenance`, `lowercase source digest required`, `fixture backend differs from the profile backend`, `continuation cites native fixtures`, `more than one profile for one backend`, `unknown fixture`.

### Test catalogs

- A test catalog is a JSON object `{"<backend>": <profile file>}`; pointers change from `/profiles/0/...` to `/<backend>/profile/...` and from `/fixtures/N/...` to `/<backend>/screens/<id>/...`.
- A synthetic fixture is a stub screen `{"native_version": "0.0.1-synthetic"}` (plus `composer_evidence` where a test needs explicit records). `synthetic_definitions()`, `single_profiles()`, `fixture_at()` and the composer/inspection/gated/question helpers emit stubs into the owning backend's `screens`.
- A test that today sets `kind: "native"` to stop the decoration either calls `validate_provenance` directly (the composer-evidence tests already do) or is deleted when it asserts a removed rule (the "preserves native evidence gaps" half, the continuation-native rule, the duplicate-profile rule).
- Tests reading bundled data use `bundled_files()`; `bundled_fixture(id)` and `bundled_file(id)` locate the screen by searching every file's `screens`, and `bundled_fixture` keeps returning the screen's canonical JSON (fixture replay reads fields by name, so the added `native_version` and `composer_evidence` keys leave it unaffected); `detector.rs` `suggestion_fixtures` iterates screens whose id contains `-suggestion-` and reads `native_version` from the screen; `service.rs` `native_screen` reads `native_version`, `rows` and `cols` from the screen.
- `bundled_file(id) -> (CodingAgent, Value)` returns the backend and the parsed profile file whose `screens` holds `id` (panicking with `the bundled catalog contains <ID>` otherwise). `service.rs` `native_screen` (`:3804`) and the `input.rs` bundled-entry reader (`:5965`–`:5984`) take the backend from the first element and `native_version` (plus `rows`/`cols` where read) from `file["screens"][id]`.
- Write new guard cases as one table-driven test over named cases: cross-backend citation, duplicate id across files, a screen id repeated within one file's text, mismatched file backend, `schema_version` 2, bundled screen without `native_version`, bundled screen with an extra key, bundled screen with empty `composer_evidence`, and the embedded files passing `from_bundled_files`.

### Conversion (one-off)

Script `BASE/.scripts/fold_registry.py`, run with `python3 -I` from the repository root, not committed:

1. Load `bundled.json` and every `native/<backend>.json`; index registry entries by id.
2. For each backend `b` with a profile: build `{"schema_version":1,"backend":b,"profile":{"rules":…,"capabilities":…},"screens":{}}`.
3. For each screen of `native/b.json` in file order: assert its registry entry exists, has `backend == b`, `source_ref == "native/b.json"` and matching `rows`/`cols`; write `native_version`, `composer_evidence` (when present) and then the screen fields.
4. Assert every registry entry was consumed exactly once and every backend with a native file has a profile.
5. Write `assets/profiles/<b>.json` with `json.dumps(…, indent=2, ensure_ascii=False) + "\n"`.
6. Delete `assets/profiles/bundled.json` and the `assets/profiles/native/` directory from the filesystem; staging the deletions belongs to the Director's commit.

### Documentation changes

| Surface | Change |
|---|---|
| `SPEC.md` §6.3 Asset installation | Intro: the binary carries every listed data file; setup installs none of the profile files. `assets/profiles/` row: "One profile file `<backend>.json` per backend with a bundled profile, holding that backend's profile and the native screen fixtures it cites" |
| `SPEC.md` §6.9 Bundled profiles and native screen fixtures, opening paragraph | The binary embeds the bundled catalog: one profile file per backend (`assets/profiles/<backend>.json`) holding the profile and the native screen fixtures that are its provenance records; one file per backend means at most one profile per backend; a profile object has exactly `rules` and `capabilities`; each screen records its own `native_version`; a backend with no profile file launches with `coverage=unknown` |
| `SPEC.md` §6.9 registry paragraph (`:2362`) | Each screen key is a fixture id cited by `fixture_ids`; validation covers every screen, cited or not; run-time loading ignores `screens` contents and `fixture_ids`; rules are ids unique across files, no repeated object key in a profile file, nonempty citations and citations naming a screen of the same file. The §6.4 canonical-JSON reference, which existed only for `sha256`, is deleted |
| `SPEC.md` §6.9 `composer_evidence` paragraph | An optional screen field; duplicates rejected per screen |
| `SPEC.md` §6.9 fixture file table, ordering sentence and validity paragraph (`:2790-2818`) | Becomes the profile file table above plus the two new screen-field rows; the `screens` row reads "An object mapping each fixture id to its captured screen object" instead of "each catalog fixture `id`"; delete the screen-key ordering sentence; keep the formatting sentence for profile files; validity paragraph states `schema_version` `1`, `backend` equal to the file name and ids unique across files |
| `SPEC.md` §6.9 screen-field row `rows`, `cols` (`:2805`) | "Positive measured geometry" (drop "equal to the catalog fixture entry") |
| `SPEC.md` §6.9 format paragraph (`:2820`) | "Each file and screen parses with exactly its fields above" keeps its rules; the closing sentence "JSON formatting leaves the canonical screen digest unchanged." is deleted |
| `SPEC.md` §6.9 continuation sentence (`:2893`) | "Provenance validity additionally requires every `permission_continuation` rule to cite a fixture." |
| `SPEC.md` §6.7 Measurement (`:2932`) | "privacy-reviewed fixture digests" → "privacy-reviewed fixtures" |
| `docs/docs/contributing.md` project-structure row | `assets/profiles/` holds one runner-profile file per backend with its native screen fixtures; the binary embeds every file |
| `docs/docs/contributing.md` § Bundled catalog provenance checks | Guard checks the embedded profile files; production loading skips `screens` and `fixture_ids`; error table replaced by the table above; synthetic sentence: test catalogs use stub screens, the test loader derives missing composer evidence, and the guard requires complete captured screens |
| `docs/docs/contributing.md` § Native screen fixture capture | The conversion inserts the screen under its fixture id in `screens` of `assets/profiles/<backend>.json`, recording `native_version` and any `composer_evidence` in the screen |
| `.claude/rules/code-quality.md` § Test placement | Parenthetical becomes "(the files production code embeds, including the native screen fixtures inside each profile file)" |
| `README.md`, `assets/skills/` | No change: neither mentions the registry or file paths (verified by grep) |

`SPEC.md` text stays implementation-neutral: it names the files, fields and rules, never `include_str!`, serde or the test suite.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] Rewrite §6.3 Asset installation intro and the `assets/profiles/` row <!-- completed: 2026-11-23T07:51 -->
- [x] Rewrite the §6.9 Bundled profiles and native screen fixtures opening and registry paragraphs and the `composer_evidence` paragraph <!-- completed: 2026-11-23T07:51 -->
- [x] Replace the fixture file table, ordering sentence and validity paragraph with the profile file format and the two screen-field rows <!-- completed: 2026-11-23T07:51 -->
- [x] Reword the continuation sentence and the §6.7 Measurement digest phrase <!-- completed: 2026-11-23T07:51 -->
- [x] Grep `SPEC.md` with `grep -nF` for each literal `bundled.json`, `native/<backend>`, `profiles/native`, `fixtures[]`, `fixture registry`, `fixture entry`, `source_ref`, `canonical screen digest`, `fixture digest`, `` `provenance` `native` ``; every hit is a defect to fix, since none of these phrases describes the new layout <!-- completed: 2026-11-23T07:51 -->

### Step 2: docs and rules

- [x] Update `docs/docs/contributing.md` project-structure row, provenance-check section (prose, error table, synthetic sentence) and capture section <!-- completed: 2026-11-23T07:52 -->
- [x] Update the `.claude/rules/code-quality.md` Test placement parenthetical <!-- completed: 2026-11-23T07:52 -->
- [x] Run the Step 1 literal grep over `README.md`, `docs/docs/`, `assets/skills/` and `.claude/rules/`; every hit is a defect to fix <!-- completed: 2026-11-23T07:52 -->

### Step 3: Data conversion

- [x] Before any data or code edit, run `mise //:build` and record the byte size of `target/release/cafleet` in `BASE/.notes/binary-size.md` as the baseline <!-- completed: 2026-11-23T07:55 -->
- [x] Write and run `BASE/.scripts/fold_registry.py` per § Conversion; all assertions pass <!-- completed: 2026-11-23T07:55 -->
- [x] Spot-check one screen per backend and one `composer_evidence` screen against the old files with `git diff --stat` and `git show HEAD:<path>` <!-- completed: 2026-11-23T07:55 -->
- [x] Delete `assets/profiles/bundled.json` and `assets/profiles/native/` from the filesystem (conversion step 6), leaving staging to the Director <!-- completed: 2026-11-23T07:55 -->

### Step 4: Production loader

- [x] Add `RawProfileFile`, drop `backend` from `RawProfile`, pass the backend into `compile_profile` <!-- completed: 2026-11-23T07:58 -->
- [x] Add `bundled_profile_text` (exhaustive match over every backend) and `decode_file`; rebuild `BUNDLED` from them <!-- completed: 2026-11-23T07:58 -->
- [x] Delete `RawCatalog`, `BUNDLED_DEFINITIONS`, `from_definitions` and the duplicate-profile check; rewrite the provenance comment on the raw types <!-- completed: 2026-11-23T07:58 -->

### Step 5: Guard and test support

- [ ] Add `#[cfg(test)] impl ProfileCatalog { from_files, from_bundled_files }`, `parse_bundled_text` (duplicate-key rejection) and `bundled_files()` <!-- completed: -->
- [ ] Rework `implicit_pattern_classes` to walk each file's `profile` with the backend from the file <!-- completed: -->
- [ ] Rework `validate_provenance` and `with_synthetic_composer_evidence` to read screens from profile files per § Provenance validation <!-- completed: -->
- [ ] Replace `verify_native_screen` with `verify_bundled_screen` (new field set, no digest or registry geometry) and add cross-file id uniqueness and same-file citation checks with the new error strings <!-- completed: -->
- [ ] Delete `verify_bundled_fixtures`, `verify_native_fixture_file`, `BUNDLED_FIXTURES`, the `include_bytes!` constants, `FixtureKind` and the registry fields of `ProvenanceFixture` <!-- completed: -->
- [ ] Rework `synthetic_definitions`, `single_profiles`, `fixture_at`, `bundled_file`, `bundled_fixture` and the shared definition helpers to the object-keyed file shape with stub screens <!-- completed: -->

### Step 6: Test migration

- [ ] Migrate `src/coding_agent/catalog.rs` tests to the new shape; delete tests of removed rules (duplicate profile, native provenance, digest, uncited screen, embedded bytes, continuation-native) <!-- completed: -->
- [ ] Migrate `src/runtime/input.rs` and `src/runtime/service.rs` tests (including `native_screen` and the bundled-entry reader) per the Background inventory <!-- completed: -->
- [ ] Migrate `src/coding_agent/detector.rs` (`suggestion_fixtures`, catalog builders, bundled-profile iterations), `src/runtime/observation.rs` and `src/cli/member.rs` per the Background inventory <!-- completed: -->
- [ ] Add the table-driven guard test of § Test catalogs <!-- completed: -->
- [ ] Sweep `src/` with `grep -rnF` for each literal `["fixtures"]`, `"/fixtures/`, `"fixtures":`, `["profiles"]`, `"/profiles/`, `"profiles":`, `source_ref`, `"kind":"synthetic"`, `"kind":"native"`, `profiles/native`, `bundled.json`, `BUNDLED_FIXTURES`, `bundled_definitions`, `FixtureKind`. These lookups index `serde_json::Value` and compile under the old shape, so the grep, not the compiler, finds them; every hit is a caller still on the old shape and must be migrated until the sweep returns nothing <!-- completed: -->

### Step 7: Verification

- [ ] `mise //:test` passes <!-- completed: -->
- [ ] `mise //:lint` passes <!-- completed: -->
- [ ] `mise //docs:build` passes <!-- completed: -->
- [ ] Run `mise //:build`, compare the size of `target/release/cafleet` with the Step 3 baseline and report the delta (expected ≈ +2.88 MB, per Background) to the Director <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-23 | Initial draft |
| 2026-11-23 | Review round: file inventory, duplicate-key rejection, accepted load cost, helper signatures, SPEC passages, literal grep sweeps, baseline binary size; approved |
