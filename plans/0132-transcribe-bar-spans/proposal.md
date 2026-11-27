# 0132: bar spans go through the transcriber as bars

## Goal

`strudeland transcribe --bars A-B` asks for bars A to B. Today the CLI turns that request into
seconds, the WAV loader rounds the start to a frame, and the transcriber turns the rounded
seconds back into a bar. The rounding can put the start up to `0.5/rate` before the barline,
10 times the `1e-6` slack, so the notation starts at bar A-1 with a leading `~`. This happens
at ordinary tempos (110 bpm at 48 kHz, `--bars 2-3`; [reverify.md](reverify.md) §1).

After this change, the bars reach the transcriber as bars:

- `Options.bars: Option<(u32, u32)>` replaces `Options.last_bar: Option<u32>`. It is
  `--bars A-B` as `(A, B)`, the same type as `TranscribeArgs::bars`. `span_start_seconds` and
  `memo_seconds` stay as they are.
- A new private `notation::window` picks the notation's bars. With `bars` set it gives exactly
  A..B. With `bars` `None` it keeps today's rule, the 1e-6 slack included, byte for byte.
- The CLI still cuts the file in seconds (`resolve_span` is unchanged) and sets
  `bars: args.bars`. `load_wav_span` still takes a path.
- The two suggestion calls (`lib.rs:184-200`) become one private `suggest::suggest` that
  returns a `Suggestions`.
- The crate doc stops saying "never paths" (`lib.rs:4`, README `:5-6`).

The bug is fixed because, for a bars request, the window no longer comes from the rounded
start: `span_start_seconds` still places notes and onsets in the memo, but it no longer picks
the first bar.

**One visible change, the fix.** For `--bars A-B`, the notation always starts at A. Every other
output stays byte-identical for every input: human and `--json` output, warnings, and the exit
1 and 4 texts. The eval scores and pins stay byte-identical too. A baseline over a grid of
synthetic memos, recorded before any code change, proves it. Every difference must be a
`--bars` case where today's notation starts at A-1, and must now start at A.

## Background

[reverify.md](reverify.md) holds the evidence at `4af3182`, and its **Decisions** section is
binding. In short:

- The round trip: `build_hint` (`cli/src/run.rs:557-588`), `GridHint::bar_span`
  (`transcribe/src/grid.rs:101-118`, called from `resolve_span`, `run.rs:661-680`),
  `span_frames` rounding (`load.rs:119`, `:131`; `Audio.start_seconds`, `:93`), into
  `Options` (`run.rs:637-646`), then `span_bar` (`lib.rs:295-305`) and the window
  (`lib.rs:247-273`).
- `transcribe()` has three callers outside its unit tests: the CLI (`run.rs:648`), and
  `tests/perf_budget.rs:50` and `src/eval/cases.rs:263`, which both pass
  `Options::default()`. The app does not use the crate.
- Eval and perf run whole memos with the default options. So eval stays identical exactly
  when the whole-memo rule (`pickup ? 0 : max(span_bar, 1)`) is kept.
- The tests miss the bug. The lib tests pass the unrounded start (`lib.rs:598-664`). The CLI
  tests use a 16 kHz take at seed 1000's tempo, where the rounding happens to land on or after
  the barline (`run.rs:2838-2892`).

The user's decisions (reverify, Decisions):

1. `--bars A-B` notation always covers exactly bars A to B. It ships as a fix, with 48 kHz
   regression tests.
2. With `--bars 1-B`, notes quantized before bar 1 do not move the start: the notation starts
   at 1, and those notes stay in the JSON `notes`.
3. No output field is added or renamed. `--start/--end` keep today's rule, the 1e-6 slack
   included.
4. The `BadSpan` texts stay worded in seconds, also for a `--bars` request. The
   `MAX_NOTATION_BARS` warning text does not change.
5. `load_wav_span`'s path argument, `memo`/`backing`, and `LOW_CONFIDENCE` stay where they are.
6. The smaller `Options` form: only `last_bar` becomes `bars`. No `SpanRequest`, no
   `Placement` (reverify, Decisions, refine review).

## Scope

In:

1. `crates/strudeland-transcribe/src/lib.rs`:
   - `Options.bars` replaces `Options.last_bar`;
   - validation of `bars` (`from == 0` or `from > to` is `BadOption`);
   - the window through `notation::window`;
   - the doc of `BARLINE_SLACK_SECONDS`.
2. `crates/strudeland-transcribe/src/notation.rs`: `WindowStart`, `Window`, `window`, and
   their tests.
3. `crates/strudeland-cli/src/run.rs`: `bars: args.bars` in the `Options` literal, and a
   48 kHz regression test. `resolve_span` keeps its signature and body.
4. Lib tests: only the three `last_bar` settings of `a_requested_last_bar_keeps_trailing_rest_bars`
   (`:641`, `:652`, `:659`) change, and only those fields. Their assertions stay. New lib
   tests: the 48 kHz frame-rounded regression, `--bars 1-B` over a pickup, and the `bars`
   validation.
5. Stage 002: `suggest::suggest` returns `Suggestions`, and the helpers it composes become
   private. The crate doc (`lib.rs:1-8`), the README intro (`:3-7`) and the one AGENTS.md
   word that goes with it (`:333`, "audio I/O" to "audio devices") are fixed.
6. Docs: the README notation row (`:24`) and the AGENTS.md transcription layout bullet
   (`:332-339`).

Out:

- Rewording `BadSpan` in bars, the `MAX_NOTATION_BARS` text, and any new output field
  (Decisions 3 and 4).
- `load_wav_span` taking samples or a reader. Its `Wav` error carries the path.
- `GridHint::bar_span` and `resolve_span`. They stay as they are: the CLI still cuts the file
  in seconds.
- A span request type or a placement type in `Options` (Decision 6).
- `memo`/`backing`, `LOW_CONFIDENCE`, and the `no sidecar; the grid is inferred…` message
  (reverify item 7, filed separately).
- `SKILL.md`. The fix makes `:283-284` ("with `--bars 3-6` it is 3") true. No sentence
  becomes false: `:278-280` describes the pickup for a take as a whole, and `--bars 1-B` is
  covered by "it is A". The frozen-file gate checks that `SKILL.md` is unchanged.
- Eval: `tests/eval.rs`, `src/eval/*`, the README eval tables, `consts.rs`.

## Architecture

```
strudeland-cli run.rs                       strudeland-transcribe
────────────────────────────────            ─────────────────────────────────────────────
args.bars ─────────────────────────────────► Options.bars (only Some changes behaviour)
   │
resolve_span(args, hint)   (unchanged)      transcribe(samples, rate, hint, opts)
  --bars  → hint.bar_span → load::Span        span_start_seconds: notes and onsets to memo
  --start → load::Span::Seconds                 time, resolve_grid's span_start (unchanged)
  else    → load::Span::Whole                 memo_seconds: Bar1PastEnd (unchanged)
   │                                           │
load_wav_span(path, span) → load::Audio        grid Known:
   │                                             bars Some((a, b)) → Requested{from: a, to: b}
   ├─ audio.start_seconds ──► span_start_seconds          None     → SpanBar(span_bar(..))
   └─ audio.memo_seconds ───► memo_seconds     notation::window(start, &notated)
                                                 → Bars{first,last} → render_notation
                                                 → TooLong{last_bar} → today's warning
                                                 → Empty
```

The CLI still cuts the WAV in seconds. Only the way back from seconds to a bar goes away: for a
bars request the notation's bars come from `opts.bars`, not from the rounded start.

## Technical design

### The crate API (`lib.rs`)

```rust
pub struct Options {
    pub grid: GridDivision,
    pub key: Option<Key>,
    pub tuning_cents: Option<f32>,
    pub shift_ms: f64,
    pub span_start_seconds: f64,     // unchanged
    pub memo_seconds: Option<f64>,   // unchanged
    /// `--bars A-B` as `(A, B)`: memo bars, 1-based and inclusive, `1 <= A <= B`. The
    /// notation then covers exactly bars A to B, rest bars included, whatever bar the samples
    /// start in. `None` notates from the bar the samples start in (or the pickup bar) to the
    /// last bar with a note.
    pub bars: Option<(u32, u32)>,    // replaces last_bar
    pub debug: bool,
}
```

**Why a tuple and not a named struct.** `TranscribeArgs::bars` (`cli.rs:435`) is already
`Option<(u32, u32)>`, validated by `parse_memo_bars` (`cli.rs:467`), so the CLI passes it
through as is. A named `{ from, to }` struct would be one more public type for two fields
whose order the doc states. The crate destructures it once, as `(from, to)`.

`span_start_seconds` keeps its meaning: the crate must place notes at the frame-rounded start,
because the JSON note times, `onsets` and `resolve_grid`'s `span_start` use it today and must
not move by a microsecond. A bars request with samples that do not start at bar A gets A..B
anyway. That is the contract, and the doc says so.

Validation in `transcribe` (`lib.rs:90-109`): the four finite checks and their texts are
unchanged. One check is new, after them: `Some((from, to))` with `from == 0 || from > to` is
`BadOption(format!("the bars must be 1 <= A <= B (got {from}-{to})"))`. The CLI cannot reach
it, because `parse_memo_bars` refuses such input first. It stays, library-only, because
`from == 0` would otherwise silently mean the pickup bar.

### The window (`notation.rs`)

```rust
/// What fixes the notation's first bar.
pub(crate) enum WindowStart {
    /// `Options.bars`: exactly `from..=to`.
    Requested { from: u32, to: u32 },
    /// The bar the samples start in (`span_bar`; 0 before bar 1): today's rule.
    SpanBar(u32),
}

/// The bars to notate.
pub(crate) enum Window {
    Bars { first: u32, last: u32 },
    /// Past `MAX_NOTATION_BARS`: the caller warns with `last_bar`.
    TooLong { last_bar: u32 },
    /// Nothing to notate.
    Empty,
}

/// `notated` is the notes the notation can show (`lib.rs:249-254`).
pub(crate) fn window(start: WindowStart, notated: &[Note]) -> Window;
```

The body is today's `lib.rs:255-273`, with the two starts:

- `note_bar` = the highest `bar` in `notated`.
- `Requested { from, to }`: `(first, last) = (from, to)`.
- `SpanBar(b)`: `pickup = b == 0 && notated has a bar-0 note`;
  `first = if pickup { 0 } else { b.max(1) }`; `last = note_bar.unwrap_or(0)`. This is
  today's code with `opts.last_bar` `None`. Today the CLI sets `last_bar` only for `--bars`,
  so whole and seconds spans keep their output.
- Then, in today's order: `last > MAX_NOTATION_BARS` gives `TooLong { last_bar: last }`.
  Otherwise `note_bar.is_some_and(|b| b >= first) && last >= first` gives `Bars`. Otherwise
  `Empty`.

`lib.rs` keeps the warning text exactly: `format!("the notes reach bar {last_bar}; notation
is limited to {MAX_NOTATION_BARS} bars")`, bound from `Window::TooLong { last_bar }`.
`span_bar` and `BARLINE_SLACK_SECONDS` stay in `lib.rs`, unchanged in value. Only the slack's
doc changes, because it now serves whole and seconds spans only.

`WindowStart` is chosen inside the `GridOutcome::Known` arm (`lib.rs:206`) from `opts.bars`
and carried in `notation_grid` in place of the span bar. With `bars` set, `span_bar` is not
computed.

**Why `bars` output matches today wherever the bug does not hit.** For `--bars A-B` the grid
is the hint's grid: `bar_span` returns `None` under an unheard sidecar tempo, so a grid
inferred from the audio never pairs with a bars request. Its origin is
`bar1 + shift_ms/1000` on both sides (`grid.rs:109`, `:295`). The loaded start is within
`0.5/rate` of `origin + (A-1)*bar`, so `span_bar` is A or A-1, never A+1. When it is A
(`A >= 1`), today gives `first = A` (no pickup, since `span_bar != 0`), the same as
`Requested`. When it is A-1, today is the bug. Today's `last` for `--bars` is already B.

### The CLI (`run.rs`)

One line of `run_transcribe` changes: `last_bar: args.bars.map(|(_, to)| to)` (`:644`)
becomes `bars: args.bars`. `resolve_span`, `build_hint` and the order of work (hint, key,
span with `BarsNeedGrid` before any read, load, transcribe) are unchanged, so every exit 4 and
exit 1 path, and its text, is reached in the same order.

### The suggestion fold (stage 002, `suggest.rs`)

```rust
/// The suggestions for `raw` on `grid` quantized at `requested` with timing spread
/// `requested_spread_ms`, under the `--shift` already `given_shift_ms`.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestions {
    /// `GridReport::suggested_division`.
    pub division: Option<GridDivision>,
    /// `GridReport::suggested_shift_ms`: `given_shift_ms` plus the suggested delta.
    pub shift_ms: Option<f64>,
    /// The division warning, then the shift warning, each when suggested.
    pub warnings: Vec<String>,
}

pub fn suggest(raw: &[RawNote], grid: &Grid, requested: GridDivision,
               requested_spread_ms: f32, given_shift_ms: f64) -> Suggestions;
```

The derives are for `suggest_is_its_parts`, which compares whole values. The body is
`lib.rs:184-200` moved. The shift is measured on the suggested division when there is one,
else on the requested one with `requested_spread_ms`. The `shift_warning` gets the same
arguments. `shift_ms` is `given_shift_ms + delta`, the same f64 addition as `lib.rs:223`.
`lib.rs` then:

1. calls `suggest` (it needs `q.timing_spread_ms`);
2. extends `out.warnings` with `q.warnings`, then with the suggestion warnings, which is
   today's order (`:191`, `:192-200`);
3. fills `suggested_division` and `suggested_shift_ms` from the result.

`suggest_division`, `rhythm_fit`, `suggest_shift`, `division_warning`, `shift_warning` and
`DivisionSuggestion` lose `pub`. Only `suggest` and `Suggestions` stay `pub` (crate-visible:
the module is private).

### Texts that must not change

| Text | Where | Stage |
|---|---|---|
| `the span start must be a finite number (got …)`, `the memo length …`, `--shift …`, `--tuning …` (`bad(...)`) | `lib.rs:90-109` | 001 leaves them as they are |
| `the notes reach bar {last_bar}; notation is limited to {MAX_NOTATION_BARS} bars` | `lib.rs:260-263` | 001 |
| `timing fits --grid {} better (spread {:.0} ms vs {:.0} ms)`, both `notes sit ~{:.0} ms {side} …` forms | `suggest.rs:144-171` | 002 |
| every `BadSpan` text | `load.rs:106-146` | frozen |
| `BarsNeedGrid` and every CLI `Failure` text | `error.rs`, `run.rs` | frozen |

Stage 001 writes a literal-count script over these, and both stages run it.

## Stages

1. [001-span-request.md](001-span-request.md). First the proof setup, before any code edit:
   - the synthetic memo grid and its requests;
   - the base binary's outputs;
   - the eval table and a per-case JSON dump of every eval transcription;
   - red runs of the two regression tests: the CLI one in its final form, the lib one written
     with `last_bar` (the one field that changes).

   Then `Options.bars`, `notation::window`, the CLI line, the three `last_bar` settings moved
   to `bars`, and the new tests. The grid runs again and a classifier checks every difference.
2. [002-suggest-and-doc.md](002-suggest-and-doc.md). The suggestion fold and the doc fixes.
   Every grid output and the eval dump must be byte-identical to 001's.

**Why two stages, split this way.** 001 holds the only visible change. Its proof (base
against 001, every diff classified as the fix) must run on a diff that holds nothing else.
002 is a pure refactor. Measured against 001's outputs it needs no classifier: any byte of
difference is a defect. Folding the suggestion change into 001 would turn its warnings and
`suggested*` fields into "should be identical" lines inside a diff that already has allowed
changes, which is a weaker check. The baseline is not a stage of its own. It is 001's first
steps, logged before the first source edit, as 0131 did with its pins. A separate stage would
add a full gate run over unchanged code and nothing else. The regression tests go in with the
fix, not as failing tests in an earlier commit. Their red runs on the base code are logged in
001, which shows they test the bug.

The stages are sequential: 002's zero-diff check compares against 001's outputs.

## Dependencies

None new. The scratch generator (`$S/0132-gen/`, never committed) depends on
`strudeland-transcribe` (feature `eval`) and `strudeland-project` by path, and on
`serde_json` with `float_roundtrip`. It builds with a copy of the workspace `Cargo.lock`,
`--offline`, in its own target directory.

## Risks

- **The grid misses a path.** Mitigation: the grid crosses three rates, five tempos, four
  meters, bar 1 at 0, 0.5 and 0.73 s and at a fractional sidecar frame, the sidecar kinds
  (heard, none, unheard, recovered, unreadable), a pickup, a tie across a barline, a truncated
  file and an over-10-minute file, with seconds, bars, shift, grid, key, debug and every
  error request. The generator predicts each bars case's hit with the crate's own
  `GridHint::bar_span` and `load_wav_span`, and only `span_bar`'s formula copied. The
  classifier requires every predicted hit with a notation at A-1 to change, and every other
  case to stay identical. A prediction that disagrees with the base output is a STOP.
- **f64 drift from moving code.** `window` and `suggest` move expressions without changing
  them. The eval dump (every eval transcription as JSON, `float_roundtrip`) and the grid
  outputs compare bytes, so a reordered float operation shows up.
- **A library caller's `bars`.** A caller that sets `bars` with samples that do not start at
  bar A gets A..B anyway. That is the contract, and the doc says so. `None` with a nonzero
  `span_start_seconds` behaves as today.

## Open questions

None. The refine review settled both earlier ones: the smaller `Options` form (Decision 6),
and the library-only `bars` validation stays.
