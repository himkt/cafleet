# Member capture keeps CRLF-terminated rows

**Status**: Complete
**Progress**: 7/7 tasks complete
**Last Updated**: 2026-10-31

## Overview

On herdr, plain-mode `cafleet member capture` blanks every row except an unterminated last row, because `strip_ansi` reads the CR half of herdr's CRLF row terminator as a carriage-return overwrite (issue #525). The fix trims each line's trailing carriage returns before the CR-redraw defrag, so captures return the last N rows intact on every multiplexer while spinner overwrites (`one\rONE` → `ONE`) still resolve.

## Success Criteria

- [x] Plain-mode `member capture` on herdr returns every windowed row's text; a CRLF terminator is a line ending, not an overwrite.
- [x] CR-redraw defrag is unchanged for interior carriage returns: `one\rONE` → `ONE`, `a\rb\rc` → `c`, `\x1b[2Kspinner\rdone` → `done`.
- [x] `--ansi` output stays the byte-for-byte windowed raw capture (still CRLF on herdr), with `content_sha256` hashing those bytes.
- [x] `SPEC.md` § "`strip_ansi` regex and CR-defrag" states the trailing-CR rule.
- [x] `mise //cafleet:test` and `mise //cafleet:lint` pass with the new test cases in `render.rs`, `herdr.rs` and `capture.rs`.

---

## Background

The plain capture path is: backend `capture_pane` → shared `capture_window` → `CaptureSnapshot::from_raw` → `strip_ansi`.

| Stage | Location | Current behavior |
|---|---|---|
| herdr read | `HerdrMultiplexer::capture_pane` (`cafleet/src/multiplexer/herdr.rs`) | Runs `herdr pane read <pane> --format ansi --source recent-unwrapped --lines <N+1000>`; every row ends with `\r\n`. |
| Windowing | `capture_window` (`cafleet/src/multiplexer/mod.rs`) | Splits on `\n` only, keeping `\r` for the later defrag; drops the visually blank tail (`\r` and ` \r` are blank because `str::trim` removes `\r`); keeps the last N lines with their original bytes. |
| Plain cleanup | `strip_ansi` (`cafleet/src/output/render.rs`) | Strips CSI, splits on `\n`, keeps the segment after the last `\r` per line. |

For herdr buffer `row1\r\nrow2\r\n\r\n`, windowing returns `row1\r\nrow2\r`. `strip_ansi` then splits it into `row1\r` and `row2\r`, and the segment after the last `\r` is empty for both, so the capture is blank. A plain `\r\n` → `\n` replacement would still blank `row2`: the window's last kept line ends in a bare `\r` because the blank tail was cut at the `\n` split.

Under the default `--event-evidence metadata`, the Director identifies a permission request's operation from current capture. With every herdr row blanked, it cannot identify the request at all.

---

## Specification

### `strip_ansi` rule

After CSI stripping and the `\n` split, each line is resolved in two steps:

1. Trim the trailing run of `\r`. A carriage return with nothing written after it overwrites nothing; this covers a CRLF terminator, a window-cut bare `\r` and a repeated `\r\r\n`.
2. Keep the segment after the last remaining `\r` (the existing CR-redraw defrag).

Lines are re-joined with `\n`. Only `strip_ansi` changes; backends and `capture_window` keep their bytes, so `--ansi` output and the windowing contract are unchanged.

```rust
pub(crate) fn strip_ansi(text: &str) -> String {
    let cleaned = ANSI_ESCAPE_RE.replace_all(text, "");
    cleaned
        .split('\n')
        .map(|line| {
            line.trim_end_matches('\r')
                .rsplit('\r')
                .next()
                .expect("rsplit yields a segment")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

| Input | Before | After |
|---|---|---|
| `one\r\ntwo\r\nthree` | `\n\nthree` | `one\ntwo\nthree` |
| `row1\r\nrow2\r` | `\n` | `row1\nrow2` |
| `row\r` | empty | `row` |
| `row\r\r\n` | `\n` | `row\n` |
| `spin\rDONE\r\nnext` | `\nnext` | `DONE\nnext` |
| `kept\x1b[0m\r\n` | `\n` | `kept\n` |
| `one\rONE\ntwo\rTWO` | `ONE\nTWO` | `ONE\nTWO` |
| `a\rb\rc` | `c` | `c` |

CSI stripping runs before the trim, so a CSI sequence between the row text and its CRLF (`kept\x1b[0m\r\n`) never hides the terminator.

### `SPEC.md` contract

Replace the CR-defrag sentence in § "`strip_ansi` regex and CR-defrag" with:

> After stripping CSI matches, split on `\n` and, per line, trim the trailing run of `\r` — a carriage return with nothing written after it, such as the CR of a CRLF row terminator, overwrites nothing — then keep only the substring after the **last** remaining `\r` (CR-redraw defrag: a TUI redraw `prefix\rNEW` keeps only `NEW`, and a CRLF-terminated `row\r\n` keeps `row`), then re-join with `\n`.

The remaining text of that section, § "Capture windowing" ("preserving `\r` for the CLI's later CR-defrag"), § "Capture snapshot" and the `member capture` option text ("cleans carriage-return redraws") remain accurate. `skills/cafleet/reference/cli.md`, `docs/` and `README.md` use the same "carriage-return redraws" wording and need no edit. No version bump or CHANGELOG entry is part of this change.

### Tests

Each test lives in the file that owns the asserted behavior (`.claude/rules/code-quality.md` § Test placement).

| File | Test | Cases |
|---|---|---|
| `cafleet/src/output/render.rs` | Extend `capture_cleanup_removes_csi_and_keeps_the_final_carriage_return_segment` | The first six "After" rows of the `strip_ansi` table above, as `(raw, expected)` pairs; existing pairs stay. |
| `cafleet/src/multiplexer/herdr.rs` | New `capture_pane_keeps_crlf_rows_and_drops_the_crlf_blank_tail` | Fake read returns `scrollback\r\nprompt> ls\r\nstatus bar\r\n\r\n \r\n`; `capture_pane("w1:p2", 2)` returns `prompt> ls\r\nstatus bar\r` (raw `\r` kept, blank CRLF tail dropped); `assert_styled_read(&runner, "w1:p2", "1002")`. |
| `cafleet/src/capture.rs` | New `snapshot_treats_crlf_row_terminators_as_line_endings` | `CaptureSnapshot::from_raw("old\r\x1b[31m雪\x1b[0m\r\n緑\r", false, now())` equals content `雪\n緑`, `captured_at` `WHEN`, `content_sha256` `PLAIN_HASH`. |

The herdr test pins the windowing half of the herdr path and passes before the fix; the `render.rs` and `capture.rs` cases fail before the fix and pass after it.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `SPEC.md` § "`strip_ansi` regex and CR-defrag" with the trailing-CR rule from § Specification > `SPEC.md` contract <!-- completed: 2026-10-31T00:36 -->

### Step 2: Tests

- [x] Add the CRLF and trailing-CR pairs to `capture_cleanup_removes_csi_and_keeps_the_final_carriage_return_segment` in `cafleet/src/output/render.rs` <!-- completed: 2026-10-31T00:44 -->
- [x] Add `capture_pane_keeps_crlf_rows_and_drops_the_crlf_blank_tail` to `mod tests` in `cafleet/src/multiplexer/herdr.rs` <!-- completed: 2026-10-31T00:44 -->
- [x] Add `snapshot_treats_crlf_row_terminators_as_line_endings` to `mod tests` in `cafleet/src/capture.rs` <!-- completed: 2026-10-31T00:44 -->

### Step 3: Fix

- [x] Trim each line's trailing `\r` run before the per-line `rsplit('\r')` in `strip_ansi` (`cafleet/src/output/render.rs`) <!-- completed: 2026-10-31T00:50 -->

### Step 4: Validation

- [x] Run `mise //cafleet:test` and confirm every test passes (the change's tests pass; the runtime::runner and deletion failures reproduce on the base commit) <!-- completed: 2026-10-31T00:54 -->
- [x] Run `mise //cafleet:lint` and confirm it is clean <!-- completed: 2026-10-31T00:50 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-31 | Initial draft |
| 2026-10-31 | Approved after Reviewer approval and user authorization |
| 2026-10-31 | Implemented and reviewed; Status set to Complete |
