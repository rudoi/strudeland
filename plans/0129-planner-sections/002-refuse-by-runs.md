# 002: the refusal names sections from the plan

## Goal

`refuse` names each unsupported signature's section by looking its first bar up in
`planned.sections`. `section_at`, `section_of_song_bar` and `section_of_clip_bar` are deleted
with their tests, so `strudeland-render` no longer re-derives the clip or linear mapping. Every
refusal text stays byte-identical. This stage is Rust only: no planner file, bundle, ref or TS
file changes.

## Prerequisites

- Stage 001 committed and green. Its plan A/B and refusal A/B logs end `equal`.
- `$B` (`$S/0129-baseline/`) present with its binaries and recordings. If it is missing,
  rebuild it from `ee91dbd` with `git archive` and its own `CARGO_TARGET_DIR`, as
  [000-prompt.md](000-prompt.md#the-full-gate) says, never by switching the working tree.

## Files

- Edited:
  - `crates/strudeland-render/src/plan.rs`;
  - `AGENTS.md`: the render pipeline bullet's line that reads `plan (host to jobs, refusal of
    unsupported material; the only V8 caller)` (line 285 at `ee91dbd`; find it by that text).
- Untouched, besides the shared list:
  - everything under `src/`, `scripts/` and `tests/`;
  - `crates/strudeland-host/` (`js/` included);
  - `crates/strudeland-engine/`;
  - `crates/strudeland-render/src/{error.rs,pipeline_audio.rs,testing.rs,bin/}`;
  - `crates/strudeland-render/tests/`, `crates/strudeland-render/README.md`.

## Steps

1. **The lookup.** In `plan.rs`, add:

   ```rust
   /// The section of the run in `sections` (sorted, disjoint) that contains layout bar `bar`;
   /// `None` for a bar in no run, a negative bar included.
   fn section_name(sections: &[PlannedSection], bar: i64) -> Option<&str> {
       let i = sections.partition_point(|s| i64::from(s.to_bar) <= bar);
       sections
           .get(i)
           .filter(|s| i64::from(s.from_bar) <= bar)
           .map(|s| s.name.as_str())
   }
   ```

2. **`refuse`** (`:62-90`) becomes `refuse(sections: &[PlannedSection], signatures: impl
   IntoIterator<Item = UnsupportedSignature>)`. The dedupe, the `Unsupported` fields and the
   sort are unchanged. Only `section:` changes, to
   `section_name(sections, layout_bar).map(str::to_string)`. The doc comment says "each named
   with the section the plan places its bar in (`PlannedRender::sections`)".
3. **`take_jobs`** (`:58`): `refuse(&planned.sections, signatures)?`.
4. **Delete** `section_at` (`:92-98`), `section_of_song_bar` (`:100-110`) and
   `section_of_clip_bar` (`:112-136`). Trim the `strudeland_host` import (`:8-10`) to what is
   still used (`ExportPreset, Host, PlannedRender, PlannedSection, RenderPlanRequest`): `Layout`,
   `PlannedWindow` and `Structure` go. In the test module, `use strudeland_host::{Meter,
   Section};` (`:141`) goes too: only the deleted `section`, `song_linear` and `no_sections`
   helpers use it. Let clippy confirm both.
5. **Tests in `plan.rs`:**
   - Delete `clip_bars_map_through_the_loop`, `clip_diagnostics_follow_each_section_inside_a_repeated_range`
     and `song_bars_map_linearly` (`:190-279`). Stage 001's TS `planRender sections` tests
     hold their cases.
   - Delete the helpers `section`, `song_linear`, `no_sections` and `window` (`:143-188`,
     `:289-295`), and add `fn run(from_bar: u32, to_bar: u32, name: &str) -> PlannedSection`.
   - `refused(sections: &[PlannedSection], sigs)` replaces `refused(window, structure, sigs)`.
   - The refusal tests keep every expected literal and assertion byte for byte. Only the
     first argument changes:
     - `nothing_unsupported_is_not_refused`: `refuse(&[], vec![])`;
     - `signatures_dedupe_by_sound_and_keys_with_the_minimum_bar`: `&[]`. Today's input has
       no sections, and the same `section.is_none()` assertion and text follow;
     - `the_clip_layout_names_the_clip_section`: `song-linear`'s clip runs,
       `[run(0, 2, "intro"), run(2, 10, "body"), run(10, 12, "tail")]` (what stage 001's TS
       test pins for that clip). The comment says so;
     - `the_linear_layout_names_the_song_section`: `song-linear`'s linear runs,
       `[run(0, 2, "intro"), run(2, 6, "body"), run(6, 8, "tail")]`. Its comment says what
       the runs stand for: the song's linear layout. A real `--section body` plan's runs stop
       at bar 6, and a bar-6 signature cannot come from it (reverify §3, first bar `< c1`).
       The test is about the lookup and the text. The lookback naming is pinned end to end by
       `tests/pipeline.rs:274-304` and by 001's TS test.
   - Add `section_name_looks_bars_up_in_the_runs`, over
     `[run(0, 2, "intro"), run(2, 10, "body"), run(12, 14, "tail")]`:
     - bar `-1` is `None`;
     - bars `0` and `1` are `intro`;
     - bars `2` and `9` are `body`;
     - bars `10` and `11` are `None` (the gap);
     - bars `12` and `13` are `tail`;
     - bars `14` and `i64::MAX` are `None`;
     - with `&[]`, every bar is `None`.
   - Targeted: `cargo test -p strudeland-render --lib plan::`, then
     `cargo test -p strudeland-render --test pipeline`.
6. **Docs.** `AGENTS.md`, the render pipeline bullet, found by the text `plan (host to jobs,
   refusal of unsupported material; the only V8 caller)`: `plan` "(host to jobs, refusal of
   unsupported material, its sections named from the plan's `sections`; the only V8 caller)".
   `crates/strudeland-render/README.md:15-17` and `SKILL.md` stay as they are: they describe
   the refusal list, which does not change.

## Tests

- Deleted: the three mapping tests and their helpers (step 5). Their cases live in
  `tests/render-job-plan.test.ts` (`planRender sections`) since 001.
- Inputs changed, expected literals byte-identical: the four refusal tests (step 5).
- New: `section_name_looks_bars_up_in_the_runs`.
- Unchanged and passing: every text-pinning test in [000-prompt.md](000-prompt.md), and
  `tests/pipeline.rs`'s two real-plan refusal tests above all. They are the end-to-end proof
  that the runs reach `refuse`.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=002`: steps 1 (with
`host-unchanged`), 3 (check mode, exit 0: no ref is stale), 6-11, 13-15. Steps 2, 4, 5, 12, 16
and 17 do not apply: no planner file changes. Also:

```sh
git diff --stat HEAD -- src scripts tests crates/strudeland-host crates/strudeland-engine \
  crates/strudeland-render/src/error.rs crates/strudeland-render/src/pipeline_audio.rs \
  crates/strudeland-render/src/testing.rs crates/strudeland-render/src/bin crates/strudeland-render/tests \
  crates/strudeland-render/README.md > "$S/0129-002-own-untouched.log"; echo "exit $?" >> "$S/0129-002-own-untouched.log"
grep -n "section_at\|section_of_song_bar\|section_of_clip_bar" -r crates src-tauri \
  > "$S/0129-002-gone.log"; echo "exit $?" >> "$S/0129-002-gone.log"
git diff HEAD -- crates/strudeland-render/src/plan.rs | grep '^[-+].*\\n  s=' \
  > "$S/0129-002-literals.log"; echo "exit $?" >> "$S/0129-002-literals.log"
```

Run these before committing (`HEAD` is stage 001's commit).

- `own-untouched.log` lists no files.
- `gone.log` ends `exit 1`: grep found nothing.
- `literals.log` ends `exit 1`, so no refusal literal line was removed or added in
  `plan.rs`. If `cargo fmt` rewrapped a call that holds a literal, read the diff by hand and
  confirm the literal itself is byte-identical. Say so in the report.
- The refusal A/B (gate step 11) is the visible-behaviour check. It must give the same
  `equal: <n> cases` as in 001, against the same `ee91dbd` baseline.

## STOP

- Any refusal A/B difference.
- A refusal test that needs a changed expected literal, sound, bar or `section` assertion.
- `tests/pipeline.rs`, `tests/bin.rs`, `error.rs`, the CLI tests or `e2e.rs` failing.
- Anything in the shared STOP list.

## Commit message

```
refactor(render): name refused sections from the plan's sections

The unsupported-material refusal looks each first bar up in the runs
the planner returns instead of re-deriving the clip and linear bar
mappings in Rust, so the transport is the one place that knows which
section a bar plays in. Every refusal text is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
