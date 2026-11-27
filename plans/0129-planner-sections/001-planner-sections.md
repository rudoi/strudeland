# 001: section runs from the planner

## Goal

`planRender` returns `sections`: the runs of layout bars `[0, c1)` and the section each one
plays in, from `locate` on the segments the query used. The mapping cases that pin
`plan.rs:190-279` today are rewritten as TS tests of these runs. The host bundle is rebuilt,
the refs' provenance is rewritten and the three parity modes pass. The Rust mirror reads the
field as required, and `strudeland-render` still names sections its old way. An uncommitted
probe shows that the old way and the runs agree on every bar of real host plans. No engine
job, ref audio, golden, vector, refusal text or `--info` document changes.

## Prerequisites

- [000-prompt.md](000-prompt.md): the contract, the rules and the gate.
- A clean tree at `ee91dbd` plus the plan commits.

## Files

- Edited:
  - `src/lib/render/job-plan.ts`;
  - `tests/render-job-plan.test.ts`;
  - `crates/strudeland-host/src/planned.rs`, `crates/strudeland-host/src/lib.rs` (the
    re-export), `crates/strudeland-host/tests/plan_render.rs` (one new test);
  - `crates/strudeland-render/src/pipeline_audio.rs`: the destructure at `:68-78` and the test
    helper `planned` at `:284-302` only;
  - `crates/strudeland-host/README.md` (the result paragraph, `:76-79`);
  - `AGENTS.md:280-281`.
- Generated: `crates/strudeland-host/js/host.js`, `crates/strudeland-host/js/host.inputs.json`
  (`bun run host:build`), and `crates/strudeland-engine/tests/refs/**` (`bun run engine:refs
  --write`).
- Untouched, besides the shared list:
  - `crates/strudeland-render/src/plan.rs` (the probe is added in step 9 and removed before the
    commit), `error.rs`, `src/bin/`;
  - `crates/strudeland-render/src/testing.rs`. Its `:160` builds a `RenderedAudio`, not a
    `PlannedRender`, so it needs no field. The brief listed it; reverify §2 names it among the
    hand-built values, but it builds the audio result, not the plan;
  - `src/render/`, `src/host/`, `scripts/`, `src/lib/render/page-job.ts`.

## Steps

1. **Baseline, before any edit.** On the clean tree, write and record everything under
   `B="$S/0129-baseline"`:
   1. `cargo build --locked -p strudeland-cli -p strudeland-render -p strudeland-host --bins`,
      then copy `target/debug/{strudeland,strudeland-render,strudeland-host}` to `$B/bin/`.
   2. The A/B songs, in `$B/songs/`:
      - `refuse-sections.js`:

        ```js
        const meta = {
          bpm: 120,
          meter: '4/4',
          sections: [
            { name: 'intro', bars: 2 },
            { name: 'body', bars: 4, loop: true },
            { name: 'tail', bars: 2 },
          ],
          loops: 2,
        };

        $: song(meta, {
          intro: s("zzfx"),
          body: stack(note("c3").s('sine'), s("<~ ~ ~ kick>")),
          tail: s("hat"),
        })
        ```

      - `refuse-range.js`: the same, except the sections have no `loop: true`, `meta` has
        `loop: { from: 'intro', through: 'body' }`, and `intro` is `note("c3").s('sine')`;
      - `refuse-mixed.js`: the `meta` of `tests/render/mixed-meter-loop.js`, with `E:
        stack(note('e3').s('sine'), s("<~ clap>"))`, `ending: s("hat")` and the other parts
        as in that file;
      - `lookback.js`: `LOOKBACK_SONG` from `crates/strudeland-render/tests/pipeline.rs:259-272` (the text between `r#"` and `"#`);
      - `nosections.js`: `const meta = { bpm: 120, meter: '4/4' };` then `$: s("zzfx*2")`.
   3. `$B/refuse-cases.tsv`, one case per line: `tool<TAB>song<TAB>args`. `tool` is `cli`
      (`strudeland render SONG ARGS`) or `harness` (`strudeland-render render --song SONG
      ARGS`). In `args`, `@OUT` is the case's empty output directory, and `@REPO` in `song`
      is the repo root. Every case writes into `@OUT` (`--out @OUT/x.wav`, or `--out
      @OUT/stems` with `--stems`). At least these cases:
      - `refuse-sections.js` (cli): no flags; `--loops 1`; `--loops 3`; `--length 5`;
        `--length 12`; `--section intro`; `--section body`; `--section tail`;
        `--preset tagged-loop`; `--preset intro-loop`; `--preset folded-loop`;
        `--preset one-shot`; `--preset one-shot --section body`; `--stems orbit`;
        `--stems voice --section tail`;
      - `refuse-sections.js` (harness): `--from 0 --to 12 --layout clip`;
        `--from 0 --to 14 --layout clip`; `--from 2 --to 6 --layout linear`;
        `--from 0 --to 10 --layout linear`;
      - `refuse-range.js` (cli): no flags; `--loops 3`; `--section tail`;
        `--preset tagged-loop`; `--preset folded-loop`;
      - `refuse-mixed.js` (cli): no flags; `--loops 1`; `--section ending`;
        `--preset intro-loop`; `--preset folded-loop`;
      - `lookback.js` (cli): `--section body`; no flags;
      - `nosections.js` (cli): `--length 4`; `--length 8`;
      - `@REPO/tests/render/corpus/samples.js`: (cli) `--length 2` and `--length 4`;
        (harness) `--from 0 --to 2 --layout linear`.
   4. `$B/refuse-ab.sh record|check OUTDIR`. It runs every case with `$B/bin/` (record) or
      `target/debug/` (check), writing `OUTDIR/<n>.out`: the stdout, then `exit <code>`, then
      `ls -A` of the case's `@OUT`. It also writes `OUTDIR/<n>.err` (the stderr). `check`
      then runs `diff -r "$B/refuse-base" OUTDIR`. It prints `equal: <n> cases` and exits 0,
      or prints the diff and exits 1. `record` writes `$B/refuse-base`.
   5. `$B/plan-cases.tsv`, one case per line: `song<TAB>request-json`. For every
      `tests/render/corpus/*.js`, `tests/render/mixed-meter-loop.js` and every `$B/songs/*.js`,
      these requests:
      - `{"length":{"kind":"length","bars":4}}`;
      - `{"length":{"kind":"clip"}}`;
      - `{"length":{"kind":"clip","loops":3}}`;
      - `{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"}}`;
      - `{"length":{"kind":"window","fromBar":1,"toBar":3,"layout":"clip"},"chunk":{"overlapBars":1,"lookbackSec":"auto","mode":"mix"}}`;
      - `{"length":{"kind":"clip"},"preset":P}` for each of the four presets.

      Add `{"length":{"kind":"section","name":X}}` for every section of `song-linear.js`,
      `mixed-meter-loop.js` and the sectioned `$B/songs`. A request the song cannot satisfy
      is a case too: its error text is compared.
   6. `$B/plan-ab.sh record|check OUTDIR`. It runs `strudeland-host plan-render --song SONG
      --request JSON` for each case (`$B/bin/` or `target/debug/`). It writes
      `OUTDIR/<n>.out` (`exit <code>`) and `OUTDIR/<n>.err`. On success it also writes
      `OUTDIR/<n>.json`: `jq -S .` of stdout in `record`, and `jq -S 'del(.sections)'` in
      `check`. `record` also fails if any baseline plan has a `sections` key. `check` also
      fails unless every successful plan `has("sections")`, and it writes `jq -c .sections`
      to `OUTDIR/<n>.sections` for the report. Then it runs `diff -r` against
      `$B/plan-base`, ignoring `*.sections`, and prints `equal: <n> cases`. JSON strings
      (each chunk's `json`, the job text) pass through `jq` unchanged, so the job texts are
      compared byte for byte.
   7. `$B/refs-ab.sh`. It must exit 0 only if, for `R=crates/strudeland-engine/tests/refs`:
      - `git status --porcelain -- "$R"` shows no untracked or deleted file;
      - every changed `*.wav` is `mod-lfo-note.wav` or `vowel-formants.wav`;
      - every changed `*.json` other than `index.json` is byte-equal to its `ee91dbd`
        version once both have every `"plannerSha256": "<64 hex>"` value (with or without a
        space after the colon) replaced by `X`. Compare with `cmp`, not `jq`;
      - after the same replacement, `index.json`'s `diff` against `ee91dbd` has only
        `"sha256":` and `"bytes":` lines under `files`, plus the `"floorDb":` lines of the two
        floor cases. Every `files` entry's `sha256` and `bytes` match the file on disk;
      - every `plannerSha256` in `$R` has the same new value (`grep -oh … | sort -u | wc -l`
        is 1) and differs from `b8342ee4…`.

      It prints the counts: JSON files changed, WAVs changed.
   8. `$B/host-diff.sh`. Using `git diff -U0 ee91dbd -- crates/strudeland-host/js/host.js`,
      every hunk's new-side line range must lie between the line of
      `  // src/lib/render/job-plan.ts` and the next `  // src/` line of the new file. A
      pure deletion's anchor counts too. `git diff -U0 ee91dbd -- crates/strudeland-host/js/host.inputs.json`
      must change only the `"src/lib/render/job-plan.ts":` line.
   9. Run `"$B/refuse-ab.sh" record` and `"$B/plan-ab.sh" record`, then run each again
      in `check` mode against the baseline binaries (temporarily point check at `$B/bin`)
      to show the runs are deterministic. If a line differs between two baseline runs, the
      script filters exactly that line kind (say which in the report), or the scripts are
      unusable: STOP.
   10. Read `$B/refuse-base`. Every `refuse-*`, `lookback`, `nosections` and `samples` case
       must exit 1 with `the song uses sounds or controls the engine cannot play yet`. The
       one exception is a case that fails as a usage error at baseline, which is recorded as
       is. Together the cases must name `'intro'`, `'body'`, `'tail'`, `'E'` and `'ending'`,
       and must include texts with no section. If a song does not refuse as intended (a
       sound turns out to be playable), change that song's sound to another one that
       `samples.js` shows is refused, and re-record. This is the only point where a baseline
       may be re-recorded, and no code has been edited yet.
2. **`job-plan.ts`: the type.** After `ChunkInfo` (`:244-248`), add:

   ```ts
   /** Layout bars `[fromBar, toBar)` that play in the section `name`. */
   export interface PlannedSection {
     fromBar: number;
     toBar: number;
     name: string;
   }
   ```

   In `PlannedRender` (`:258-282`), add `sections: PlannedSection[]` right after `structure`,
   with a doc comment. It says the field holds the section each layout bar in `[0, c1)` plays
   in: `c1` is `window.toBar` plus a single chunk's overlap. It is computed by `locate` on the
   segments the schedule query used, as runs in bar order, with a bar in no section in no
   run. The runs start at bar 0, so they cover the lookback before `window.fromBar` and a
   loop preset's repeat pass. The engine's refusal names a first bar's section from them.
3. **`job-plan.ts`: the computation.** Change `import type { Segment } from
   '../strudel/transport'` (`:18`) to `import { locate, type Segment } from
   '../strudel/transport'`. Add a private helper next to `onsetIndex`:

   ```ts
   /** The runs of `[0, c1)` that `locate` places in one section, merged while the section
    *  index stays the same on consecutive bars. */
   function sectionRuns(
     structure: SongStructure,
     segments: readonly Segment[],
     c1: number,
   ): PlannedSection[] {
     const runs: PlannedSection[] = [];
     let open = -1; // the section index of runs.at(-1) when it ends at `bar`, else -1
     for (let bar = 0; bar < c1; bar++) {
       const at = locate(bar, structure, segments);
       if (!at) {
         open = -1;
         continue;
       }
       if (at.section === open) runs.at(-1)!.toBar = bar + 1;
       else runs.push({ fromBar: bar, toBar: bar + 1, name: structure.sections[at.section]!.name });
       open = at.section;
     }
     return runs;
   }
   ```

   In `planRender`, after `const c1 = …` (`:675`), add `const sections =
   sectionRuns(structure, segments, c1);`. Use the `structure` and `segments` consts of
   `:560-563`, not `song.structure`. In the returned object (`:689-718`), add `sections,`
   right after `structure,`. Update `planRender`'s doc comment (`:536-546`) with one clause
   saying it names the section of every layout bar it schedules (`sections`).

   Cost: at most `MAX_RENDER_BARS` `locate` calls, each with a scan of at most 64 sections.
   Step 17 of the gate is the check on it.
4. **TS tests**, in `tests/render-job-plan.test.ts`, a new `describe('planRender sections',
   …)`. It uses the file's `fakeRuntime`, `renderReq` and `buildStructure`.
   `SONG_LINEAR = buildStructure({ bpm: 120, loops: 2, sections: [{ name: 'intro', bars: 2 },
   { name: 'body', bars: 4, loop: true }, { name: 'tail', bars: 2 }] })` mirrors `plan.rs`'s
   `song_linear()`. Write `r(from, to, name)` for `{ fromBar, toBar, name }`. Each case
   asserts `got.sections` with `toEqual`:
   - **clip, loops 2** (`plan.rs:191-213`): `[r(0,2,'intro'), r(2,10,'body'), r(10,12,'tail')]`.
   - **`--loops 3`** (`:214-223`): `[r(0,2,'intro'), r(2,14,'body'), r(14,16,'tail')]`.
   - **`--loops 1`**: `[r(0,2,'intro'), r(2,6,'body'), r(6,8,'tail')]`.
   - **`loop_range`** (`:226-238`): `SONG_LINEAR`'s sections without `loop: true`, with
     `loop: { from: 'intro', through: 'body' }` and `loops: 2`. First assert `clipBars` is
     14, then `[r(0,2,'intro'), r(2,6,'body'), r(6,8,'intro'), r(8,12,'body'),
     r(12,14,'tail')]`.
   - **linear** (`:240-258`): window `{ fromBar: 0, toBar: 8, layout: 'linear' }` gives
     `[r(0,2,'intro'), r(2,6,'body'), r(6,8,'tail')]`.
   - **past the song**: window `0..10` linear gives the same three runs, ending at 8. Window
     `0..14` clip gives the clip runs, ending at 12.
   - **no loop section** (`:259-276`): sections `a` 3 and `b` 1 give `[r(0,3,'a'), r(3,4,'b')]`
     for both a clip and a linear window `0..6`.
   - **no sections** (`:277-278`): with structure `null` and the plain `CODE` header,
     `{ kind: 'length', bars: 4 }` gives `[]`, and so does a clip window `0..2`.
   - **`--length` with sections**: `{ kind: 'length', bars: 5 }` gives clip
     `[r(0,2,'intro'), r(2,5,'body')]`.
   - **`--section` lookback** (`plan.rs:366-380`, `tests/pipeline.rs:274-304`):
     - `{ kind: 'section', name: 'body' }` (window linear `2..6`) gives
       `[r(0,2,'intro'), r(2,6,'body')]`, so the lookback bars before `fromBar` are named;
     - `name: 'tail'` gives all three linear runs.
   - **presets**:
     - `tagged-loop` and `intro-loop` (window linear `0..6`, with `export.repeat`) give
       `[r(0,2,'intro'), r(2,6,'body')]`. Assert that every repeat chunk's
       `[plan.fromBar, plan.toBar)` lies inside the runs' span;
     - `folded-loop` (window `2..6`) gives the same two runs;
     - `one-shot` gives the clip runs.
   - **chunk overlap**: window `0..4` linear with `chunk: { overlapBars: 2, lookbackSec: 0,
     mode: 'mix' }` gives `[r(0,2,'intro'), r(2,6,'body')]`, covering to `c1` = 6.
   - **mixed meter**: `buildStructure` of `tests/render/mixed-meter-loop.js`'s `meta`. Copy it
     into the test; the file is not imported. Assert `clipBars` is 22, then the clip gives
     `[r(0,1,'opening'), r(1,4,'F1'), r(4,7,'F2'), r(7,11,'E'), r(11,14,'F1'),
     r(14,17,'F2'), r(17,21,'E'), r(21,22,'ending')]`. The meter changes move no bar.
     `tagged-loop` gives the first four runs.
   - **the evaluated structure**: in the existing `describe('planRender follows the evaluated
     structure')` setup (`evaluated`, `:1173-1182`), `{ kind: 'clip', loops: 2 }` gives
     `[r(0,4,'intro'), r(4,44,'main'), r(44,48,'outro'), r(48,50,'coda')]`. The runs read
     the `--loops`-applied structure.
   - **agrees with `locate`**: for each request above, for every bar `b` in `[0, c1)`, the run
     containing `b` (or none) names the same section as `locate(b, got.structure,
     layoutSegments(got.window.layout, got.structure))`. The runs are sorted, non-empty and
     disjoint, and two touching runs never share a section index. Import `locate` and
     `layoutSegments` for this test.

   Where a case's expected runs differ from the `plan.rs` case it mirrors (for example the
   past-the-song bars), say so in a comment: the runs stop at `c1`, while `plan.rs` answered
   for any bar. Targeted: `bun test tests/render-job-plan.test.ts`.
5. **`planned.rs`:**
   - add, after `PlannedWindow`:

     ```rust
     /// Layout bars `[from_bar, to_bar)` that play in section `name` (`PlannedSection` in
     /// `job-plan.ts`).
     #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
     #[serde(rename_all = "camelCase")]
     pub struct PlannedSection {
         pub from_bar: u32,
         pub to_bar: u32,
         pub name: String,
     }
     ```

   - in `PlannedRender`, after `structure`, add `pub sections: Vec<PlannedSection>`, with no
     serde attribute. Its doc comment says: the section of each layout bar from 0 to the
     window's end (plus a single chunk's overlap), as runs in bar order, with a bar in no
     section in no run;
   - the module doc gains one sentence: `sections` is required because the bundle is compiled
     in (`bundle.rs`), and a plan without it is malformed rather than unnamed;
   - in `lib.rs`, add `PlannedSection` to the `planned` re-export (`:30-33`);
   - tests:
     - `PLANNED` (`:183`) gains
       `"sections":[{"fromBar":0,"toBar":4,"name":"a"},{"fromBar":4,"toBar":12,"name":"b"}],`
       right after the `structure` object;
     - `the_bundle_shape_parses_exactly` asserts those two runs, and that the re-serialized
       text contains `"sections":[{"fromBar":0,"toBar":4,"name":"a"},`;
     - `a_wrong_shape_is_rejected` gains three cases: the whole `"sections":[…],` removed;
       `"fromBar":0,"toBar":4,"name":"a"` with `"fromBar":-1`; and `"name":"a"` replaced with
       `"name":1`;
     - every other test keeps its assertions and still passes on the new `PLANNED`.
6. **`pipeline_audio.rs`:** add `sections: _,` to the `PlannedRender` destructure
   (`:68-78`), and `sections: vec![],` to the test helper `planned` (`:290-301`). Nothing else.
7. **`crates/strudeland-host/tests/plan_render.rs`:** add
   `sections_name_the_layout_bars_of_a_real_plan`. Plan `song-linear.js` (`common::corpus_song`)
   as a clip, and as `--section body` (`LengthRequest::Section`), through `Host::plan_render`.
   Assert the clip runs `intro 0..2, body 2..10, tail 10..12` and the section runs
   `intro 0..2, body 2..6`. This is the end-to-end check that the bundle emits the field and
   the mirror reads it.
8. **Docs** (made stale by this stage):
   - `AGENTS.md:280-281`, the render pipeline bullet: `planRender` "(resolve the length
     request, evaluate once, query the schedule once, plan every chunk's job, or one chunk for
     the harness with `chunk`, and name the section of every layout bar it schedules,
     `sections`)";
   - `crates/strudeland-host/README.md:76-79`, the result paragraph: add `sections`, "the runs
     of layout bars from 0 to the window's end (plus a single chunk's overlap) and the section
     each plays in, as `locate` places them on the plan's segments".
9. **Equivalence probe (uncommitted).** Append a `#[cfg(test)] mod sections_probe` to
   `crates/strudeland-render/src/plan.rs`. For each song in `tests/render/corpus/song-linear.js`,
   `tests/render/mixed-meter-loop.js`, `$B/songs/*.js` (read by absolute path) and each
   request in this list:
   - clip;
   - clip with loops 1 and 3;
   - `--length 5`;
   - every section;
   - each of the four presets;
   - the clip window `0..clip_bars + 2`;
   - the linear window `0..total_bars + 2`.

   Run `plan(code, &opts, preset)`, with `RenderOptions` built as
   `crates/strudeland-render/tests/common/mod.rs:39` `options` builds them. Skip requests
   the song cannot satisfy, and print them. For every bar in `-1..planned.window.to_bar`,
   assert that `section_at(planned.window.layout, &planned.structure, bar)` equals the name
   of the run in `planned.sections` that contains `bar`, or `None`. Print the number of bars
   checked. Run gate step 12, then delete the module, so that `git diff ee91dbd --
   crates/strudeland-render/src/plan.rs` is empty.

## Tests

- New: the TS `planRender sections` describe (step 4); `planned.rs`'s new assertions and
  wrong-shape cases (step 5); `sections_name_the_layout_bars_of_a_real_plan` (step 7).
- Changed inputs, same assertions: the `PLANNED` string (step 5) and the
  `pipeline_audio.rs` helper (step 6).
- Unchanged and passing: every `plan.rs` test, including the mapping tests, which stage 002
  removes; every text-pinning test in [000-prompt.md](000-prompt.md); the bun staleness
  tests for `host.inputs.json` and the refs provenance (they pass only after steps 1 and 4
  of the gate).
- Targeted while working: `bun test tests/render-job-plan.test.ts`,
  `cargo test -p strudeland-host --lib planned::`,
  `cargo test -p strudeland-host --test plan_render`, `cargo test -p strudeland-render --lib`.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=001`, all steps. Also:

```sh
git diff --stat ee91dbd -- crates/strudeland-render/src/plan.rs crates/strudeland-render/src/error.rs \
  crates/strudeland-render/src/bin crates/strudeland-render/src/testing.rs src/render src/host scripts \
  src/lib/render/page-job.ts > "$S/0129-001-own-untouched.log"; echo "exit $?" >> "$S/0129-001-own-untouched.log"
git diff ee91dbd -- crates/strudeland-render/src/pipeline_audio.rs > "$S/0129-001-pipeline-audio.log"
```

- `own-untouched.log` lists no files. In particular, the probe is gone.
- `pipeline-audio.log` shows exactly the two added lines from step 6.
- What stays unchanged, and the check for each:
  - every engine job text, by plan A/B (host) and `host:parity` (page against host),
    including `--batch` against the page's single-chunk jobs;
  - ref audio, by refs A/B;
  - goldens and vectors, by gate step 7 and the vector checks;
  - refusal texts, by refusal A/B, which passes here trivially because no Rust refusal code
    changed. It is run anyway as 002's control;
  - the `--info` document, because `bin/strudeland-render.rs` is untouched and its `Info`
    names its fields.

## STOP

- A step 1 baseline that is not deterministic, or a song that cannot be made to refuse as
  step 1.10 requires.
- The probe finds a bar where `section_at` and the runs disagree. Report the song, the
  request, the bar and both answers. Do not adjust `sectionRuns` toward `plan.rs` without
  the user's decision: the transport is the source of truth, and a disagreement means a
  refusal text would change in 002.
- Anything in the shared STOP list.

## Commit message

```
refactor(render): name the sections of a render's bars in the planner

planRender now returns the section each layout bar of the render plays
in, as runs from bar 0 through the end of the schedule, computed with
the transport's own locate on the segments the query used. The host's
plan mirror reads the field; the renderer does not use it yet. No job,
ref audio or golden changes; the refs record the new planner hash.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
