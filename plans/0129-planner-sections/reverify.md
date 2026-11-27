# 0129 step 0: re-verification on main ee91dbd

The duplication stands. There is no divergence today, and no new copy since 63b945d. Since 63b945d,
`plan.rs` only changed its imports and request fields (`capability::`, `instruments`, `chunk`).

## 1. The re-derivation and its callers

| What | Where |
|---|---|
| `refuse(window, structure, signatures)`: dedupes by `(s, keys)` keeping the minimum bar, names a section, sorts | `crates/strudeland-render/src/plan.rs:62-90` |
| `section_at(layout, structure, bar)`: picks clip or linear | `plan.rs:93-98` |
| `section_of_song_bar`: scans sections for a song bar | `plan.rs:101-110` |
| `section_of_clip_bar`: `loop_range`, else the `loop_index` section, × `loops`, then the tail | `plan.rs:115-136` |
| Only caller: `take_jobs` gathers `unsupported_signatures(&job)` over the opening and repeat chunks, then calls `refuse(&planned.window, &planned.structure, ..)` | `plan.rs:39-59` |
| `take_jobs` caller (the one pipeline path for the CLI, the app export, the stems export and the harness binary) | `pipeline_audio.rs:67` |
| `Unsupported { layout_bar, s, keys, section }` and its order | `error.rs:39-47` |
| Refusal text `… first at bar N (section 'x')` | `error.rs:81-98` (`display_unsupported`) |
| App export text: `Error::Render(e) => e.to_string()` | `src-tauri/src/export_audio.rs:146`, `export/error.rs:69` |

- The engine's `signatures` subcommand and `rust-engine.ts`/`formatSignature`
  (`engine-job.ts:402`) print `bar=` with no section. Only `refuse` maps `firstBar` to a section.
- `first_bar` is `floor(hap.cycle)` (`capability.rs:713`, `job.rs:241`). `cycle` is the absolute
  **layout** cycle (`engine-job.ts:45,354`, `Selected.onsetCycle = whole.begin`,
  `plan.ts:49,80`). One cycle is one written bar, mixed meter included: timing only maps cycles to
  seconds (`render/timing.ts:35-55`).

Tests that pin the refusal:

| Test | Pins |
|---|---|
| `plan.rs:190-224` `clip_bars_map_through_the_loop` | per-bar clip names, loops 2 and 3 |
| `plan.rs:226-238` `clip_diagnostics_follow_each_section_inside_a_repeated_range` | `loop_range` |
| `plan.rs:240-279` `song_bars_map_linearly` | linear names, the no-loop and no-section cases |
| `plan.rs:313-380` | dedupe and sort; full texts for clip and linear (`--section` lookback) |
| `error.rs:150-153,194-205` | full texts with and without a section |
| `tests/pipeline.rs:231-255` (samples.js), `:274-304` (`--section body` lookback names `intro`; clip names `intro`) | real host plans, full text |
| `tests/bin.rs:182`, `:451` | harness binary stderr |
| `tests/never_panics.rs:75` | the variant only |
| `strudeland-cli/src/render.rs:1365-1374`, `tests/e2e.rs:625` | `first at bar` |
| `SKILL.md:509-511`, `skill.rs:208`, `strudeland-render/README.md:17` | wording `s=... keys=... first at bar N` (no section wording) |
| TS | none. TS never names sections for signatures. |

## 2. What the plan-render result carries

- TS `PlannedRender` (`job-plan.ts:258-282`): `window {fromBar,toBar,layout}`, `export
  {loopStartBar, loopStartFrame, tailFrames, repeat{chunks,tailFrames,musicFrames}}`,
  `musicFrames`, `cps`, `structure` (the `loopsApplied` one, `:571,684`), `lookbackSec`, `errors`,
  `events?`, `chunks[{json, plan: ChunkWindowPlan (c0,c1,frames,…,fromBar,toBar), info?}]`. It
  carries no sections per bar and no segments.
- The Rust mirror is `crates/strudeland-host/src/planned.rs:13-33` (structure `:126-144`, no
  `deny_unknown_fields`, so a new field needs `#[serde(default)]` or an update to the `PLANNED`
  test string at `:185`). It is built by hand in `pipeline_audio.rs:290,530` and
  `testing.rs:160-170`.
- How the bar → section mapping works today, case by case. The planner's `segments` are
  `layoutSegments(layout, structure)` (`render/timing.ts:14-19`): `clipSegments` or
  `LINEAR_SEGMENTS`.

| Request | Layout, window (`resolveWindow`/`exportWindow`, `job-plan.ts:398-503`) | Mapping |
|---|---|---|
| clip, `--loops N` | clip `[0, clipBars)`, `structure.loops = N` | `section_of_clip_bar` |
| `--length N` with sections | clip `[0, N)` | clip |
| `--length N` without sections | linear | linear, and with no sections the name is `None` |
| `--section X` | linear `[start, end)`. Lookback bars before `start` are named by song section | linear |
| tagged-loop / intro-loop | linear `[0, loop.end)`, the repeat pass from `loopStartBar` (linear bars) | linear |
| folded-loop | linear `[loop.start, loop.end)` | linear |
| one-shot | as the length request | as the length request |
| stems | the same jobs and the same `refuse` | same |
| mixed meter | bars are logical, and a meter change does not move them | same |

## 3. Divergence from `transport.ts`

There is none in reachable input. I checked against `clipSegments` (`transport.ts:242-246`),
`songBarAt` (`:76-85`) and `locate` (`:88-101`):
- Clip, before `loop.start`: `p`. Within loop passes: `start + mod`. From the release at
  `loop.start + loops*period`: `loop.end + elapsed`. These are equal to `plan.rs:126-135`, both
  for `loop_range` (preferred by both: `authoredLoop`, `structure.ts:262`) and for a single loop
  section.
- Past the song: `locate` returns null when `bar >= totalBars`, and `section_of_song_bar` returns
  `None` (sections are contiguous from 0). These agree, also for harness clip windows past
  `clipBars`.
- Negative bars: `locate` clamps `g < 0` to bar 0, and `plan.rs` returns `None`. These differ, but
  the case cannot be reached: `c0 >= 0` (`plan.ts:140-142`) and `selectHaps` keeps only
  `hasOnset` haps, so `first_bar >= 0`. The engine unit test with `first_bar = -1`
  (`capability.rs:898-900`) is synthetic.
- `loops = 0` cannot occur (`loopsProblem`, `structure.ts:162`), and both sides would agree
  anyway.
- No test cross-checks the two. Only the hand-written `plan.rs` cases above pin the Rust copy.

## 4. Cost of a planner change (AGENTS.md)

- `job-plan.ts` is in `PLANNER_FILES` (`scripts/engine/provenance.ts:30-39`), so the work needs
  `engine:refs --write`. Only the provenance hash moves, and no ref audio does. Chromium for
  Playwright 1.64.1 is installed (`~/Library/Caches/ms-playwright/chromium-1248`, revision
  1248).
- `job-plan.ts` and `transport.ts` are already in the host bundle (`host.inputs.json:93,113`).
  After the change, run `host:build`, then `host:parity --batch` (and `--mixed-meter`), then
  `cargo test -p strudeland-host`.
- Engine jobs: no change. The job text comes from `pageJob` and is independent of a new top-level
  field.
- Goldens: no change.
- Render vectors (`loudness.json`, `joiner.json`): no change.
- `host:parity`: the new field is in the whole-plan comparison (it compares without `errors`
  only). Both sides compute it in the same `planRender`, so the field is equal by construction.
- The `--info` file of the `strudeland-render` binary lists its fields explicitly (`bin/strudeland-render.rs:17,185`), so it is unaffected unless chosen.
- Size: a per-bar array could hold up to `MAX_RENDER_BARS` = 100 000 strings. Runs
  `{fromBar, toBar, section}` are bounded by `loops × sections`. They must cover the lookback bars
  before `window.fromBar` (the `--section` case), so cover `[0, c1)` of the layout.
- "Names for given bars" would need a second host call (a fresh isolate and a re-evaluation),
  because the signatures are only known after Rust decodes the jobs. Reject that option.

## 5. Visible behaviour

No text changes for any reachable input. The only difference, at negative bars, cannot be reached
(§3).

## 6. New callers or copies since 63b945d

None. `refuse` and `take_jobs` have one caller each. No other Rust code maps clip bars to song
bars. The `splice`/`finish` `loop_range` is a frame range, unrelated. The TS users of `locate`
(`song.ts`, `timing.ts`, `event-edits.ts`, `control/events.ts`, `runtime.ts`) are already the
source of truth.

## Recommendation: proceed, with this scope

1. In `planRender`, add `sections: { fromBar, toBar, name }[]` to `PlannedRender`. It holds the
   runs of `locate(bar, structure, segments)` over the layout bars `[0, c1)`, with `segments`
   the ones the query used and bars with no section omitted.
2. Mirror the field in `planned.rs` (`Vec<SectionRun>`, `#[serde(default)]`).
3. `refuse` takes `&planned.sections` instead of the window and structure, then dedupes, does a
   binary-search lookup and sorts. Delete `section_at`, `section_of_clip_bar` and
   `section_of_song_bar`.
4. Move the mapping cases from `plan.rs:190-279` to a TS test of the runs: clip with loops 2/3,
   `loop_range`, linear, no sections, past the end, mixed meter, and the `--section` lookback.
5. Keep the Rust `refuse` tests on synthetic runs. Keep every text-pinning test unchanged (step 1
   table), and they must pass unedited.
6. Run `host:build`, `engine:refs --write`, `host:parity --batch` and `--mixed-meter`,
   `bun test`, and `cargo test --workspace`.

Decisions needed:
- **Representation:** runs, or one name per bar? Recommended: runs (bounded and compact).
- **Negative bars** (unreachable): keep `None`, or follow `locate`'s clamp to bar 0? Recommended:
  start the runs at bar 0, which keeps `None` for negative bars and today's behaviour. No visible
  change.
- **Should the field also appear in `strudeland-render --info`?** Recommended: no, to keep the
  scope small.
