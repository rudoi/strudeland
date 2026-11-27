# 001: typed length codes

## Goal

Every length-request error the planner throws carries a `LengthProblem`: a code from the
proposal's table plus its parameters. It goes from `job-plan.ts` through the host slot to
`HostError::MetaFlag { problem, message }`. The CLI keeps printing `message`, so its output is
byte-identical. The desktop dialog shows `export::dialog_text(problem)`. The bundle is rebuilt,
the refs record the new planner hash, and the three parity modes pass. No engine job, golden,
vector, ref audio (beyond the floor cases), CLI text, exit code or IPC binding changes.

## Prerequisites

- [000-prompt.md](000-prompt.md): the contract, the rules and the gate.
- A tree code-identical to `0113154`. Only `plans/` differs.

## Files

- Edited:
  - `src/lib/render/job-plan.ts`;
  - `src/host/main.ts`;
  - `tests/render-job-plan.test.ts`;
  - `tests/host-entry.test.ts`;
  - `crates/strudeland-host/src/error.rs`, `src/host.rs` (the `Slot`, `read_slot` and its
    tests, and the `plan_render` doc at `:95`), `src/lib.rs` (the re-export);
  - `crates/strudeland-host/tests/plan_render.rs`;
  - `crates/strudeland-host/README.md` (`:19-21`, `:63-65`);
  - `crates/strudeland-render/src/lib.rs` (`:57-59`, the re-export),
    `src/error.rs` (the doc at `:10-12`, the doc at `:62-63`, the test at `:128-133`),
    `src/export/mod.rs` (`mod wording; pub use wording::dialog_text;`), the new
    `src/export/wording.rs`;
  - `crates/strudeland-render/tests/pipeline.rs` (`:316-319` and `:340-343`, the `matches!`
    only);
  - `crates/strudeland-cli/src/render.rs`, the test at `:1332-1341` (the construction only);
  - `src-tauri/src/export_audio.rs` (`export_message` and its test);
  - `AGENTS.md:298` and the `export/` item at `:308-310`.
- Generated:
  - `crates/strudeland-host/js/host.js` and `crates/strudeland-host/js/host.inputs.json`, by
    `bun run host:build`;
  - `crates/strudeland-engine/tests/refs/**`, by `bun run engine:refs --write`.
- Frozen (gate step 7, against `0113154`):

  ```
  tests/render/goldens crates/strudeland-render/tests/vectors crates/strudeland-render/tests/golden
  crates/strudeland-engine/tests/vectors crates/strudeland-engine/src
  crates/strudeland-cli/skill crates/strudeland-cli/tests crates/strudeland-cli/strudel-docs.json
  crates/strudeland-cli/src/run.rs crates/strudeland-cli/src/output.rs crates/strudeland-ctl
  crates/strudeland-host/tests/bin.rs crates/strudeland-host/tests/never_panics.rs
  crates/strudeland-render/tests/bin.rs crates/strudeland-render/src/bin crates/strudeland-render/src/plan.rs
  crates/strudeland-render/src/export/request.rs crates/strudeland-render/src/export/error.rs
  src/lib/ipc src/lib/control src/lib/strudel src/lib/export src/components src/render scripts
  src-tauri/src/command_error.rs
  ```

## Steps

1. **Baseline, before any edit.** On the untouched tree, write and record everything under
   `B="$S/0144-baseline"`:
   1. Run `cargo build --locked -p strudeland-cli -p strudeland-render -p strudeland-host
      --bins`, then copy `target/debug/{strudeland,strudeland-render,strudeland-host}` to
      `$B/bin/`.
   2. Write the songs in `$B/songs/`. `N` below stands for `note("c3").s('sine')`. Write it out
      in each file; do not use a shared binding unless a song fails to evaluate without one
      (see step 1.8).
      - `plain.js`: `const meta = { bpm: 120, meter: '4/4' };` then `$: note("c3").s('sine')`;
      - `sections.js`: `meta = { bpm: 120, meter: '4/4', sections: [{ name: 'intro', bars: 2
        }, { name: 'body', bars: 4, loop: true }, { name: 'tail', bars: 2 }] }`, then
        `$: song(meta, { intro: N, body: N, tail: N })`. Its clip is 8 bars;
      - `long-loop.js`: sections `{ name: 'a', bars: 2 }, { name: 'b', bars: 10000, loop: true
        }`, `bpm: 120`, then `$: song(meta, { a: N, b: N })`. Its clip is 10002 bars;
      - `long-loop-meta.js`: the same with `loops: 10` in `meta`. Its clip is 100002 bars;
      - `long-plain.js`: eleven sections `s0` … `s10` of `bars: 10000`, no loop, `bpm: 120`,
        then `$: song(meta, { s0: N, …, s10: N })`. Its clip is 110000 bars;
      - `long-last-loop.js`: `long-plain.js` with `loop: true` on `s10`;
      - `long-range.js`: `long-plain.js` with `loop: { from: 's9', through: 's10' }` in `meta`.

      Strings in `meta` use single quotes (AGENTS.md).
   3. Write `$B/length-cases.tsv`, one case per line: `tool<TAB>song<TAB>args<TAB>expect`.
      - `tool` is `cli` (`strudeland render SONG ARGS --out @OUT/x.wav`) or `harness`
        (`strudeland-render render --song SONG ARGS --out @OUT/x.wav`). A `--stems` case uses
        `--out @OUT/stems` instead.
      - `@OUT` is `$S/0144-out/<n>` for case `<n>`. The script removes and recreates it before
        every run of the case, so a printed path is the same in `record` and `check`.
      - `expect` is the code letter the case must give at baseline (reverify §1), or `cli` for
        the CLI's own preset guard. Step 1.8 checks it.

      These 35 cases:

      | Song | Tool | Args → expect |
      |---|---|---|
      | `plain.js` | cli | (none) → D; `--section a` → G; `--loops 2` → B; `--preset tagged-loop` → K; `--preset intro-loop` → K; `--preset folded-loop` → K; `--preset one-shot` → D; `--stems orbit` → D; `--stems voice --section a` → G |
      | `sections.js` | cli | `--section drop` → H; `--length 9` → J; `--preset one-shot --length 9` → J; `--stems orbit --length 9` → J; `--preset tagged-loop --section body` → cli; `--preset tagged-loop --loops 2` → cli |
      | `long-loop.js` | cli | `--loops 1000` → E; `--loops 10` → E |
      | `long-loop-meta.js` | cli | (none) → F (with `or --loops N`); `--stems orbit` → F |
      | `long-plain.js` | cli | (none) → F (without `--loops`); `--section s10` → I; `--preset one-shot --section s10` → I |
      | `long-last-loop.js` | cli | `--preset tagged-loop` → I; `--preset folded-loop` → I |
      | `long-range.js` | cli | `--preset tagged-loop` → L; `--preset intro-loop` → L |
      | `plain.js` | harness | (none) → D; `--section a` → G; `--loops 2` → B |
      | `sections.js` | harness | `--section drop` → H; `--length 9` → J |
      | `long-loop.js` | harness | `--loops 1000` → E |
      | `long-loop-meta.js` | harness | (none) → F |
      | `long-plain.js` | harness | (none) → F; `--section s10` → I |

      That is 26 `cli` cases and 9 `harness` cases. A and C cannot be reached from either
      tool. The plan A/B reaches A through the host (step 1.5).
   4. Write `$B/length-ab.sh record|check OUTDIR`. It `cd`s to the repo root first.
      - Binaries: `record` always uses `$B/bin/`; `check` uses `BIN=${BIN:-target/debug}`.
      - For every case it writes `OUTDIR/<n>.out` (stdout, then `exit <code>`, then `ls -A` of
        the case's `@OUT`) and `OUTDIR/<n>.err` (stderr).
      - `record` writes `$B/length-base`. `check` runs `diff -r "$B/length-base" OUTDIR`,
        then prints `equal: <n> cases` and exits 0, or prints the diff and exits 1.
   5. Write `$B/plan-cases.tsv`, one case per line: `song<TAB>request-json`.
      - For every `tests/render/corpus/*.js`, `tests/render/mixed-meter-loop.js`,
        `$B/songs/plain.js` and `$B/songs/sections.js`, add these requests:
        - `{"length":{"kind":"length","bars":4}}`;
        - `{"length":{"kind":"clip"}}`;
        - `{"length":{"kind":"clip","loops":3}}`;
        - `{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"}}`;
        - `{"length":{"kind":"window","fromBar":1,"toBar":3,"layout":"clip"},"chunk":{"overlapBars":1,"lookbackSec":"auto","mode":"mix"}}`;
        - `{"length":{"kind":"clip"},"preset":P}` for each of the four presets.
      - Add `{"length":{"kind":"section","name":X}}` for every section of `song-linear.js`,
        `mixed-meter-loop.js` and `sections.js`, and for `X` = `drop` on `sections.js`.
      - Add `{"length":{"kind":"length","bars":2},"preset":"tagged-loop"}` and
        `{"length":{"kind":"clip","loops":2},"preset":"tagged-loop"}` on `sections.js`. These
        are A, which only the host reaches.
      - Add these for the long songs only, because a plan of 10000 bars or more is slow:
        `{"length":{"kind":"clip"}}` on each long song;
        `{"length":{"kind":"clip","loops":1000}}` on `long-loop.js`;
        `{"length":{"kind":"section","name":"s10"}}` on `long-plain.js`; and
        `{"length":{"kind":"clip"},"preset":"tagged-loop"}` on `long-last-loop.js` and
        `long-range.js`.
      - A request the song cannot satisfy is a case too: its stderr and exit code are compared.
   6. Write `$B/plan-ab.sh record|check OUTDIR`. It runs `strudeland-host plan-render --song
      SONG --request JSON` for each case, with its binaries read as in `length-ab.sh`.
      - It writes `OUTDIR/<n>.out` (`exit <code>`) and `OUTDIR/<n>.err` (stderr).
      - On success it also writes `OUTDIR/<n>.json`, which is `jq -S .` of stdout. No plan
        field is added in this task, so nothing is deleted before the compare. JSON strings
        pass through `jq` unchanged, so job texts are compared byte for byte.
      - `check` runs `diff -r "$B/plan-base" OUTDIR` and prints `equal: <n> cases`.
   7. Write `$B/refs-ab.sh`. With `R=crates/strudeland-engine/tests/refs`, it must exit 0 only
      if all of these hold:
      - `git status --porcelain -- "$R"` shows no untracked or deleted file;
      - every changed `*.wav` is `mod-lfo-note.wav` or `vowel-formants.wav`;
      - every changed `*.json` other than `index.json` is byte-equal to its `0113154` version
        once both have every `"plannerSha256": "<64 hex>"` value replaced by `X` (with or
        without a space after the colon). Compare with `cmp`, not `jq`;
      - after the same replacement, `index.json`'s diff against `0113154` has only `"sha256":`
        and `"bytes":` lines under `files`, plus the two floor cases' `"floorDb":` lines. Every
        `files` entry's `sha256` and `bytes` match the file on disk;
      - every `plannerSha256` in `$R` has the same new value, and it differs from
        `860326f8f6ed525ee13a9fe85f652c645bf75e667f3fc219a818305d9c6ab0db`.

      It prints the counts of JSON files and WAVs changed.
   8. Write `$B/host-diff.sh`. From `git diff -U0 0113154 -- crates/strudeland-host/js/host.js`,
      every hunk's new-side range (a pure deletion's anchor line included) must lie in an
      allowed region of the **new** file:
      - from the line `  // src/lib/render/job-plan.ts` to the line before the next line that
        matches `^  // ` (at `0113154`, lines 18577-18946);
      - from each line `  // src/host/main.ts` to the line before the next `^  // ` line, or
        to the end of file (at `0113154`, 11446-11456 and 32970-33139).

      Compute the regions from the new file; do not hard-code those numbers. Also, `git diff
      -U0 0113154 -- crates/strudeland-host/js/host.inputs.json` must change exactly three
      lines: `"src/host/main.ts":`, `"src/lib/render/job-plan.ts":` and `"combined":`. The
      script prints the hunk count per region.
   9. Run `"$B/length-ab.sh" record` and `"$B/plan-ab.sh" record`. Then run each in `check`
      mode against the baseline binaries, with no script edit:
      `BIN="$B/bin" "$B/length-ab.sh" check "$S/0144-det-length"` and
      `BIN="$B/bin" "$B/plan-ab.sh" check "$S/0144-det-plan"`. Both must be equal.
      - If a line differs between two baseline runs, add a filter for exactly that line kind
        (say which in the report), record both again and repeat.
      - If they still differ, STOP.

      The scripts are final once this pass is equal.
   10. Read `$B/length-base` and check each case against its `expect`. A planner case must
       exit 4 with stderr `error: <the site's text>` (cli) or `strudeland-render: <the site's
       text>` (harness). The site's texts are in reverify §1. F must carry `or --loops N` on
       `long-loop-meta.js` and must not on `long-plain.js`. A `cli` case must exit 4 with the
       CLI guard's text (`render.rs:44-47`). Every `ls -A` must be empty.

       If a song does not reach its code, for example because a `song()` part is missing or a
       binding is needed, fix that song once and re-record both baselines (steps 1.9-1.10). This
       and step 1.9's filter are the only times a baseline is re-recorded, and no code has been
       edited yet. A second miss is a STOP.
2. **`job-plan.ts`: the type.** Above `PlanJobError` (`:52`), add an exported union
   `LengthProblem` with exactly the twelve members of the proposal's table:

   ```ts
   | { code: 'presetWithLength' }
   | { code: 'loopsWithoutLoop' }
   | { code: 'loopsInvalid'; message: string }
   | { code: 'noSections' }
   | { code: 'loopsPastLimit'; loops: number; clipBars: number; limit: number }
   | { code: 'clipPastLimit'; clipBars: number; limit: number; hasLoop: boolean }
   | { code: 'sectionWithoutSections' }
   | { code: 'unknownSection'; name: string; sections: string[] }
   | { code: 'sectionPastLimit'; name: string; toBar: number; limit: number }
   | { code: 'lengthPastEnd'; bars: number; clipBars: number }
   | { code: 'presetWithoutLoop' }
   | { code: 'loopPastLimit'; end: number; limit: number }
   ```

   Give each member a one-line doc that names the request it refuses. Add an exported
   `lengthFlag(problem: LengthProblem): '--length' | '--section' | '--loops' | '--preset'`,
   an exhaustive `switch` that returns the proposal table's flag column.

   Change `PlanJobError`:
   - `declare public readonly flag?: string` becomes `declare public readonly problem?:
     LengthProblem`;
   - the third constructor parameter becomes `problem?: LengthProblem`, set only when given,
     so `'problem' in e` is false otherwise, as `'flag' in e` is today;
   - the doc comment (`:53-55`) says that `problem` is the typed length-request error, and that
     only `planRender` sets it, and only with kind `'meta'`.
3. **`job-plan.ts`: the twelve sites.** At each site, replace the flag argument with the
   problem. **Every message string stays exactly as it is** (the same template literal, the same
   `limit` and `advice` locals at `:430` and `:438`). The problems:
   - A: `{ code: 'presetWithLength' }`;
   - B: `{ code: 'loopsWithoutLoop' }`;
   - C: `{ code: 'loopsInvalid', message: e.message }`;
   - D: `{ code: 'noSections' }`;
   - E: `{ code: 'loopsPastLimit', loops: length.loops, clipBars, limit: MAX_RENDER_BARS }`;
   - F: `{ code: 'clipPastLimit', clipBars, limit: MAX_RENDER_BARS, hasLoop:
     authoredLoop(structure) !== null }`. Compute `hasLoop` once, and derive `advice` from it,
     so the two cannot disagree;
   - G: `{ code: 'sectionWithoutSections' }`;
   - H: `{ code: 'unknownSection', name: length.name, sections: sections.map((s) => s.name)
     }`. Build the message's list from the same array;
   - I: `{ code: 'sectionPastLimit', name: section.name, toBar, limit: MAX_RENDER_BARS }`;
   - J: `{ code: 'lengthPastEnd', bars: length.bars, clipBars: structure.clipBars }`;
   - K: `{ code: 'presetWithoutLoop' }`;
   - L: `{ code: 'loopPastLimit', end: loop.end, limit: MAX_RENDER_BARS }`.

   Update the comments that say "flagged" (`:388`, `:409`) and `planRender`'s doc (`:580`,
   "length errors are kind 'meta' with a `problem`"). Add no other top-level identifier besides
   `LengthProblem` and `lengthFlag`. If you need a helper, first check with `grep -c '\bNAME\b'
   crates/strudeland-host/js/host.js` that its name gives 0.
4. **`src/host/main.ts`.** In `HostResult` (`:40`), replace `flag?: string` with
   `length?: LengthProblem`, and import the type from `job-plan`. In `failure` (`:77-85`), write
   `...(e.problem === undefined ? {} : { length: e.problem })`. Nothing else changes.
5. **TS tests.**
   - `tests/render-job-plan.test.ts`:
     - `planRender length errors` (`:626-686`) and `planRender render limit` (`:1458-1501`):
       each row keeps its message and flag columns and gains the expected problem. The test
       asserts `e.problem` with `toEqual`, `lengthFlag(e.problem!)` against the flag column,
       and the message with `toBe`. For example: H gives `{ code: 'unknownSection', name:
       'bridge', sections: ['intro', 'main', 'outro'] }`; J (25) gives `{ code:
       'lengthPastEnd', bars: 25, clipBars: 24 }`; F gives `{ code: 'clipPastLimit', clipBars:
       110000, limit: 100000, hasLoop: true }`; E gives `{ code: 'loopsPastLimit', loops:
       1000, clipBars: 10010000, limit: 100000 }`;
     - the "without a flag" tests (`:688-723`): `e.problem` is undefined, and `'problem' in e`
       is false;
     - `a clip without a loop section names only --length` (`:1505`) also asserts `hasLoop:
       false`;
     - `PlanJobError flag` (`:1162-1170`) becomes `PlanJobError problem`. It is set only when
       given, and `'problem' in plain` is false;
     - `:1442-1446` asserts `problem: { code: 'lengthPastEnd', bars: 71, clipBars: 70 }` in
       place of `flag`;
     - `:1646-1661`: every case has `lengthFlag(e.problem!) === '--preset'`. The `clip` case is
       `presetWithoutLoop`, and the other three are `presetWithLength`. Confirm this against
       the fake's structure, and say so in the report if it differs;
     - new cases, each asserting problem, flag and **exact** message, and that nothing was
       queried:
       - A: `{ kind: 'length', bars: 1 }` with `tagged-loop`;
       - K: `tagged-loop` on `CODE`;
       - L: a structure of eleven 10000-bar sections with `loop: { from: 's9', through: 's10'
         }` and `tagged-loop`, giving `the loop ends at bar 110000, past the render limit of
         100000 bars`;
       - I through a preset: the same sections with `loop: true` on `s10` and `tagged-loop`,
         giving `{ code: 'sectionPastLimit', name: 's10', toBar: 110000, limit: 100000 }`;
     - `lengthFlag` of one example of each of the twelve codes. The test writes the examples
       as a `LengthProblem[]` literal, so `bun run check` proves each is well-typed. It
       asserts the proposal table's flag column. C has no throw test, because no input reaches
       it (proposal, Open questions 2).
   - `tests/host-entry.test.ts:245-293`:
     - the slot is `{ ok: false, kind: 'meta', error: '<D text>', length: { code: 'noSections'
       } }`;
     - the section case is `[section.kind, section.length]` equal to `['meta', { code:
       'sectionWithoutSections' }]`;
     - `'flag' in …` becomes `'length' in …` (false);
     - the bad-request loop asserts `length: undefined`;
     - one new assertion: no slot has a `flag` key.

   Targeted: `bun test tests/render-job-plan.test.ts tests/host-entry.test.ts`. The host entry
   test runs the source, not the bundle.
6. **`strudeland-host`: `LengthProblem`.** In `src/error.rs`, add:

   ```rust
   /// A render's length request the song cannot satisfy (`LengthProblem` in `job-plan.ts`), as
   /// the bundle's result slot carries it.
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(tag = "code", rename_all = "camelCase", rename_all_fields = "camelCase")]
   pub enum LengthProblem {
       PresetWithLength,
       LoopsWithoutLoop,
       LoopsInvalid { message: String },
       NoSections,
       LoopsPastLimit { loops: u32, clip_bars: u64, limit: u64 },
       ClipPastLimit { clip_bars: u64, limit: u64, has_loop: bool },
       SectionWithoutSections,
       UnknownSection { name: String, sections: Vec<String> },
       SectionPastLimit { name: String, to_bar: u64, limit: u64 },
       LengthPastEnd { bars: u64, clip_bars: u64 },
       PresetWithoutLoop,
       LoopPastLimit { end: u64, limit: u64 },
   }
   ```

   Give each variant a doc line. Then:
   - add `pub fn flag(&self) -> &'static str`, an exhaustive match that returns the table's
     flag;
   - change `MetaFlag { flag: String, message: String }` to `MetaFlag { problem: LengthProblem,
     message: String }`, keeping `#[error("meta: {message}")]`;
   - update the enum doc (`:7-8`): a `MetaFlag` carries the typed problem and the planner's
     message, and `LengthProblem::flag` names its CLI flag;
   - `kind()` and `is_bad_request()` are unchanged;
   - in `lib.rs`, use `pub use error::{HostError, LengthProblem};`;
   - in the unit tests at `:76-79` and `:101-105`, replace `flag: "--length".into()` and
     `flag: "--loops".into()` with `problem: LengthProblem::NoSections` and `problem:
     LengthProblem::LoopsWithoutLoop`. Their assertions and literals stay the same;
   - add `every_length_problem_round_trips`. For one example of each of the twelve variants,
     take the exact JSON the TS planner writes (field order as in step 3, for example
     `{"code":"unknownSection","name":"drop","sections":["intro","body","tail"]}`).
     `serde_json::from_str` must give the variant, and `to_string` must give the same text
     back. Add a `flag()` table over the same examples;
   - add `an_unknown_code_or_a_bad_param_is_rejected`, for `{"code":"nope"}`,
     `{"code":"lengthPastEnd","bars":-1,"clipBars":8}`, `{"code":"lengthPastEnd","bars":9}` and
     `{"code":"clipPastLimit","clipBars":1.5,"limit":100000,"hasLoop":true}`.
7. **`strudeland-host`: the slot.** In `host.rs:207-246`:
   - change the `Slot` doc to `{ ok: false, kind, error, length? }`;
   - change the field `flag: Option<String>` to `length: Option<LengthProblem>`;
   - change the match arm to `("meta", Some(problem)) => HostError::MetaFlag { problem,
     message: error }`. The other arms are unchanged;
   - a `length` that does not parse fails `serde_json::from_str`, which is already
     `Runtime("malformed result slot: …")`;
   - change the doc at `:95` to "… is [`HostError::MetaFlag`] with its [`LengthProblem`]".

   Tests at `:386-399`:
   - the meta case reads `"length":{"code":"noSections"}` into `MetaFlag { problem:
     LengthProblem::NoSections, message: "m" }`;
   - a `length` on kind `plan` is ignored;
   - `"length":null` is `Meta`;
   - a slot with the old `"flag":"--length"` and no `length` is `Meta`. This pins how a stale
     bundle would read;
   - `"length":{"code":"nope"}` is `Runtime` starting with `malformed result slot`.
8. **`crates/strudeland-host/tests/plan_render.rs`.** Rename
   `length_errors_are_meta_with_their_flag` (`:292`) to
   `length_errors_are_meta_with_their_problem`. Each row keeps its message literal and its flag
   column, and gains the expected `LengthProblem`. The test asserts `Err(HostError::MetaFlag {
   problem, message })`, and `problem.flag()` against the flag column. Add rows through real V8
   for A, E, F (both forms), I (`--section` and a preset), K and L. Use inline long songs like
   `crates/strudeland-render/tests/pipeline.rs:330-333`. Every code except C is then exercised
   through the bundle. Keep the bad-meta tail as it is.
9. **`strudeland-render`.**
   - `src/lib.rs:57-59`: add `LengthProblem` to the `strudeland_host` re-export.
   - `src/error.rs`:
     - the `Plan` doc (`:10-12`) says a `MetaFlag` prints just its message, the planner's
       advice;
     - the `exit_class` doc (`:62-63`) says "a planning failure of the length request
       ([`HostError::MetaFlag`])";
     - `plan_text` and `exit_class` are unchanged;
     - the test at `:128-133` builds `problem: LengthProblem::NoSections`, and its expected
       text `"pass --length N"` and `ExitClass::Usage` stay.
   - New `src/export/wording.rs`:

     ```rust
     //! The desktop dialog's wording of a length request the song cannot satisfy. The CLI
     //! prints the planner's own message instead (it names flags).

     /// The dialog text for `problem`: lowercase, flag-free, with the remedy the dialog offers.
     pub fn dialog_text(problem: &LengthProblem) -> String
     ```

     It is an exhaustive `match`, with no `_` arm, giving exactly the proposal's Dialog
     wordings table. `{list}` is `sections.join(", ")`, and every `{limit}` comes from the
     variant, never from `MAX_BARS`. In `export/mod.rs`, add `mod wording;` and `pub use
     wording::dialog_text;`, and add one line to the module doc: the dialog's wording of the
     planner's length problems.
   - Unit tests in `wording.rs`: `every_problem_has_its_dialog_text`, one row per variant with
     the exact expected string. Use `unknownSection` with `name: "drop"` and `sections:
     ["intro","body","tail"]`, and the other numbers from the step 6 examples. Assert that no
     text contains `--`, and that none starts with an uppercase letter except the
     `loopsInvalid` passthrough (use `message: "x"`).
   - `tests/pipeline.rs:316-319` and `:340-343`: match on `HostError::MetaFlag { problem, .. }
     if problem.flag() == "--length"` (and `"--loops"`), and also assert
     `matches!(problem, LengthProblem::NoSections)` (and `LoopsPastLimit { loops: 1000,
     clip_bars: 10000002, limit: 100000 }`). The `to_string` literals stay.
10. **CLI test.** In `crates/strudeland-cli/src/render.rs:1333-1336`, change the construction
    to `HostError::MetaFlag { problem: LengthProblem::NoSections, message: "this song has no
    sections; pass --length N".into() }`, importing `LengthProblem` from `strudeland_render`
    next to `HostError` (`:429`). The `o.code` and `o.stderr` assertions stay byte-identical.
    No other CLI file changes, and `SKILL.md` is untouched.
11. **App.** In `src-tauri/src/export_audio.rs` `export_message` (`:144-160`), add a first
    arm:

    ```rust
    export::Error::Render(RenderError::Plan(HostError::MetaFlag { problem, .. })) => {
        export::dialog_text(problem)
    }
    ```

    Keep `export::Error::Render(e) => e.to_string()` for everything else. Import `HostError` and
    `RenderError` from `strudeland_render`. The doc says that a length problem gets the
    dialog's wording, and everything else gets the module's.

    In `export_messages_keep_their_wording` (`:786`), add:
    - `Render(Plan(MetaFlag { problem: NoSections, message: "<D's CLI text>" }))`, giving
      `this song has no sections; choose a length in bars`;
    - `Render(Plan(MetaFlag { problem: PresetWithoutLoop, message: "<K's CLI text>" }))`,
      giving `loop export needs a meta.loop range or a section with loop: true`;
    - `Render(Plan(HostError::Eval("boom")))`, giving `eval: boom`, unchanged;
    - `Render(Plan(HostError::Meta("bad")))`, giving `meta: bad`, unchanged.

    Also assert that `run_error` of a `RunError` built from the D case carries the same
    message, with kind `other` and an empty `written`. No command signature,
    `CommandErrorKind` or `ExportError` field changes.
12. **Docs** (made stale by this stage):
    - `AGENTS.md:298`: "`HostError::MetaFlag` (a length request the song cannot satisfy: its
      typed `LengthProblem` and the planner's message)";
    - `AGENTS.md:308-310`, the `export/` item: add "the dialog wording of the planner's
      length problems (`wording.rs`, `dialog_text`)" after "the partial write";
    - `crates/strudeland-host/README.md:20`: "(`MetaFlag`: a `meta` failure of a render's
      length request, with its typed `LengthProblem` and the planner's message)";
    - `crates/strudeland-host/README.md:64-65`: "… is `HostError::MetaFlag`, whose
      `LengthProblem` names the CLI flag (`--length`, `--section`, `--loops`, `--preset`)".
13. **Generated files and obligations:** gate steps 1-5 and 16, in the gate's order.

## Tests

- New:
  - the problem and flag assertions in the TS tables;
  - TS throw tests for A, K, L and I through a preset;
  - the `lengthFlag` table;
  - `every_length_problem_round_trips`, `an_unknown_code_or_a_bad_param_is_rejected` and the
    `read_slot` cases;
  - the real-V8 rows in `plan_render.rs`;
  - `every_problem_has_its_dialog_text`;
  - the `export_message` and `run_error` cases.
- Changed constructions and matches only, with the same literals: `error.rs` (host and
  render), `pipeline.rs`, the CLI `render.rs` test, `host-entry.test.ts` and the `flag`
  assertions in `render-job-plan.test.ts`.
- Unchanged and passing: `crates/strudeland-host/tests/bin.rs:283`,
  `crates/strudeland-render/tests/bin.rs:272,286`, `crates/strudeland-cli/tests/e2e.rs:576-608`,
  `never_panics.rs`, and the bun staleness tests for `host.inputs.json` and the refs
  provenance (green only after gate steps 1 and 4).
- Targeted while working:
  - `bun test tests/render-job-plan.test.ts tests/host-entry.test.ts`;
  - `cargo test -p strudeland-host --lib`;
  - `cargo test -p strudeland-host --test plan_render`;
  - `cargo test -p strudeland-render --lib export::`;
  - `cargo test -p strudeland-render --test pipeline length`;
  - `cargo test -p strudeland-app export_messages`.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=001`, all steps. Also run:

```sh
git diff --name-only 0113154 -- crates/strudeland-cli > "$S/0144-001-cli-files.log"
git diff 0113154 -- crates/strudeland-cli/src/render.rs > "$S/0144-001-cli-diff.log"
```

- `cli-files.log` lists only `crates/strudeland-cli/src/render.rs`.
- `cli-diff.log` touches only the test module: the `MetaFlag` construction and the import.
- What stays unchanged, and the check for each:
  - CLI and harness text, exit codes and written files: the length A/B (35 cases);
  - host plans, job texts and host error text: the plan A/B and the three `host:parity`
    modes;
  - ref audio: refs A/B;
  - goldens and vectors: gate steps 6-7;
  - IPC: gate step 9.

## STOP

- A baseline case that misses its expected code after one song fix (step 1.10), or a baseline
  that is not deterministic (step 1.9).
- A site whose message would have to change to carry its problem. The message is the CLI
  contract.
- Anything in the shared STOP list.

## Commit message

```
feat(render): type the planner's length errors and word them in the export dialog

The planner's length-request errors now carry a typed code and its
parameters from the TS planner through the headless host. The CLI still
prints the planner's message unchanged; the desktop export dialog words
each code without command-line flags. The refs record the new planner
hash; no job, golden or CLI output changes.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
