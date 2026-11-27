# 0132: bar spans go through the transcriber as bars

## Goal

`strudeland transcribe --bars A-B` asks for bars A to B. Today the CLI turns that request into
seconds, the WAV loader rounds the start to a frame, and the transcriber turns the rounded
seconds back into a bar. The rounding can put the start up to `0.5/rate` before the barline,
10 times the `1e-6` slack, so the notation starts at bar A-1 with a leading `~`. This happens
at ordinary tempos (110 bpm at 48 kHz, `--bars 2-3`; [reverify.md](reverify.md) §1).

After this change, the request reaches the transcriber as a request:

- `Options` carries a `SpanRequest` (`Whole`, `Seconds { start, end }` or
  `Bars { from, to }`), and a `Placement` that says where the loaded samples sit in the memo.
  The old fields `span_start_seconds`, `memo_seconds` and `last_bar` go away.
- A new private `notation::window` picks the notation's bars. A `Bars` request gives exactly
  A..B. `Whole` and `Seconds` keep today's rule, the 1e-6 slack included, byte for byte.
- The CLI maps its flags to the request once. It still turns the request into seconds for
  `load_wav_span`, and `load_wav_span` still takes a path.
- The two suggestion calls (`lib.rs:184-200`) become one private `suggest::suggest` that
  returns a `Suggestions`.
- The crate doc stops saying "never paths" (`lib.rs:4`, README `:5-6`).

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

## Scope

In:

1. `crates/strudeland-transcribe/src/lib.rs`:
   - `SpanRequest` and `Placement` (public, re-exported at the root);
   - `Options.span` and `Options.placement` replace `span_start_seconds`, `memo_seconds` and
     `last_bar`;
   - validation of a `Bars` request;
   - the window through `notation::window`;
   - the doc of `transcribe` and of `BARLINE_SLACK_SECONDS`.
2. `crates/strudeland-transcribe/src/notation.rs`: `WindowStart`, `Window`, `window`, and
   their tests.
3. `crates/strudeland-cli/src/run.rs`:
   - `span_request(args) -> SpanRequest`;
   - `resolve_span(&SpanRequest, &GridHint, shift_ms) -> Result<load::Span, Failure>`;
   - `Options { span, placement: Placement::of(&audio), .. }`;
   - a 48 kHz regression test.
4. The lib tests that set the old fields move to the new ones. Their assertions stay as they
   are. New lib tests: the 48 kHz frame-rounded regression, `--bars 1-B` over a pickup, the
   `Bars` validation, and `Placement::of`.
5. Stage 002: `suggest::suggest` returns `Suggestions`, and the four helpers it composes
   become private. The crate doc (`lib.rs:1-8`) and README intro (`:3-7`) are fixed.
6. Docs: the README notation row (`:24`) and the AGENTS.md transcription layout bullet
   (`:332-339`).

Out:

- Rewording `BadSpan` in bars, the `MAX_NOTATION_BARS` text, and any new output field
  (Decisions 3 and 4).
- `load_wav_span` taking samples or a reader. Its `Wav` error carries the path.
- `GridHint::bar_span`. It stays `pub`, unchanged: the CLI still cuts the file in seconds.
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
flags ──span_request──► SpanRequest ───────► Options.span
           │                                   │ (only Bars changes behaviour)
           ▼                                   ▼
resolve_span(req, hint, shift)              transcribe(samples, rate, hint, opts)
  Bars    → hint.bar_span → load::Span        placement.start_seconds: notes and onsets to
  Seconds → load::Span::Seconds                 memo time, resolve_grid's span_start
  Whole   → load::Span::Whole                 placement.memo_seconds: Bar1PastEnd
           │                                   │
load_wav_span(path, span) → load::Audio        grid Known:
           │                                     start = Bars{from,to} → Requested{from,to}
Placement::of(&audio) ─────────────────────►             otherwise     → SpanBar(span_bar(..))
                                               notation::window(start, &notated)
                                                 → Bars{first,last} → render_notation
                                                 → TooLong{last_bar} → today's warning
                                                 → Empty
```

The CLI still cuts the WAV in seconds. Only the way back from seconds to a bar goes away: for a
bars request the notation's bars come from the request, not from the rounded start.

## Technical design

### The crate API (`lib.rs`)

```rust
/// The part of the memo the caller asked for. Only `Bars` changes what `transcribe` does: its
/// notation covers exactly bars `from` to `to`. `Whole` and `Seconds` are notated from the bar
/// the samples start in (`Placement::start_seconds`), or from the pickup bar.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SpanRequest {
    /// The whole memo.
    #[default]
    Whole,
    /// `--start S --end S`, as asked (memo seconds).
    Seconds { start: f64, end: f64 },
    /// `--bars A-B`: memo bars, 1-based and inclusive, `1 <= from <= to`.
    Bars { from: u32, to: u32 },
}

/// Where the samples sit in the memo, as `load_wav_span` read them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Placement {
    /// Memo seconds of `samples[0]` (`load::Audio::start_seconds`; 0.0 for the whole memo).
    pub start_seconds: f64,
    /// The memo's whole length in seconds, for `Bar1PastEnd`; `None` is the end of the
    /// samples.
    pub memo_seconds: Option<f64>,
}

impl Placement {
    /// The placement of a loaded span.
    pub fn of(audio: &load::Audio) -> Placement;
}

pub struct Options {
    pub grid: GridDivision,
    pub key: Option<Key>,
    pub tuning_cents: Option<f32>,
    pub shift_ms: f64,
    pub span: SpanRequest,      // replaces last_bar, and the start of the round trip
    pub placement: Placement,   // replaces span_start_seconds and memo_seconds
    pub debug: bool,
}
```

Why a `Placement` beside the request, and not the request alone: the crate must place notes at
the frame-rounded start, because the JSON note times, `onsets` and `resolve_grid`'s
`span_start` use it today and must not move by a microsecond. The requested start
(`Seconds.start`, or `bar_span`'s start) is not that value. Taking both from the loaded
`Audio` with `Placement::of` means the CLI cannot pass the requested start by mistake. It also
keeps `memo_seconds` from the header for a truncated file (`load.rs:57-58`), where it is
longer than the samples. `SpanRequest::Seconds { start, end }` carries the asked values
because the CLI derives `load::Span` from the request (`resolve_span`). The crate does not
read them.

Validation in `transcribe` (`lib.rs:90-109`): the same checks and texts, read from
`opts.placement`. The literals `"the span start"` and `"the memo length"` are unchanged. One
check is new: a `Bars { from, to }` that is not `1 <= from <= to` is
`BadOption(format!("the bars must be 1 <= A <= B (got {from}-{to})"))`. The CLI cannot reach
it, because `parse_memo_bars` (`cli.rs:467`) refuses such input first.

### The window (`notation.rs`)

```rust
/// What fixes the notation's first bar.
pub(crate) enum WindowStart {
    /// A bars request: exactly `from..=to`.
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
  so `Whole` and `Seconds` keep their output.
- Then, in today's order: `last > MAX_NOTATION_BARS` gives `TooLong { last_bar: last }`.
  Otherwise `note_bar.is_some_and(|b| b >= first) && last >= first` gives `Bars`. Otherwise
  `Empty`.

`lib.rs` keeps the warning text exactly: `format!("the notes reach bar {last_bar}; notation
is limited to {MAX_NOTATION_BARS} bars")`, bound from `Window::TooLong { last_bar }`.
`span_bar` and `BARLINE_SLACK_SECONDS` stay in `lib.rs`, unchanged in value. Only the slack's
doc changes, because it now serves seconds spans only.

`WindowStart` is chosen inside the `GridOutcome::Known` arm (`lib.rs:206`) and carried in
`notation_grid` in place of the span bar. For a `Bars` request, `span_bar` is not computed.

**Why `Bars` output matches today wherever the bug does not hit.** For `--bars A-B` the grid
is the hint's grid: `bar_span` returns `None` under an unheard sidecar tempo, so a grid
inferred from the audio never pairs with a bars request. Its origin is
`bar1 + shift_ms/1000` on both sides (`grid.rs:109`, `:295`). The loaded start is within
`0.5/rate` of `origin + (A-1)*bar`, so `span_bar` is A or A-1, never A+1. When it is A
(`A >= 1`), today gives `first = A` (no pickup, since `span_bar != 0`), the same as
`Requested`. When it is A-1, today is the bug. Today's `last` for `--bars` is already B.

### The CLI (`run.rs`)

```rust
/// The span the flags ask for: `--bars`, `--start`/`--end`, or the whole memo.
fn span_request(args: &TranscribeArgs) -> SpanRequest {
    match (args.bars, args.start, args.end) {
        (Some((from, to)), _, _) => SpanRequest::Bars { from, to },
        (None, Some(start), Some(end)) => SpanRequest::Seconds { start, end },
        _ => SpanRequest::Whole,
    }
}

/// The part of the file `transcribe` reads for `request`: bars through the hint's grid (its
/// bar 1 moved by `shift_ms`, as the grid's is), seconds as given, or the whole memo.
fn resolve_span(request: &SpanRequest, hint: &GridHint, shift_ms: f64) -> Result<Span, Failure>;
```

`resolve_span`'s arms and errors are today's (`run.rs:663-680`). `run_transcribe` calls
`span_request` once and passes the request to `resolve_span` and to `Options.span`, and builds
`placement: Placement::of(&audio)`. The order of work does not change: hint, key, span
(`BarsNeedGrid` before any read), load, transcribe. So every exit 4 and exit 1 path, and its
text, is reached in the same order.

### The suggestion fold (stage 002, `suggest.rs`)

```rust
/// The suggestions for `raw` on `grid` quantized at `requested` with timing spread
/// `requested_spread_ms`, under the `--shift` already `given_shift_ms`.
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

The body is `lib.rs:184-200` moved. The shift is measured on the suggested division when
there is one, else on the requested one with `requested_spread_ms`. The `shift_warning` gets
the same arguments. `shift_ms` is `given_shift_ms + delta`, the same f64 addition as
`lib.rs:223`. `lib.rs` then:

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
| `the span start must be a finite number (got …)`, `the memo length …`, `--shift …`, `--tuning …` (`bad(...)`) | `lib.rs:90-109` | 001 moves the reads, not the literals |
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
     against the old `Options` fields.

   Then `SpanRequest`, `Placement`, `notation::window`, the CLI mapping, the tests moved to
   the new fields, and the new tests. The grid runs again and a classifier checks every
   difference.
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
`strudeland-transcribe` (feature `eval`) and `strudeland-project` by path. It builds with a
copy of the workspace `Cargo.lock`, `--offline`, in its own target directory.

## Risks

- **The grid misses a path.** Mitigation: the grid crosses three rates, five tempos, four
  meters, bar 1 at 0, 0.5 and 0.73 s and at a fractional sidecar frame, the sidecar kinds
  (heard, none, unheard, recovered, unreadable), a pickup, a tie across a barline, a truncated
  file and an over-10-minute file, with seconds, bars, shift, grid, key, debug and every
  error request. The generator predicts each bars case's hit from the same f64 arithmetic as
  the code. The classifier requires every predicted hit with a notation at A-1 to change, and
  every other case to stay identical. A prediction that disagrees with the base output is a
  STOP.
- **f64 drift from moving code.** `window` and `suggest` move expressions without changing
  them. The eval dump (every eval transcription as JSON, `float_roundtrip`) and the grid
  outputs compare bytes, so a reordered float operation shows up.
- **`Placement` for a library caller.** A caller that passes a `Bars` request with samples
  that do not start at bar A gets A..B anyway. That is the contract, and the doc says so.
  `Whole` with a nonzero `start_seconds` is accepted, as `span_start_seconds` was.

## Open questions

1. `Placement` is a new public type beside `SpanRequest`, so `Options` has the request and
   where the samples are, and no loose seconds fields. The alternative is to keep
   `span_start_seconds` and `memo_seconds` as two fields and only replace `last_bar` with
   `span`. That is smaller, but it keeps the field the bug went through as a free `f64`. The
   plan takes `Placement`. Say so if you prefer the smaller form: only 001's lib edits change.
2. The new `Bars` validation (`BadOption` for `from == 0` or `from > to`) is a library-only
   text the CLI cannot reach. It can be dropped, in which case `window` would notate whatever
   `from..=to` gives. The plan keeps it, because `from == 0` would silently mean the pickup
   bar.
