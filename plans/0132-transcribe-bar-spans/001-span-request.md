# 001: record the baseline, then carry the requested bars into the transcriber

## Goal

First, before any source edit, record what the base code does:

- a grid of synthetic memos and requests, run through the base `strudeland` binary;
- the eval table and a per-case dump of every eval transcription;
- red runs of the two regression tests.

Then make the fix. `Options.bars: Option<(u32, u32)>` replaces `Options.last_bar`;
`span_start_seconds` and `memo_seconds` stay. A private `notation::window` notates exactly
bars A..B when `bars` is set and keeps today's rule otherwise. The CLI sets
`bars: args.bars`; `resolve_span` is unchanged.

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
     `float_roundtrip` matters because the grid's bar 1 is `bar1Frame / rate`, and a
     `bar1Frame` off by one ulp can move a predicted hit. The CLI reads sidecars through
     `strudeland-project`, and its `serde_json` has `float_roundtrip` by feature unification
     (`strudeland-project/Cargo.toml:17` and `strudeland-transcribe/Cargo.toml:16` both turn
     it on), so it parses `bar1Frame` back to the exact `f64` that was written. The generator
     declares the feature too, so its own `serde_json` (the `Sidecar` it builds and the eval
     dump) parses and prints floats exactly as well. The predictor reads each sidecar back
     with `Project::recordings()`, as the CLI does, so it uses the very `f64` the CLI uses.
   - **`memos`** writes `$S/0132-grid/` from scratch (it removes the directory first):
     - `proj/` is a project, made with `Project::create(&dir, "grid")`. It creates
       `proj/recordings/` itself (`project.rs:418`);
     - each memo's WAV is written with `eval::synth::write_wav`, then its sidecar with
       `Project::write_sidecar` (which validates it and needs the WAV first), from a `Sidecar`
       built by `serde_json::from_value` on the same shape as `sidecar_value`
       (`run.rs:1987-2005`). Set `input.sampleRate` to the memo's rate, `tempo.bpm`,
       `tempo.meter` and `tempo.beatsPerBar` to the memo's, and `timing.bar1Frame` to the
       memo's bar-1 frame. The `unheard` memo is the exception: `Sidecar::validate` refuses a
       `bar1Frame` and a `countIn: true` without a song or metronome
       (`strudeland-project/src/recordings.rs:251-252`, `:283-296`), so it has
       `backing.song`, `backing.metronome` and `backing.countIn` all false, and
       `timing.bar1Frame` and `timing.bar1HostNs` null. `badside`'s sidecar is not a
       `Sidecar`: write the bytes `{nope` to `proj/recordings/badside.json` with
       `std::fs::write`, after its WAV;
     - memo ids are valid stems (`is_valid_stem`, `strudeland-project/src/project.rs:923-931`:
       ASCII letters, digits, space, `_` or `-`, 1 to 64 bytes, no dots or slashes). Write a
       decimal point as `p` and a meter's slash as `-`: `core-48000-133p5-0p73`,
       `meter-44100-7-8`, `kind-fracframe`;
     - `memos.tsv` and `requests.tsv`, described below.

     **The melody.** A `Case` built field by field. Its `truth` holds, for each bar
     `k` in `1..=bars`, a note on every whole beat `i` in `0..floor(beats)`:
     - start `origin + (k-1)*bar + i*beat`, length `0.85*beat`;
     - MIDI from the C-major run `60 62 64 65 67 69 71 72`, indexed by `(k + i) % 8`.

     Two extras:
     - the last beat note of bar 3 lasts `1.5*beat`, so it crosses into bar 4 (a tie). The
       render plays one note at a time, so bar 4's first note then sounds from `0.5*beat`
       into the bar;
     - when the memo has `pickup`, a note at `origin - 0.5*beat`, `0.4*beat` long, MIDI 55
       (bar 0). Insert it first in `truth`: `Case::render` walks `truth` in order and assumes
       sorted starts (`eval/synth.rs:421-427`).

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
     `B` the bar in seconds, `D` the WAV's length in seconds as its header states it (the
     `load::Audio::memo_seconds` the CLI's past-the-end checks use; for `trunc` this is the
     uncut length, longer than its data), and every number is printed with Rust's `{}`
     (shortest round-trip), so clap parses back the same `f64`. For every memo, except where
     `long` is excluded:

     | Requests | Forms |
     |---|---|
     | whole | json, human, debug |
     | `--start 1.3 --end 6.1`; `--start {O+2B} --end {O+4B}` (a barline) | json, human |
     | `--start 0 --end {D+1}` (not on `long`); `--start {O+2B-0.004} --end {O+3B}`; `--start 5 --end 4`; `--start {D+5} --end {D+6}` | json |
     | `--bars` `1-1`, `1-2`, `2-2`, `2-3`, `3-4`, `2-5`, `4-6`, `5-6`, `6-9`, `1-6` | json, human |
     | `--bars 2-3 --shift 40`; `--bars 2-3 --shift -40`; `--bars 1-2 --shift -{O*1000+30}` (a negative start); `--bars 30-31`; `--bars 2-100001` (not on `long`) | json |
     | `--bars 2-3 --grid 8`; `--grid 8t`; `--grid 4` | json |
     | `--bars 2-4 --key C:major` | json, human |
     | `--bpm {bpm} --meter {meter} --bar1 {O} --bars 2-3`, and the same with `--grid 4` | json, human; json |

     `long` is left out of the two rows that would analyse almost all of its 601 s
     (`--start 0 --end {D+1}` and `--bars 2-100001`): each would cost minutes in the debug
     binary and covers nothing the other memos do not. `long`'s whole request still checks the
     over-10-minute refusal.

     Four named rows, on the `none` memos, re-run reverify §1's cases (ids `repro-a` to
     `repro-d`):

     | id | memo | arguments | expected |
     |---|---|---|---|
     | `repro-a` | `none`, 48 kHz, 110 bpm | `--bpm 110 --meter 4/4 --bar1 0.5 --grid 4 --bars 2-3` | hit |
     | `repro-b` | the same | `--bpm 110 --meter 4/4 --bar1 0.5 --grid 4 --bars 3-4` | no hit |
     | `repro-c` | `none`, 48 kHz, 100 bpm | `--bpm 100 --meter 4/4 --bar1 0.5 --bars 2-3` | no hit |
     | `repro-d` | `none`, 48 kHz, 120 bpm | `--bpm 120 --meter 4/4 --bar1 0.50001 --bars 2-3` | hit |

     **`predict`.** It is `hit`, `nohit` or `na`. Non-bars rows are `na`. For a bars row the
     generator uses the crate's own code wherever it is public, and copies only `span_bar`'s
     formula:
     - It builds the `GridHint` as `build_hint` does (`run.rs:559-588`). The trusted sidecar
       is the memo's sidecar unless it is missing, unreadable (`badside`) or `recovered`.
       `bpm` is `Hinted::flag(--bpm)`, else `Hinted::sidecar(tempo.bpm)`. `beats_per_bar` is
       `Hinted::flag(parse_meter(--meter))`, else `Hinted::sidecar(tempo.beatsPerBar)`.
       `origin_seconds` is `Hinted::flag(--bar1)`, else
       `Hinted::sidecar(bar1Frame / f64::from(rate))`. `meter` is `--meter`, else
       `tempo.meter`. `tempo_unheard` is the trusted sidecar's `!song && !metronome`.
     - `hint.bar_span(from, to, shift_ms)` gives `(start, end)`. `None` (no grid before the
       audio is read: no sidecar and no flags, `unheard`, `recovered` or `badside` without
       flags) makes the row `na`.
     - `load::load_wav_span(path, Span::Seconds { start, end: Some(end) })`. An error (a
       negative start, a start past the end) makes the row `na`. Otherwise
       `s = audio.start_seconds`.
     - With `origin = hint origin + shift_ms / 1000.0` and `bar = beats * 60.0 / bpm`, from
       the same hint values: `index = ((s - origin + 1e-6) / bar).floor() + 1.0`
       (`lib.rs:301-305`, copied).
     - The row is `hit` when `index < f64::from(from)`, else `nohit`.

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
     - **`na` and `nohit` rows, every form:** `.out` is byte-identical (class `identical`).
     - **`hit` rows, `json` and `debug`:** both `.out` files parse as JSON. Every key except
       `notation` is equal. Then:
       - The base notation is null, or its `firstBar` is not `from - 1`: the outputs must be
         byte-identical (class `hit-unchanged`; the `from == 1` rows without a pickup land
         here).
       - Otherwise the base `firstBar` is `from - 1`, and the row must differ.
         - If the after notation is not null, it must have `firstBar == from`,
           `lastBar == to`, the base `lastBar == to`, and the same `scale`. With
           `k = to - from + 2` (the base's bar count), its class is
             - `fix-leading-rest` when the base `note` is `<~ G…>` and the after `note` is
               the same groups without the leading `~`. For one remaining group, its
               brackets are stripped. The groups are split at bracket depth 0. The same rule
               holds for `n` when it is not null;
             - `fix-ties` when the base `note` is `[X]/k` and the after `note` is `[Y]/(k-1)`
               (a slow sequence over one bar fewer), and the same holds for `n` when it is
               not null;
             - `fix-other` otherwise (bar A-1 had notes in the base output).
         - If the after notation is null, the class is `fix-none`. It is accepted only when
           no note in `notes` has `bar >= from` and a non-null `step`.
     - **`hit` rows, `human`:** classified by their `json` twin.
       - `hit-unchanged` twin: `.out` is byte-identical.
       - `fix-leading-rest`, `fix-ties` or `fix-other` twin: the files may differ only in the
         first line, which ends in `  (bars {from-1}-{to})` before and in `  (bars {from}-{to})`
         (`  (bar {from})` when `from == to`) after, and in the `note("…")` and `n("…")`
         lines. Everything else, the blank line before `note(` included, is byte-identical.
       - `fix-none` twin: the after first line is the base first line without its
         `  (bars {from-1}-{to})` suffix, and the after file is the base file without its
         blank line, `note("…")` line and `n("…")` line (when present). Everything else is
         byte-identical.
       - Any other twin class rejects the row.
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
     - Every path in `git diff --name-only 4af3182` (working tree included) and in
       `git ls-files --others --exclude-standard` (new untracked files) must be in the
       stage's allowed list:
       - `001`: `crates/strudeland-transcribe/src/lib.rs`, `…/src/notation.rs`,
         `…/README.md`, `crates/strudeland-cli/src/run.rs`, `AGENTS.md`, and
         `plans/0132-transcribe-bar-spans/*`;
       - `002`: the same plus `crates/strudeland-transcribe/src/suggest.rs`.
     - Every README hunk in `git diff -U0 4af3182 -- crates/strudeland-transcribe/README.md`
       lies within lines 1-34 on the old side: for a header `@@ -a,b +c,d @@` (`b` is 1 when
       omitted), its end `a + b - 1` (or `a` when `b` is 0) is at most 34. And the text from
       the line `## Evaluation` to the end of the file is byte-identical to `4af3182`'s
       (`sed -n '/^## Evaluation/,$p'` of both, then `cmp`), so the evaluation section
       (`:35` onward) is untouched whatever the intro's line count.
     - For `002` only: `git diff --name-only C001` must list only `lib.rs`, `suggest.rs`,
       `README.md`, `AGENTS.md` and plan files. `C001` is
       `git log -1 --format=%H --grep '^fix(transcribe): notate exactly'`. The AGENTS.md
       change is one line: `git diff -U0 C001 -- AGENTS.md` has one hunk, one `-` line and
       one `+` line, and the `+` line is the `-` line with `audio I/O` replaced by
       `audio devices`.
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
     | `pub span_start_seconds: f64,` | lib.rs | 1 | 1 | 1 |
     | `pub memo_seconds: Option<f64>,` | lib.rs | 1 | 1 | 1 |
     | `pub bars: Option<(u32, u32)>,` | lib.rs | 0 | 1 | 1 |
     | `span_start_seconds` | lib.rs (whole) | 12 | 13 | 13 |
     | `last_bar: Some(` | lib.rs (whole) | 2 | 0 | 0 |
     | `opts.last_bar` | lib.rs (whole) | 1 | 0 | 0 |
     | `suggest::division_warning(` | lib.rs | 1 | 1 | 0 |
     | `suggest::suggest(` | lib.rs | 0 | 0 | 1 |
     | `"timing fits --grid {} better (spread {:.0} ms vs {:.0} ms)"` | `transcribe/src/suggest.rs` | 1 | 1 | 1 |
     | `"notes sit ~{:.0} ms {side} the --grid {} grid; try --grid {} --shift {shift:.0}"` | suggest.rs | 1 | 1 | 1 |
     | `"notes sit ~{:.0} ms {side} the grid; try --shift {shift:.0}"` | suggest.rs | 1 | 1 | 1 |
     | `"no sidecar; the grid is inferred from the audio"` | `cli/src/run.rs` | 1 | 1 | 1 |
     | `"the memo was recovered after a crash; its tempo is a placeholder"` | run.rs | 1 | 1 | 1 |
     | `TranscribeError::BarsNeedGrid` | run.rs | 1 | 1 | 1 |
     | `span_start_seconds: audio.start_seconds,` | run.rs | 1 | 1 | 1 |
     | `last_bar: args.bars.map(` | run.rs | 1 | 0 | 0 |
     | `bars: args.bars,` | run.rs | 0 | 1 | 1 |

     The base count of `span_start_seconds` in the whole of `lib.rs` is 12 (`:61`, `:63`,
     `:78`, `:93`, `:523`, `:558`, `:564`, `:609`, `:639`, `:846`, `:873`, `:1048`). 001 adds
     exactly one line with it, the step 5 lib test's `Options` literal; no other new code or
     doc names it.

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
     - The flags run first
       (`["take48", "--bpm", "110", "--meter", "4/4", "--bar1", "0.5", "--grid", "4", "--bars", "2-3"]`),
       then the sidecar run (`["take48", "--bars", "2-3"]`). For each, in this order:
       - exit 0, with stderr as the message;
       - `assert_eq!(v["notation"]["firstBar"], 2, "{}", v["notation"])`: the message is the
         whole `notation` value, so a red run logs the base `note` and `n`;
       - `lastBar == 3`, with the same message;
       - a `note` that does not start with `<~`.
     - The flags run's `note` is exactly `<[g4 a4 b4 c5] [c5 b4 a4 g4]>`, asserted after its
       `firstBar` and `lastBar`.
     - The human form of the flags run has a first line that ends in `(bars 2-3)`.
   - **Lib**, in `lib.rs`'s test module after `a_requested_last_bar_keeps_trailing_rest_bars`:
     `a_bars_request_starts_at_its_bar_when_the_cut_rounds_early`.
     - The same melody through `test_support::case_of`, with `bpm`, `beats_per_bar`,
       `origin_seconds` and `bars` set after it, rendered at 48 kHz, with `hint = sidecar(&case)`.
     - `let (from, to) = hint.bar_span(2, 3, 0.0).unwrap();`. Cut the samples at the rounded
       frames, as `span_frames` does: `a = (from * 48000.0).round()`,
       `b = min((to * 48000.0).round(), len)`. Set `start = a / 48000.0`, and assert
       `from - start > BARLINE_SLACK_SECONDS`.
     - One `Options` literal, `seconds`, with `span_start_seconds: start` and
       `memo_seconds: Some(len / 48000.0)`, and `last_bar` left at its default `None`. It is the only
       new line in the file that names `span_start_seconds` (the literal-count table relies on
       it).
     - **The seconds run first**, with `seconds`. It keeps today's rule, and its assertions
       are proved on the base code before the red assertion is reached: `(first_bar,
       last_bar) == (1, 3)`, and its `note` starts with `<~ `.
     - **Then the bars run**, with `Options { last_bar: Some(3), ..seconds.clone() }` (red
       form; `bars: Some((2, 3))` after step 10, the only edit). First
       `assert_eq!(bars.notes, seconds.notes)`: the same samples and placement give the same
       notes, on the base code too. Then the red assertion,
       `(first_bar, last_bar) == (2, 3)`, with the bars notation in its message.
     - Last, the oracle that only the leading rest bar goes: the seconds run's `note` equals
       `format!("<~ {}", &bars_note[1..])`.

   Run `L red sh -c 'cargo test --locked -p strudeland-cli transcribe_bars_start_at_their_bar_on_a_48k_take; cargo test --locked -p strudeland-transcribe --features eval --lib a_bars_request_starts_at_its_bar'`
   with `N=001`. Both tests must **fail**, each on its first `firstBar`/`first_bar` assertion
   (2 expected, 1 found): the CLI test on the flags run, the lib test on the bars run, after
   its seconds-run assertions passed. The CLI failure message is the flags run's whole base
   `notation`, so the log shows the base `note`. If it is not
   `<~ [g4 a4 b4 c5] [c5 b4 a4 g4]>`, correct the flags run's expected `note` to the base
   one without its leading `~ `, and say so in the report. That is the one edit allowed after
   the red run. If either test passes, or fails on its precondition, STOP.
6. **Record the baseline's end.** `$S/0132-base-done.log` holds `date` and
   `git status --short`. It must list only the two test files from step 5. Every later source
   edit is after this time.

### The change

7. **`lib.rs` API.**
   - `Options`: replace `last_bar` and its doc with `bars: Option<(u32, u32)>` and the doc the
     proposal gives ("The crate API"), in the same place. `span_start_seconds`,
     `memo_seconds` and their docs are unchanged. Keep `Default` (`bars: None`).
   - `transcribe`: after the four finite checks (unchanged),
     `if let Some((from, to)) = opts.bars && (from == 0 || from > to)` returns
     `BadOption(format!("the bars must be 1 <= A <= B (got {from}-{to})"))`. The rest of the
     validation, the `span_start` local and every use of `memo_seconds` are unchanged.
   - `BARLINE_SLACK_SECONDS` doc: "Slack when a whole or seconds span starts on a barline,
     in seconds, so a start computed as the barline's time, but a rounding short of it,
     starts in that bar." The value is unchanged.
8. **`notation.rs` `window`.** Add `WindowStart`, `Window` and `window` after
   `render_notation`, as the proposal gives them, with `use crate::consts::MAX_NOTATION_BARS`.
   The body is today's `lib.rs:255-273` with the two starts. Keep the order: compute `first`
   and `last`, then the `MAX_NOTATION_BARS` test, then the render test. `#[derive(Debug,
   Clone, Copy, PartialEq, Eq)]` on both enums, for the tests.
9. **`lib.rs` uses it.**
   - In the `Known` arm, `notation_grid = Some((q.steps_per_bar, start))`, where `start` is
     `WindowStart::Requested { from, to }` for `opts.bars == Some((from, to))`, and
     `WindowStart::SpanBar(span_bar(span_start, &grid))` for `None`.
   - At `:247`, after `notated` (`:249-254`, unchanged), match `notation::window(start, &notated)`:
     - `TooLong { last_bar }` pushes today's literal;
     - `Bars { first, last }` calls `render_notation(&notated, spb, first, last, out.key.as_ref())`;
     - `Empty` does nothing.
   - Remove the now-unused `note_bar`, `pickup`, `first_bar` and `last_bar` locals from
     `lib.rs`. Import `notation::{Window, WindowStart}`.
10. **The one existing lib test that moves.** In
    `a_requested_last_bar_keeps_trailing_rest_bars` (`:626`), only the three `last_bar`
    settings change, and nothing else in it:
    - `last_bar: Some(8)` (`:641`) becomes `bars: Some((5, 8))`;
    - `last_bar: None` (`:652`, the "without it" run) becomes `bars: None`;
    - `last_bar: Some(MAX_NOTATION_BARS + 1)` (`:659`) becomes
      `bars: Some((5, MAX_NOTATION_BARS + 1))`.

    Its `span_start_seconds: from` (the unrounded start) stays: with `bars` set the window no
    longer reads it, and with `None` it is today's seconds rule. Every other lib test is
    unchanged, `never_panics_on_odd_input`, `debug_fills_frames_and_onsets_only_when_set`,
    `span_offset_is_added_to_note_times`, `a_pickup_bar_starts_the_notation` and
    `span_bar_is_the_bar_the_span_starts_in` included. So is
    `a_span_later_in_the_memo_starts_its_notation_at_its_bar` (`:598`): it sets no
    `last_bar`, so it stays on the seconds rule (`bars` `None`) and keeps checking that a
    span started on a barline (unrounded) starts in that bar. Do not give it `bars`.

    The step 5 lib test's `last_bar: Some(3)` becomes `bars: Some((2, 3))`. That is its only
    edit, and its assertions are frozen.
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
      `grid_at(1.0)`). With `bars: Some((1, 2))`, notation is `(1, 2)`. The `notes` equal the
      whole run's `notes`, and they include a bar-0 note. The whole run's notation still
      starts at 0.
    - `lib.rs` `bad_bars_requests_are_bad_options`: `bars: Some((0, 1))` gives exactly
      `BadOption("the bars must be 1 <= A <= B (got 0-1)")` and `bars: Some((3, 2))` gives
      `… (got 3-2)`.

    Targeted: `cargo test --locked -p strudeland-transcribe --features eval --lib`.
12. **The CLI (`run.rs`).** In `run_transcribe`'s `Options` literal, `last_bar:
    args.bars.map(|(_, to)| to)` (`:644`) becomes `bars: args.bars`. Nothing else in the
    non-test code changes: `resolve_span` keeps its signature `(args, hint)` and its body,
    and `span_start_seconds: audio.start_seconds` and `memo_seconds: Some(audio.memo_seconds)`
    stay.

    Targeted: `cargo test --locked -p strudeland-cli transcribe`. The step 5 CLI test is now
    green, unchanged.
13. **Docs.**
    - The README notation row (`:24`): "`note("…")` names and `n("…").scale("…")` degrees,
      one cycle per bar: exactly bars A to B for a bars request (`Options.bars`), else from
      the bar the span starts in (`firstBar`), or from bar 0 when it has a pickup; tied parts
      play as one held note".
    - `AGENTS.md:332-339`: after "every threshold in `consts.rs`;", add "`Options.bars`
      (`--bars A-B`) is notated as exactly bars A to B by `notation::window`, other spans
      from the bar their samples start in;".
    - `SKILL.md` is unchanged: the fix makes `:283-284` true.

## Tests

- New, red on the base (step 5): `transcribe_bars_start_at_their_bar_on_a_48k_take` (CLI)
  and `a_bars_request_starts_at_its_bar_when_the_cut_rounds_early` (lib).
- New: `window_rules`, `span_bar_window_is_todays_rule` (notation),
  `a_bars_request_from_bar_1_leaves_the_pickup_in_the_notes` and
  `bad_bars_requests_are_bad_options` (lib).
- Moved: only `a_requested_last_bar_keeps_trailing_rest_bars`, its three `last_bar`
  settings and nothing else (step 10).
- Unchanged: every other lib test (`a_span_later_in_the_memo_starts_its_notation_at_its_bar`
  and `never_panics_on_odd_input` included), every CLI transcribe test
  (`run.rs:2338-2900`), the `grid.rs` `bar_span` tests, `load.rs` tests, `tests/eval.rs` and
  `tests/perf_budget.rs`.

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

Options now carries the requested bars in place of the last bar, so
the notation's bars come from the request and not from the rounded
start. A bars request is notated as exactly bars A to B. Whole memos
and seconds spans keep their rule, and every other output is
unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
