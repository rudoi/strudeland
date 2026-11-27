# 0132 transcribe bar spans: step 0 re-verification

At 4af3182 (origin/main). The claim was reviewed against 63b945d. Since then nothing under
`crates/strudeland-transcribe/` has changed, and `crates/strudeland-cli/src/run.rs` changed only
outside the transcribe code (#31, #35, instruments). No new callers or copies exist (item 6). Every
cited line still holds, with the small offsets listed below.

## 1. The round trip, as it is

| Step | Where | Math |
|---|---|---|
| hint | `cli/src/run.rs:559-588` `build_hint` | bar 1 = `--bar1`, else sidecar `bar1Frame / rate` (`:573-576`). Tempo and meter come from the flag, else the sidecar. There is one constant `beats_per_bar` and no meter changes (the sidecar has only `tempo.meter`, `project/src/recordings.rs:148-157`). |
| bars to seconds | `transcribe/src/grid.rs:101-118` `GridHint::bar_span` (called from `run.rs:663-680` `resolve_span`) | `origin = bar1 + shift_ms/1000`, `bar = beats*60/bpm`, start `= origin + (A-1)*bar`, end `= origin + B*bar`. `None` (BarsNeedGrid, exit 4) without bpm, meter and bar 1, or with an unheard sidecar tempo |
| seconds to frames | `transcribe/src/load.rs:119,131` `span_frames` | `start_frame = round(start*rate)`. `Audio.start_seconds = start_frame/rate` (`:93`). This is where the loss happens. |
| into Options | `run.rs:637-646` | `span_start_seconds: audio.start_seconds`, `memo_seconds: Some(audio.memo_seconds)`, `last_bar: args.bars.map(|(_, to)| to)` (`:644`, not `:679`: the cited line moved) |
| grid | `grid.rs:197-300` `resolve_grid` | the same `origin + shift_ms/1000` (`:293`), so the shift agrees both ways |
| seconds back to bar | `transcribe/src/lib.rs:296-305` `span_bar` | `floor((span_start - origin + 1e-6) / bar) + 1`, saturating, with 0 meaning before bar 1. `BARLINE_SLACK_SECONDS = 1e-6` (`:298`) |
| bar window | `lib.rs:247-273` | `pickup = span_bar == 0 && notes in bar 0`. `first_bar = pickup ? 0 : max(span_bar, 1)`. `last_bar = opts.last_bar or the last bar with a note`. Over `MAX_NOTATION_BARS` it warns, otherwise it calls `render_notation` |

**The round trip loses a bar today.** Rounding the start to a frame moves it by up to
`0.5/rate`, which is 10.4 µs at 48 kHz. That is 10 times the 1e-6 slack. Whenever
`start*rate` has a fractional part between about 0.048 and 0.5, the start lands just before the
barline and `span_bar` returns A-1. This needs no exotic input: it happens at a plain tempo with
an integer-frame bar 1. I reproduced it with a scratch project in the session scratchpad (a
synthetic tone WAV, no user data) and the current debug binary:

| Memo (48 kHz) | Command | Result |
|---|---|---|
| 110 bpm, bar 1 at 0.5 s | `--bpm 110 --meter 4/4 --bar1 0.5 --grid 4 --bars 2-3` | `firstBar 1, lastBar 3`, `note("<~ [g4 a4 b4 c5] [c5 b4 a4 g4]>")`: a leading rest bar, so the loop is 3 cycles instead of 2. The start is 128727.27 frames and rounds 5.7 µs early |
| same | `--bars 3-4` | correct (3-4). The start is 233454.55 frames and rounds up |
| 100 bpm, bar 1 at 0.5 s | `--bars 2-3` | correct. The bar is a whole number of frames |
| 120 bpm, `--bar1 0.50001` | `--bars 2-3` | human header `memo: take  0:04.0  (bars 1-3)`, `note("<~ […] […]>")` |

A sidecar bar 1 (`bar1Frame/rate`) behaves the same way: it fails whenever `(A-1) * bar * rate`
is not a whole number and rounds down. That covers most tempos at 44.1 and 48 kHz. SKILL.md
(`:283-284`, "with `--bars 3-6` it is 3") and the README notation row (`:24`) promise A. The code
breaks that promise.

Other cases checked:
- **Meter changes**: none in transcription (one `beats_per_bar`), so there is nothing to lose.
- **Shift**: it is applied identically in `bar_span` and `resolve_grid`. `--shift -30 --bar1 0
  --bars 1-2` gives a negative start, which fails as `the span start must be a number of seconds, 0
  or more (got -0.03)`, exit 4 (`load.rs:110-114`).
- **Pickup**: `--bars` has A >= 1, so `span_bar == 0` comes only from the rounding bug at A = 1.
  A bar-0 note could then switch the notation to start at bar 0. I found no concrete case, but
  the code path is open.
- **Span at memo end**: the end frame is clamped (`load.rs:131`). `last_bar` is B whatever the
  clamp, so trailing `~` bars appear. `--bars 4-9` on an 11 s memo gives exit 0. A start past the
  end gives `the span start (19.700 s) is past the end of the memo (11.100 s)`, exit 4, worded in
  seconds for a bar request.
- **End rounding**: it moves the end by at most one sample, which does not affect the bar choice.
- **Tests miss it** because `lib.rs:597-623` and `:625-662` pass the unrounded `from` as
  `span_start_seconds`, so they never take the CLI's path. The CLI tests at `run.rs:2836-2890` use a
  16 kHz take at seed 1000's tempo, where the rounding happens to land on or after the barline.

## 2. Callers and public API

`transcribe()` callers: the CLI (`run.rs:648`), `tests/perf_budget.rs:50` (`Options::default()`),
`src/eval/cases.rs:263` (`Options::default()`, the whole memo), and the lib unit tests
(`lib.rs:375` onward). The app does not call it: only `strudeland-cli` depends on the crate
(`cli/Cargo.toml:18,27`). Nothing in `src-tauri` or the TS code uses it.

Every Options span field and who sets it:

| Field | Set by |
|---|---|
| `span_start_seconds` | CLI `:642`. Lib tests `:523,558,609,639,846,873,1048` |
| `memo_seconds` | CLI `:643`. Lib tests `:535,610,640,874,896` |
| `last_bar` | CLI `:644`. Lib tests `:641,652,659` |

Eval and perf use only the defaults.

The public surface that would change:
- `Options` gains a span request and loses `span_start_seconds`, `memo_seconds` and `last_bar`
  (or keeps them under the request).
- `GridHint::bar_span` (`pub`) is called by the CLI only, plus its tests at `grid.rs:1165-1218`.
- `load::{Span, Audio, load_wav_span}` (`pub mod load`): `load_wav_span` is called only at
  `run.rs:636` and in its own tests (`load.rs:307-690`). `resample_to_analysis` and
  `resampled_len` are used only inside the crate.
- `lib.rs:4` says "never paths", but `load_wav_span(&Path, Span)` (`load.rs:50`) takes one, and
  the README (`:15`) documents it as a stage.

## 3. Outputs that must stay identical

- **JSON** (`report.rs:11-41`): `memo`, `duration`, `grid{…suggestedDivision, suggestedShiftMs}`,
  `tuning`, `key`, `notation{note,n,scale,firstBar,lastBar}`, `notes`, `dropped`,
  `timingSpread`, `warnings`, `backing`, and `frames`/`onsets` with `--debug`. These are pinned
  by `run.rs:2447` `transcribe_json_shape_has_every_key`. `memo` and `backing` are set by the
  CLI (`run.rs:649-653`) by design: the crate never sees sidecars. Leave them there.
- **Human output**: `output.rs:281-370` (header `(bar N)`/`(bars A-B)` from the notation, the
  code lines, the table). The low-confidence ` ?` mark uses `consts::LOW_CONFIDENCE = 0.5`
  (`transcribe/src/consts.rs:192`), applied only in the human table at `output.rs:357`. JSON
  carries the raw `confidence`. The constant is already in the crate, so its location is fine.
- **Warnings and errors**: the CLI warnings come first (`run.rs:615-623`, recovered and no
  sidecar), followed by the crate's. The division and shift warnings are produced at
  `lib.rs:184-200` (`suggest.rs:145,156`). `MAX_NOTATION_BARS` says `the notes reach bar
  {last_bar}…` (`lib.rs:260-263`). Exit 4: BarsNeedGrid, BadSpan (seconds-worded), TooLong,
  GridDoesNotDivide, BadKey, BadOption. Exit 1: Wav, Bar1PastEnd (`error.rs:58-66`).
- **SKILL.md**: `:79`, `:218`, `:231-232` (`--bars` needs a known grid), `:247-251` (the
  division and shift go together), `:278-284` (firstBar is A for `--bars`, lastBar is B), `:289-291`,
  `:327`, `:334`. The fix makes the code match `:283-284`. No wording change is needed.
- **Eval**: `tests/eval.rs` tiers (`:70-150`, F1 floors, hummed pickup `firstBar == 0` at
  `:144-147`) and the README tables (`:62`, `:71`). Eval runs whole memos only (`span_start 0`,
  `last_bar None`), so these stay byte-identical only if the whole-memo window rule
  (`pickup ? 0 : max(span_bar, 1)`) is kept exactly. The pins are never loosened.

## 4. Tests today

Baseline at 4af3182 (logged once, all green):
- `cargo test -p strudeland-transcribe --features eval`: 208 lib tests, 6 eval tiers and
  `perf_budget` (1 ignored, by hand).
- `cargo test -p strudeland-cli transcribe`: 33 lib tests (all in `run.rs:2338-2890`, plus the
  `cli.rs:802-930` parser tests). `tests/e2e.rs` has no transcribe test.

What would move:
- Lib `a_span_later…` (`:597`) and `a_requested_last_bar…` (`:625`) would build a bars request
  instead of `span_start_seconds`/`last_bar`.
- `span_bar_is_the_bar_the_span_starts_in` (`:666-693`) stays for seconds spans, or moves to
  notation.
- `grid.rs` `bar_span` tests stay, because the CLI still needs seconds to cut the file.
- The CLI `transcribe_bars_*` tests (`run.rs:2602-2890`) stay as they are.
- New tests: a 48 kHz, 110 bpm `--bars 2-3` regression through `run_transcribe`, plus a lib
  test that passes the frame-rounded start.

## 5. Is it worth it

Yes. The friction is a real, reproducible bug in a documented contract, and the user hits it on
ordinary tempos. The fix is small. The CLI still has to turn bars into seconds to cut the WAV,
so `bar_span` stays. Only the way back goes away: `Options` carries `bars: Option<(u32, u32)>`
(or a `SpanRequest::{Whole, Seconds, Bars}`), and the notation window uses A directly.

The rest of the original direction is optional:
- **Folding the two suggestion calls into one `suggest::suggest(...) -> Suggestions`**: a private
  tidy-up with no visible change. It is fine to include. It is low risk, because the warnings
  and fields must come out byte-identical.
- **Moving the bar-window choice into `notation`** (a `window(...)` function): also private.
  It makes the bars rule testable on its own. Worth doing alongside the fix.
- **`load_wav_span` taking a reader or samples instead of a path**: little value. The Wav error
  message carries the path, so the CLI would have to rebuild it. Fix the crate doc instead
  (`lib.rs:4`: "the `load` module reads a WAV file; the stages take samples").
- **Moving `memo`/`backing`/`LOW_CONFIDENCE`**: no change. They are correct where they are.

A smaller alternative is widening the slack to `0.5/rate + ε`. It fixes the symptom with one
line, but keeps the round trip and couples `span_bar` to the load rounding. Not recommended.

## 6. New callers or copies since 63b945d

None. `git log 63b945d..HEAD` touches no transcribe crate file. The only `run.rs` commits
(#31, #35, eea00c9, 07b072c) do not touch `build_hint`, `run_transcribe` or `resolve_span`.
The SKILL.md edits are in the instruments section.

## Recommendation

**Proceed, re-scoped:**
1. `Options` carries the span request (`Whole | Seconds { start } | Bars { from, to }`), with
   the loaded start seconds and memo length kept for the grid and Bar1PastEnd.
2. A `notation` window function picks `first_bar`/`last_bar`: for `Bars`, exactly A..B. For
   whole and seconds spans, today's rule.
3. One `suggest` result.
4. Fix the crate doc.
5. Regression tests at 48 kHz.

Leave `load_wav_span`'s path, `memo`/`backing` and `LOW_CONFIDENCE` as they are. Run eval
before and after: the scores must be identical.

Visible-behaviour decisions:

| # | Decision | Recommended answer |
|---|---|---|
| 1 | `--bars A-B` notation always starts at A (today it is sometimes A-1 with a leading `~`) | Yes: it is the documented contract (SKILL.md:283-284). Call it a fix in the PR |
| 2 | `--bars 1-B` with notes quantized into bar 0: start at bar 0 (pickup) or at 1? | At 1: a bars request is exactly A..B. Bar-0 notes stay in `notes` (JSON) but not in the code. Today bar 0 is reachable only by the rounding bug |
| 3 | `BadSpan` texts for `--bars` (a negative start after `--shift`, a start past the memo end) are worded in seconds | Keep the texts unchanged in this task. Re-wording them in bars is a separate, visible change |
| 4 | `--start/--end` seconds spans keep `span_bar` with the 1e-6 slack | Keep: the user chose seconds, and the behaviour stays unchanged |
| 5 | The `MAX_NOTATION_BARS` warning for a requested B says "the notes reach bar B" | Keep it unchanged (B > 100000 is nonsense input) |
| 6 | JSON and human output fields | No new fields and no renames |
| 7 | Seen in passing: `no sidecar; the grid is inferred from the audio` is printed even when `--bpm --meter --bar1` give the whole grid (`run.rs:621-623`) | Out of scope. File it as its own small task |

## Decisions (2026-11-27)

- **User:** `--bars A-B` notation always covers exactly bars A to B, as `SKILL.md` promises. It ships as a bug fix, with regression tests at 48 kHz.
- **`--bars 1-B` (user):** if notes are quantized before bar 1, the notation still starts at bar 1. Those notes stay in the JSON `notes` list.
- **No output field changes (user):** no field is added or renamed. `--start/--end` keep today's rule, including the 1e-6 slack.
- **Kept as is (recommended):** the `BadSpan` texts stay worded in seconds, and the `MAX_NOTATION_BARS` warning text is unchanged.
- **Scope (recommended):**
  - In scope: `Options` carries the span request (`Whole | Seconds | Bars{from,to}`); a `notation` window function; the two suggestion calls fold into one private result; the crate doc's "never paths" line is fixed.
  - Out of scope: `load_wav_span`'s path argument, `memo`/`backing` and `LOW_CONFIDENCE` stay where they are.
- **Filed separately:** the "no sidecar; the grid is inferred" message prints even when flags give the whole grid.
