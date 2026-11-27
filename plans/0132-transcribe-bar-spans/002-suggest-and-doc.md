# 002: one suggestion result, and a crate doc that says what the crate reads

## Goal

Two private tidy-ups with no visible change:

- The two suggestion calls and their warnings (`lib.rs:184-200` at `4af3182`) move into one
  `suggest::suggest` that returns a `Suggestions`. `lib.rs` reads the two `GridReport`
  fields and the warnings from it. The helpers it composes become private to `suggest`.
- The crate doc (`lib.rs:1-8`) and the README intro (`README.md:3-7`) stop saying the crate
  never takes paths. `load::load_wav_span` reads a WAV from a path. The stages take samples.
  The README's "no … audio I/O" becomes "no … audio devices", and so does the same phrase in
  the AGENTS.md transcription bullet (`AGENTS.md:333`), so the two stay consistent.

Every grid output, the eval table and the eval dump are byte-identical to stage 001's.

## Prerequisites

- Stage 001 is committed, and its gate logs (`$S/0132-001-*.log`) all end `exit 0`.
- `$S/0132-out/001/` and `$S/0132-001-evaldump.jsonl` exist. They are the reference for this
  stage. If they are missing (a new session or a cleared scratchpad), rebuild them from the
  001 commit before any edit here: rerun `grid.sh 001` and step 8 of the gate with `N=001`,
  and check that the classifier still passes against the base outputs. If the base outputs are
  missing too, regenerate them from `4af3182` by building that commit's CLI in this working
  tree. Use a WIP commit if needed. Never use `git worktree add`.

## Files

- Edited: `crates/strudeland-transcribe/src/suggest.rs`, `src/lib.rs`, `README.md` (lines
  3-7), and one line of `AGENTS.md` (`:333`: "audio I/O" to "audio devices").
- Frozen, on top of the shared list: `notation.rs` and `crates/strudeland-cli/src/run.rs`
  (001 is done with them), and the rest of `AGENTS.md`. The bullet still says what `suggest`
  holds.

## Steps

1. **`suggest.rs`.**
   - Add `Suggestions` (with `#[derive(Debug, Clone, PartialEq)]`, for the test's whole-value
     comparisons) and `suggest` as the proposal gives them ("The suggestion fold"), after
     `DivisionSuggestion`.
   - The body of `suggest` is `lib.rs:184-200` moved, in the same order, with `opts.grid`
     as `requested`, `q.timing_spread_ms` as `requested_spread_ms` and `opts.shift_ms` as
     `given_shift_ms`:
     1. `let division = suggest_division(raw, grid, requested);`
     2. `let (shift_division, spread) = match &division { Some(s) => (s.division, s.spread_ms), None => (requested, requested_spread_ms) };`
     3. `let shift = suggest_shift(raw, grid, shift_division, spread);`
     4. the warnings: the division warning (`division_warning(s, requested_spread_ms)`), then
        the shift warning (`shift_warning(delta, given_shift_ms, division.as_ref().map(|s| s.division))`),
        each when there is one;
     5. `division: division.map(|s| s.division)`,
        `shift_ms: shift.map(|delta| given_shift_ms + delta)`.
   - Keep the comment "the shift on the suggested division when there is one, so the two go
     together".
   - Remove `pub` from `DivisionSuggestion` and its fields, `suggest_division`, `rhythm_fit`,
     `suggest_shift`, `division_warning` and `shift_warning`. The module doc (`:1-19`) stays.
     Fix any doc link the visibility change breaks under `-D warnings`.
   - New test `suggest_is_its_parts`. For the fixtures `dotted_eighths_suggest_sixteenths_over_eighths`
     and `a_consistent_offset_suggests_a_shift` use (`suggest.rs:218-291`), each with a
     `given_shift_ms` of `0.0` and of `25.0`, build the expected `Suggestions` from the
     helpers by hand, in the order above, and assert equality. Also cover the case with no
     suggestion: `division` and `shift_ms` are `None`, and `warnings` is empty. This pins
     the warning order and the f64 addition.
2. **`lib.rs`.**
   - Replace `:184-200` and the two report fields with:

     ```rust
     let s = suggest::suggest(&raw, &grid, opts.grid, q.timing_spread_ms, opts.shift_ms);
     out.warnings.extend(q.warnings);
     out.warnings.extend(s.warnings);
     ```

     and then `suggested_division: s.division` and `suggested_shift_ms: s.shift_ms` in the
     `GridReport`. The rest of the `Known` arm (`timing_spread`, `notes`, `dropped`,
     `notation_grid`) is unchanged. The order (quantize, suggest, the quantize warnings, the
     suggestion warnings) is today's.
   - The crate doc, `lib.rs:1-8`: "The crate is pure: it holds no global state and reads no
     sidecars, devices or network. The `load` module reads a span of a WAV file. The other
     stages take samples and a grid hint." Then the rest of the paragraph (the stages, plain
     functions, `Transcription`, `eval`) as it is.
3. **`README.md:3-7`.** "… A pure library (no Tauri, V8, audio devices, threads, global state
   or `unsafe`); `strudeland transcribe` (`crates/strudeland-cli`) is its only caller.
   `load_wav_span` reads a span of a WAV file. The stages take samples and a `GridHint`, never
   a sidecar, so other callers (drum transcription, an app command) can use them on any
   audio." The stages table and everything after it are unchanged.

   "audio I/O" becomes "audio devices": the crate does read files.
4. **`AGENTS.md:333`.** In "(pure library, no Tauri, V8 or audio I/O; stages …", only
   "audio I/O" becomes "audio devices". No other byte of `AGENTS.md` changes (the frozen
   script checks it).
5. Targeted runs:
   - `cargo test --locked -p strudeland-transcribe --features eval --lib suggest`;
   - `cargo test --locked -p strudeland-transcribe --features eval --lib suggestions_name_a_better_grid_and_leave_the_notes_alone`
     (`lib.rs:964`, unchanged; it checks both fields and both warnings end to end).

## Tests

- New: `suggest_is_its_parts`.
- Unchanged: every test in `suggest.rs` and `lib.rs`, including
  `suggestions_name_a_better_grid_and_leave_the_notes_alone`, and the CLI transcribe tests.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=002`: steps 1-11.

- Step 9 is `diff -r` against `$S/0132-out/001`. It must report nothing: no classifier is
  needed, because this stage changes no output.
- Step 8 compares against the base dump, and so against 001's too. Also
  `cmp "$S/0132-001-evaldump.jsonl" "$S/0132-002-evaldump.jsonl"`, logged as `evaldump-001`.
- Step 10 checks that, against the 001 commit, only `lib.rs`, `suggest.rs`, `README.md`,
  the one `AGENTS.md` line and plan files changed.
- Step 11's 002 column: `suggest::division_warning(` 0 and `suggest::suggest(` 1 in `lib.rs`,
  and the three suggestion literals still once each in `suggest.rs`.

## STOP

- Any `diff -r` output in step 9, or any eval or dump difference.
- A suggestion literal that has to change, or a `GridReport` field whose value would change.
- A needed edit to `notation.rs`, `run.rs`, any shared frozen file, or any `AGENTS.md` line
  other than `:333`.
- Anything in the shared STOP list.

## Commit message

```
refactor(transcribe): one suggestion result, and say what reads files

The grid division and shift suggestions come from one call that
returns both report fields and their warnings in order, so the
transcriber no longer threads the division into the shift by hand.
The crate and README docs now say that load reads a WAV file and the
other stages take samples, and say "no audio devices" where they said
"no audio I/O". No output changes.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
