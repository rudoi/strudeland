# 001: record the baseline, then carry the span request into the transcriber

## Goal

First, before any source edit, record what the base code does:

- a grid of synthetic memos and requests, run through the base `strudeland` binary;
- the eval table and a per-case dump of every eval transcription;
- red runs of the two regression tests.

Then make the fix as a refactor. `Options` carries a `SpanRequest` and a `Placement` in place
of `span_start_seconds`, `memo_seconds` and `last_bar`. A private `notation::window` notates
exactly bars A..B for a `Bars` request and keeps today's rule otherwise. The CLI maps its
flags to the request once.

At the end, the grid runs again, and a classifier accepts only one kind of difference: a
`--bars` case whose notation started at A-1 now starts at A. Eval is byte-identical.

## Prerequisites

- The tree is code-identical to `4af3182`: `git diff --name-only 4af3182 HEAD` lists only
  `plans/`. If not, STOP: the baseline must come from the base code.
- `python3` is on `PATH` (`/opt/homebrew/bin/python3` when this plan was written). Run
  every scratch Python script with `python3 -I`.

## Files

- Edited:
  - `crates/strudeland-transcribe/src/lib.rs`, `src/notation.rs`, and `README.md` (the
    notation row, `:24`);
  - `crates/strudeland-cli/src/run.rs`;
  - `AGENTS.md`, the transcription layout bullet (`:332-339`).
- New, under `$S` and never committed:
  - `0132-gen/` (the scratch generator crate and the grid scripts);
  - `0132-grid/` (the synthetic project);
  - `0132-bin/` (the binaries);
  - `0132-out/` (the grid outputs);
  - `0132-frozen.sh`, `0132-literals.sh`, and the logs.
- Frozen: see [000-prompt.md](000-prompt.md#rules-for-every-stage). `suggest.rs` is frozen in
  this stage too.

## Steps

### Baseline (no source edit until step 6 is logged)

1. **The base binary.** `N=base`.
   - `L build cargo build --locked -p strudeland-cli`
   - `mkdir -p "$S/0132-bin" && cp target/debug/strudeland "$S/0132-bin/base"`
   - Log `git rev-parse HEAD` and `git diff --name-only 4af3182 HEAD` into
     `$S/0132-base-tree.log`.

   The debug profile builds `strudeland-transcribe` at `opt-level = 3` (`Cargo.toml:14-15`),
   so the debug binary is fast enough for the grid.

2. **The scratch generator and scripts** (`$S/0132-gen/`).
   - `Cargo.toml`: package `memogen`, `edition = "2024"`, an empty `[workspace]` table, and
     these dependencies:
     - `strudeland-transcribe = { path = "<repo>/crates/strudeland-transcribe", features = ["eval"] }`;
     - `strudeland-project = { path = "<repo>/crates/strudeland-project" }`;
     - `serde_json = { version = "=1.0.154", features = ["float_roundtrip"] }`.

     Two bins, `memos` and `evaldump`. Copy the repo's `Cargo.lock` in before the first build.
     Build with
     `CARGO_TARGET_DIR="$S/0132-gen/target" cargo build --release --offline --manifest-path "$S/0132-gen/Cargo.toml"`.
     If the offline resolve fails, STOP and report: do not let it fetch newer crates.
   - **`memos`** writes `$S/0132-grid/` from scratch (it removes the directory first):
     - `proj/` is a project, made with `Project::create(&dir, "grid")`, plus
       `proj/recordings/`;
     - each memo's WAV is written with `eval::synth::write_wav` and its sidecar with
       `Project::write_sidecar`, from a `Sidecar` built by `serde_json::from_value` on the same
       shape as `sidecar_value` (`run.rs:1987-2005`). Set `input.sampleRate` to the memo's
       rate, `tempo.bpm`, `tempo.meter` and `tempo.beatsPerBar` to the memo's, and
       `timing.bar1Frame` to the memo's bar-1 frame;
     - `memos.tsv` and `requests.tsv`, described below.

     **The melody.** A `Case` built field by field. Its `truth` holds, for each bar
     `k` in `1..=bars`, a note on every whole beat `i` in `0..floor(beats)`:
     - start `origin + (k-1)*bar + i*beat`, length `0.85*beat`;
     - MIDI from the C-major run `60 62 64 65 67 69 71 72`, indexed by `(k + i) % 8`.

     Two extras:
     - the last beat note of bar 3 lasts `1.5*beat`, so it crosses into bar 4 (a tie);
     - when the memo has `pickup`, a note at `origin - 0.5*beat`, `0.4*beat` long, MIDI 55
       (bar 0).

     `settings` is `SynthSettings { lead_in_seconds: origin, ..default() }`. The WAV is
     `case.render(rate)`.

     **The memos** (`id` in `memos.tsv` with rate, bpm, meter, beats per bar, origin, bar-1
     frame, sidecar kind, pickup, bars):

     | Group | Memos | Sidecar |
     |---|---|---|
     | core | rate `{16000, 44100, 48000}` × bpm `{100, 110, 97, 133.5, 120}` × origin `{0.5, 0.73}`, 4/4, 6 bars. `pickup` on the 0.73 ones | heard, `bar1Frame = origin * rate` |
     | meters | rate `{16000, 44100, 48000}` × meter `{3/4 (3), 6/8 (3), 7/8 (3.5), 12/8 (6)}`, 110 bpm, origin 0.5, 6 bars | heard |
     | kinds | 48 kHz, 110 bpm, 4/4, origin 0.5: `none` (no sidecar), `unheard` (`song` and `metronome` false), `recovered` (`recovered: true`), `badside` (the sidecar file is `{nope`), `fracframe` (`bar1Frame 24000.37`). Also `none` at 48 kHz for 100 and 120 bpm, and `none` at 44.1 kHz for 110 bpm | as named |
     | edges | `zero`: 48 kHz, 120 bpm, origin 0 (no lead-in). `zero110`: the same at 110 bpm. `short`: 48 kHz, 110 bpm, origin 0.5, 2 bars. `trunc`: the core 48 kHz 110 bpm 0.5 memo with the file cut to 60% of its data bytes, header untouched. `long`: 16 kHz, 601 s of silence (`write_wav` of zeros), 120 bpm, origin 0 | heard |

     **The requests** (`requests.tsv`: `id`, `memo`, `form`, `predict`, `from`, `to`, then the
     arguments). `form` is one of:
     - `json`: `--json transcribe MEMO --project PROJ ARGS`;
     - `human`: the same without `--json`;
     - `debug`: `--json`, with `--debug` appended.

     Every `human` request has a `json` twin with the same arguments. Here `O` is the origin,
     `B` the bar in seconds, `D` the WAV's length in seconds, and every number is printed with
     Rust's `{}` (shortest round-trip), so clap parses back the same `f64`. For every memo:

     | Requests | Forms |
     |---|---|
     | whole | json, human, debug |
     | `--start 1.3 --end 6.1`; `--start {O+2B} --end {O+4B}` (a barline) | json, human |
     | `--start 0 --end {D+1}`; `--start {O+2B-0.004} --end {O+3B}`; `--start 5 --end 4`; `--start {D+5} --end {D+6}` | json |
     | `--bars` `1-1`, `1-2`, `2-2`, `2-3`, `3-4`, `2-5`, `4-6`, `5-6`, `6-9`, `1-6` | json, human |
     | `--bars 2-3 --shift 40`; `--bars 2-3 --shift -40`; `--bars 1-2 --shift -{O*1000+30}` (a negative start); `--bars 30-31`; `--bars 2-100001` | json |
     | `--bars 2-3 --grid 8`; `--grid 8t`; `--grid 4` | json |
     | `--bars 2-4 --key C:major` | json, human |
     | `--bpm {bpm} --meter {meter} --bar1 {O} --bars 2-3`, and the same with `--grid 4` | json, human; json |

     Four named rows, on the `none` memos, re-run reverify §1's cases (ids `repro-a` to
     `repro-d`):

     | id | memo | arguments | expected |
     |---|---|---|---|
     | `repro-a` | `none`, 48 kHz, 110 bpm | `--bpm 110 --meter 4/4 --bar1 0.5 --grid 4 --bars 2-3` | hit |
     | `repro-b` | the same | `--bpm 110 --meter 4/4 --bar1 0.5 --grid 4 --bars 3-4` | no hit |
     | `repro-c` | `none`, 48 kHz, 100 bpm | `--bpm 100 --meter 4/4 --bar1 0.5 --bars 2-3` | no hit |
     | `repro-d` | `none`, 48 kHz, 120 bpm | `--bpm 120 --meter 4/4 --bar1 0.50001 --bars 2-3` | hit |

     **`predict`.** It is `hit`, `nohit` or `na`. Non-bars rows are `na`. So is a bars row
     whose grid is not known before the audio is read: no sidecar and no flags, `unheard`,
     `recovered` and `badside` without flags. A bars row whose start is negative or past the
     end is also `na`. Otherwise the generator repeats the code's arithmetic in `f64`:
     - `bar1` is the `--bar1` flag's value, else `bar1Frame / f64::from(rate)`;
     - `beats` is `strudeland_transcribe::parse_meter(meter)` for a `--meter` flag, else the
       sidecar's `beatsPerBar`;
     - `origin = bar1 + shift_ms / 1000.0`, `bar = beats * 60.0 / bpm`,
       `start = origin + (f64::from(from) - 1.0) * bar` (`grid.rs:101-118`);
     - `frame = (start * rate).round() as u64`, `s = frame as f64 / rate` (`load.rs:119`, `:93`);
     - `index = ((s - origin + 1e-6) / bar).floor() + 1.0` (`lib.rs:301-305`);
     - the row is `hit` when `index < f64::from(from)`.

     `repro-a` and `repro-d` must be `hit`, and `repro-b` and `repro-c` `nohit`. If they are
     not, STOP: the generator's arithmetic disagrees with reverify.
   - **`evaldump`**: for each `(tier, _)` in `eval::cases::TIERS`, it runs `run_tier(tier)`.
     For each `CaseResult` it prints one line:
     `{tier name}\t{seed}\t{serde_json::to_string(&r.transcription)}`.
     `run-evaldump.sh OUT` builds the crate (as above) and writes the bin's stdout to `OUT`.
   - **`run-grid.sh BIN OUT`**: for each row of `requests.tsv`, it runs `BIN` with the row's
     form and arguments. It writes `OUT/{id}.out` (stdout), `OUT/{id}.err` (stderr) and
     `OUT/{id}.code` (the exit code). It runs 8 rows in parallel (`xargs -P 8`), with
     `--project "$S/0132-grid/proj"`, so the running app is never asked.
   - **`check-base.py OUT`**: for every bars row with exit 0 and a non-null `notation` in the
     `json` form, the base output must agree with `predict`:
     - `nohit` gives `firstBar == from`;
     - `hit` gives `firstBar == from - 1` when `from >= 2`, and `firstBar` in `{0, 1}` when
       `from == 1`.

     It prints the counts of `hit`, `nohit` and `na` per rate. It exits 1 on any
     disagreement, or when a rate has no `hit` row with a notation.
   - **`classify.py BASE AFTER`**: the rules below. It prints one line per class with its
     count, then the id and both notations for every row outside `identical` and
     `fix-leading-rest`. It exits 0 only if every row is accepted.
     - **Every row:** `.err` and `.code` are byte-identical. A row with a nonzero code also
       has a byte-identical `.out`.
     - **`na` and `nohit` rows:** `.out` is byte-identical (class `identical`).
     - **`hit` rows, `json` and `debug`:** both `.out` files parse as JSON. Every key except
       `notation` is equal. Then:
       - The base notation is null, or its `firstBar` is not `from - 1`: the outputs must be
         byte-identical (class `hit-unchanged`; the `from == 1` rows without a pickup land
         here).
       - Otherwise the base `firstBar` is `from - 1`, and the row must differ.
         - If the after notation is not null, it must have `firstBar == from`,
           `lastBar == to`, the base `lastBar == to`, and the same `scale`. Its class is
             - `fix-leading-rest` when the base `note` is `<~ G…>` and the after `note` is
               the same groups without the leading `~`. For one remaining group, its
               brackets are stripped. The groups are split at bracket depth 0. The same rule
               holds for `n`;
             - `fix-ties` when the base `note` is a `[…]/k` sequence;
             - `fix-other` otherwise (bar A-1 had notes in the base output).
         - If the after notation is null, the class is `fix-none`. It is accepted only when
           no note in `notes` has `bar >= from` and a non-null `step`.
     - **`hit` rows, `human`:** accepted only if the `json` twin is a `fix-*` or
       `hit-unchanged`. For a `fix-*` twin, the files may differ only in the first line, which
       ends in `(bars {from-1}-{to})` before and in `(bars {from}-{to})` (`(bar {from})` when
       `from == to`) after, and in the `note("…")` and `n("…")` lines with the blank line
       before them. Everything else is byte-identical.
     - **Required:** `repro-a` and `repro-d` are `fix-*`, and every rate has at least one
       `fix-*` row.
   - **`grid.sh N`**:
     - `base`: runs `run-grid.sh "$S/0132-bin/base" "$S/0132-out/base"`, then
       `check-base.py`.
     - `001` or `002`: first `cargo build --locked -p strudeland-cli` and copies the binary to
       `$S/0132-bin/$N`. Then it runs `run-grid.sh` into `$S/0132-out/$N`. For `001` it
       runs `classify.py "$S/0132-out/base" "$S/0132-out/001"`. For `002` it runs
       `diff -r "$S/0132-out/001" "$S/0132-out/002"`.

     It exits with the last check's code.
   - **`$S/0132-frozen.sh N`**:
     - Every path in `git diff --name-only 4af3182` (working tree included) must be in the
       stage's allowed list:
       - `001`: `crates/strudeland-transcribe/src/lib.rs`, `…/src/notation.rs`,
         `…/README.md`, `crates/strudeland-cli/src/run.rs`, `AGENTS.md`, and
         `plans/0132-transcribe-bar-spans/*`;
       - `002`: the same plus `crates/strudeland-transcribe/src/suggest.rs`.
     - Every README hunk in `git diff -U0 4af3182 -- crates/strudeland-transcribe/README.md`
       starts at or before line 34, on both sides. The evaluation section (`:35` onward) is
       untouched.
     - For `002` only: `git diff --name-only C001` must list only `lib.rs`, `suggest.rs`,
       `README.md` and plan files. `C001` is
       `git log -1 --format=%H --grep '^fix(transcribe): notate exactly'`.
   - **`$S/0132-literals.sh N`** counts the lines that contain each fixed string in the
     non-test part of a file: the lines before the first `^mod tests`, via
     `awk '/^mod tests/{exit} {print}'`. Rows marked "whole" count the whole file. It prints
     the table and exits 1 on any count that differs for stage `N`. `base` runs in step 3,
     before any edit:

     | Literal | File | base | 001 | 002 |
     |---|---|---|---|---|
     | `"the notes reach bar {last_bar}; notation is limited to {MAX_NOTATION_BARS} bars"` | `transcribe/src/lib.rs` | 1 | 1 | 1 |
     | `bad("the span start", ` | lib.rs | 1 | 1 | 1 |
     | `bad("the memo length", ` | lib.rs | 1 | 1 | 1 |
     | `bad("--shift", ` | lib.rs | 1 | 1 | 1 |
     | `bad("--tuning", ` | lib.rs | 1 | 1 | 1 |
     | `"tuning is {c:.0} cents off the nearest semitone; the note names may be a semitone out. Pass --tuning"` | lib.rs | 1 | 1 | 1 |
     | `"the bars must be 1 <= A <= B (got {from}-{to})"` | lib.rs | 0 | 1 | 1 |
     | `const BARLINE_SLACK_SECONDS: f64 = 1e-6;` | lib.rs | 1 | 1 | 1 |
     | `span_start_seconds` | lib.rs (whole) | 12 | 0 | 0 |
     | `last_bar: Some(` | lib.rs (whole) | 2 | 0 | 0 |
     | `suggest::division_warning(` | lib.rs | 1 | 1 | 0 |
     | `suggest::suggest(` | lib.rs | 0 | 0 | 1 |
     | `"timing fits --grid {} better (spread {:.0} ms vs {:.0} ms)"` | `transcribe/src/suggest.rs` | 1 | 1 | 1 |
     | `"notes sit ~{:.0} ms {side} the --grid {} grid; try --grid {} --shift {shift:.0}"` | suggest.rs | 1 | 1 | 1 |
     | `"notes sit ~{:.0} ms {side} the grid; try --shift {shift:.0}"` | suggest.rs | 1 | 1 | 1 |
     | `"no sidecar; the grid is inferred from the audio"` | `cli/src/run.rs` | 1 | 1 | 1 |
     | `"the memo was recovered after a crash; its tempo is a placeholder"` | run.rs | 1 | 1 | 1 |
     | `TranscribeError::BarsNeedGrid` | run.rs | 1 | 1 | 1 |
     | `span_start_seconds` | run.rs (whole) | 1 | 0 | 0 |

     If `cargo fmt` would split a literal, do not let it: keep each literal on one line, as
     today.

3. **The grid on the base.** `N=base`:
   - `L memos "$S/0132-gen/target/release/memos" "$S/0132-grid"`;
   - `L grid "$S/0132-gen/grid.sh" base`;
   - `L literals "$S/0132-literals.sh" base`.

   Each must exit 0. Read `check-base.py`'s counts into the report. If `check-base.py` fails,
   STOP: the predictor and the base code disagree, so the classifier would judge the wrong
   rows.
4. **The eval baseline.**
   - `L eval sh -c 'cargo run --locked --release -q -p strudeland-transcribe --features eval --example eval > "$S/0132-base-eval.txt"'`
   - `L evaldump "$S/0132-gen/run-evaldump.sh" "$S/0132-base-evaldump.jsonl"`

   The eval table's tier rows must equal the README table (`README.md:64-69`). Diff them by
   eye and say so in the report. The dump has 25 lines (the 25 cases of `TIERS`).
5. **The regression tests, red on the base code.**
   - **CLI**, in `run.rs`'s test module after `transcribe_bars_with_shift_starts_at_its_bar`
     (`:2882`), in its final form:
     `transcribe_bars_start_at_their_bar_on_a_48k_take`.
     - Add `TruthNote` to the test module's `eval::synth` import (`:2341`). Build a `Case`
       field by field: 110 bpm, 4/4, 3 bars, origin 0.5, `seed: 11`,
       `SynthSettings { lead_in_seconds: 0.5, ..SynthSettings::default() }`. The truth is
       quarter notes `0.9 * beat` long: bar 1 `c4 d4 e4 f4`, bar 2 `g4 a4 b4 c5`, bar 3
       `c5 b4 a4 g4` (MIDI 60 62 64 65, 67 69 71 72, 72 71 69 67). Each `TruthNote` has its
       `bar`, `step: 4*i` and `steps: 4`.
     - Write it at 48 kHz as `recordings/take48.wav` into a `transcribe_project(1)` project,
       with the sidecar `sidecar_value(110.0, json!(0.5 * 48000.0))`.
     - Precondition: with
       `GridHint { bpm: Some(Hinted::flag(110.0)), beats_per_bar: Some(Hinted::flag(4.0)), origin_seconds: Some(Hinted::flag(0.5)), ..Default::default() }`,
       `bar_span(2, 3, 0.0)` gives a start with `start - (start * 48000.0).round() / 48000.0 > 1e-6`.
       So the cut rounds early by more than the slack.
     - For the sidecar run (`["take48", "--bars", "2-3"]`) and the flags run
       (`["take48", "--bpm", "110", "--meter", "4/4", "--bar1", "0.5", "--grid", "4", "--bars", "2-3"]`),
       the JSON must have exit 0 and `notation.firstBar == 2`, `lastBar == 3`, and a `note`
       that does not start with `<~`.
     - The flags run's `note` is exactly `<[g4 a4 b4 c5] [c5 b4 a4 g4]>`.
     - The human form of the flags run has a first line that ends in `(bars 2-3)`.
   - **Lib**, in `lib.rs`'s test module after `a_requested_last_bar_keeps_trailing_rest_bars`:
     `a_bars_request_starts_at_its_bar_when_the_cut_rounds_early`.
     - The same melody through `test_support::case_of`, with `bpm`, `beats_per_bar`,
       `origin_seconds` and `bars` set after it, rendered at 48 kHz, with `hint = sidecar(&case)`.
     - `let (from, to) = hint.bar_span(2, 3, 0.0).unwrap();`. Cut the samples at the rounded
       frames, as `span_frames` does: `a = (from * 48000.0).round()`,
       `b = min((to * 48000.0).round(), len)`. Set `start = a / 48000.0`, and assert
       `from - start > BARLINE_SLACK_SECONDS`.
     - The bars run (red form, with today's fields: `span_start_seconds: start`,
       `memo_seconds: Some(len / 48000.0)`, `last_bar: Some(3)`) must give
       `(first_bar, last_bar) == (2, 3)`.
     - The seconds run (the same placement, `last_bar: None`) keeps today's rule: its
       `first_bar` is 1, and its `note` is `<~ ` followed by the bars run's `note` without its
       leading `<`. This is the oracle that only the leading rest bar goes.

   Run `L red sh -c 'cargo test --locked -p strudeland-cli transcribe_bars_start_at_their_bar_on_a_48k_take; cargo test --locked -p strudeland-transcribe --features eval --lib a_bars_request_starts_at_its_bar'`
   with `N=001`. Both tests must **fail**, each on its first `firstBar`/`first_bar` assertion
   (2 expected, 1 found). The CLI failure shows the base `note`. If it is not
   `<~ [g4 a4 b4 c5] [c5 b4 a4 g4]>`, correct the flags run's expected `note` to the base
   one without its leading `~ `, and say so in the report. That is the one edit allowed after
   the red run. If either test passes, or fails on its precondition, STOP.
6. **Record the baseline's end.** `$S/0132-base-done.log` holds `date` and
   `git status --short`. It must list only the two test files from step 5. Every later source
   edit is after this time.

### The change

7. **`lib.rs` API.**
   - Add `SpanRequest` and `Placement` above `Options`, with the docs and derives given in the
     proposal ("The crate API"). `Placement::of(audio: &load::Audio) -> Placement` gives
     `Placement { start_seconds: audio.start_seconds, memo_seconds: Some(audio.memo_seconds) }`.
   - `Options`: remove `span_start_seconds`, `memo_seconds` and `last_bar`. Add `span` and
     `placement` in their place, after `shift_ms`, with the proposal's docs. Keep `Default`.
   - `transcribe`:
     - its doc says "`opts.placement` places it in the memo" in place of
       `opts.span_start_seconds`;
     - `let span_start = opts.placement.start_seconds;` keeps the local name, so the `bad`
       literals and every later use (`:173-176`, `:180`) are unchanged;
     - the `memo_seconds` check reads `opts.placement.memo_seconds`;
     - after the four finite checks, `if let SpanRequest::Bars { from, to } = opts.span && (from == 0 || from > to)`
       returns `BadOption(format!("the bars must be 1 <= A <= B (got {from}-{to})"))`;
     - `:119` reads `opts.placement.memo_seconds`.
   - `BARLINE_SLACK_SECONDS` doc: "Slack when a seconds span starts on a barline, in
     seconds, so a start computed as the barline's time, but a rounding short of it, starts
     in that bar." The value is unchanged.
8. **`notation.rs` `window`.** Add `WindowStart`, `Window` and `window` after
   `render_notation`, as the proposal gives them, with `use crate::consts::MAX_NOTATION_BARS`.
   The body is today's `lib.rs:255-273` with the two starts. Keep the order: compute `first`
   and `last`, then the `MAX_NOTATION_BARS` test, then the render test. `#[derive(Debug,
   Clone, Copy, PartialEq, Eq)]` on both enums, for the tests.
9. **`lib.rs` uses it.**
   - In the `Known` arm, `notation_grid = Some((q.steps_per_bar, start))`, where `start` is
     `WindowStart::Requested { from, to }` for `SpanRequest::Bars { from, to }`, and
     `WindowStart::SpanBar(span_bar(span_start, &grid))` otherwise.
   - At `:247`, after `notated` (`:249-254`, unchanged), match `notation::window(start, &notated)`:
     - `TooLong { last_bar }` pushes today's literal;
     - `Bars { first, last }` calls `render_notation(&notated, spb, first, last, out.key.as_ref())`;
     - `Empty` does nothing.
   - Remove the now-unused `note_bar`, `pickup`, `first_bar` and `last_bar` locals from
     `lib.rs`. Import `notation::{Window, WindowStart}`.
10. **The lib tests move to the new fields.** Only the `Options` construction changes:
    - `never_panics_on_odd_input` (`:446`):
      - the bad list's `span_start_seconds: f64::NAN` and `memo_seconds: Some(f64::NEG_INFINITY)`
        become `placement: Placement { start_seconds: f64::NAN, ..Placement::default() }` and
        `placement: Placement { memo_seconds: Some(f64::NEG_INFINITY), ..Placement::default() }`;
      - add `span: SpanRequest::Bars { from: 0, to: 1 }` and
        `span: SpanRequest::Bars { from: 3, to: 2 }` to the bad list;
      - the extreme `span_start_seconds: 1e12` becomes `placement.start_seconds`, and so does
        the `if` that reads it (`:564`).
    - `a_span_later_in_the_memo_starts_its_notation_at_its_bar` (`:598`):
      `span: SpanRequest::Bars { from: 5, to: 8 }` with
      `placement: Placement { start_seconds: from, memo_seconds: Some(…) }`.
    - `a_requested_last_bar_keeps_trailing_rest_bars` (`:626`):
      - `last_bar: Some(8)` becomes `span: SpanRequest::Bars { from: 5, to: 8 }`;
      - the "without it" run becomes `span: SpanRequest::Seconds { start: from, end: to }`;
      - the over-the-limit run becomes `span: SpanRequest::Bars { from: 5, to: MAX_NOTATION_BARS + 1 }`;
      - the placement is as in the test above.
    - `debug_fills_frames_and_onsets_only_when_set` (`:840`),
      `span_offset_is_added_to_note_times` (`:869`, including its `no_len` copy) and
      `a_pickup_bar_starts_the_notation` (`:1028`): `span_start_seconds`/`memo_seconds`
      become `placement` fields, and the request stays the default.
    - `span_bar_is_the_bar_the_span_starts_in` (`:667`) is unchanged.
    - The step 5 lib test switches its red-form fields to
      `span: SpanRequest::Bars { from: 2, to: 3 }` (bars run) and
      `span: SpanRequest::Seconds { start: from, end: to }` (seconds run), with
      `placement: Placement { start_seconds: start, memo_seconds: Some(…) }`. Its assertions
      are frozen.
11. **New lib and notation tests.**
    - `notation.rs` `window_rules`: a table over `WindowStart` × note-bar sets (built with the
      `note` helper, `notation.rs:286`):
      - `Requested { 2, 3 }` over bars `{1, 2, 3}` gives `Bars { 2, 3 }`;
      - `Requested { 1, 4 }` over `{0, 1, 2}` gives `Bars { 1, 4 }`, so the pickup does not
        move a bars request;
      - `Requested { 2, MAX_NOTATION_BARS + 1 }` over `{}` gives `TooLong { last_bar: MAX + 1 }`;
      - `Requested { 3, 4 }` over `{1, 2}` gives `Empty`;
      - `SpanBar(0)` over `{0, 2}` gives `Bars { 0, 2 }`; over `{1, 2}`, `Bars { 1, 2 }`;
      - `SpanBar(3)` over `{2, 5}` gives `Bars { 3, 5 }`; over `{}`, `Empty`;
      - `SpanBar(1)` over `{MAX + 1}` gives `TooLong { last_bar: MAX + 1 }`;
      - `SpanBar(u32::MAX)` over `{2}` gives `Empty`.
    - `notation.rs` `span_bar_window_is_todays_rule`: an oracle, a verbatim copy of today's
      `lib.rs:255-273` decision with `last_bar = None`, returning the same `Window`. For every
      `b` in `{0, 1, 2, 3, 7, MAX_NOTATION_BARS, MAX + 1, u32::MAX}` and every note-bar set
      among `{}`, `{0}`, `{0, 1}`, `{1}`, `{2, 4}`, `{0, MAX + 1}`, `{MAX}`, `{u32::MAX}`,
      `window(SpanBar(b), …)` equals the oracle.
    - `lib.rs` `a_bars_request_from_bar_1_leaves_the_pickup_in_the_notes`: the
      `a_pickup_bar_starts_the_notation` fixture (`audio(&melody(), voice())`,
      `grid_at(1.0)`). With `span: Bars { from: 1, to: 2 }`, notation is `(1, 2)`. The `notes`
      equal the whole run's `notes`, and they include a bar-0 note. The whole run's notation
      still starts at 0.
    - `lib.rs` `bad_bars_requests_are_bad_options`: `Bars { 0, 1 }` gives exactly
      `BadOption("the bars must be 1 <= A <= B (got 0-1)")` and `Bars { 3, 2 }` gives
      `… (got 3-2)`.
    - `lib.rs` `placement_of_a_loaded_span`: write a 48 kHz WAV of 1 s to a tempdir, load
      `Span::Seconds { start: 0.25, end: Some(0.75) }`, and assert
      `Placement::of(&audio) == Placement { start_seconds: 0.25, memo_seconds: Some(1.0) }`.

    Targeted: `cargo test --locked -p strudeland-transcribe --features eval --lib`.
12. **The CLI (`run.rs`).**
    - The imports add `SpanRequest` and `Placement`.
    - Add `span_request` and the new `resolve_span` signature as the proposal gives them. Its
      arms are today's, with the request in place of `args`, and the same `BarsNeedGrid`
      mapping. The doc comment keeps its wording, plus "for `request`".
    - In `run_transcribe`, at `:633`:
      - `let request = span_request(args);`
      - `let span = resolve_span(&request, &hint, args.shift)?;`
      - `Options { …, span: request, placement: Placement::of(&audio), debug: args.debug }`.
    - New `transcribe_span_request_follows_the_flags`: through `Cli::try_parse_from`,
      - no span flags give `Whole`;
      - `--start 1 --end 2` gives `Seconds { start: 1.0, end: 2.0 }`;
      - `--bars 2-5` gives `Bars { from: 2, to: 5 }`.

    Targeted: `cargo test --locked -p strudeland-cli transcribe`. The step 5 CLI test is now
    green, unchanged.
13. **Docs.**
    - The README notation row (`:24`): "`note("…")` names and `n("…").scale("…")` degrees,
      one cycle per bar: exactly bars A to B for a bars request (`SpanRequest::Bars`), else
      from the bar the span starts in (`firstBar`), or from bar 0 when it has a pickup; tied
      parts play as one held note".
    - `AGENTS.md:332-339`: after "every threshold in `consts.rs`;", add "`Options.span` is
      the request (`SpanRequest`: the whole memo, seconds, or bars A-B, notated as exactly A
      to B by `notation::window`) and `Options.placement` where the loaded samples sit in the
      memo (`Placement::of` a `load::Audio`);".
    - `SKILL.md` is unchanged: the fix makes `:283-284` true.

## Tests

- New, red on the base (step 5): `transcribe_bars_start_at_their_bar_on_a_48k_take` (CLI)
  and `a_bars_request_starts_at_its_bar_when_the_cut_rounds_early` (lib).
- New: `window_rules`, `span_bar_window_is_todays_rule` (notation),
  `a_bars_request_from_bar_1_leaves_the_pickup_in_the_notes`,
  `bad_bars_requests_are_bad_options`, `placement_of_a_loaded_span` (lib), and
  `transcribe_span_request_follows_the_flags` (CLI).
- Moved to the new fields with unchanged assertions: the six lib tests of step 10.
- Unchanged: every CLI transcribe test (`run.rs:2338-2900`), the `grid.rs` `bar_span` tests,
  `load.rs` tests, `tests/eval.rs` and `tests/perf_budget.rs`.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=001`: steps 1-11. Also:

- `0132-001-grid.log` shows the classifier's table. The report gives the counts of
  `identical`, `hit-unchanged` and each `fix-*` class, and lists every `fix-ties`,
  `fix-other`, `fix-none` and `hit-unchanged` id with both notations.
- `0132-base-done.log` predates every edit to `lib.rs`, `notation.rs` and `run.rs` outside
  the two step-5 tests. Compare its `date` with the files' mtimes, and say so in the report.
- `0132-001-red.log` shows both tests failing on base code, with the found values.

## STOP

- `check-base.py` fails, or a `repro-*` prediction is not as expected (step 3).
- A step-5 test passes on the base code, or fails on its precondition.
- The classifier rejects a row, or a required row is missing.
- The eval table or the dump differs by a byte.
- Anything in the shared STOP list.

## Commit message

```
fix(transcribe): notate exactly bars A to B for --bars

The CLI cut a --bars span in seconds, the loader rounded its start to
a frame, and the transcriber turned the rounded start back into a bar.
Rounding a start up to half a sample early is more than the barline
slack, so at ordinary tempos (110 bpm at 48 kHz, --bars 2-3) the
notation began a bar early with a rest bar.

Options now carries the span request (whole, seconds or bars) and the
placement of the loaded samples, in place of the span start, memo
length and last bar. A bars request is notated as exactly bars A to
B. Whole memos and seconds spans keep their rule, and every other
output is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
