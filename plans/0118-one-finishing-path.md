# 0118: one finishing path for every render

## Goal

Every render, whether a plain render, a one-shot or a loop preset, ends in one finishing loop.
That loop levels each file cut, encodes it through the `Codec` seam, decodes a lossy file again
and corrects an overshoot at most twice. How it levels is an explicit, tested strategy switch:
**Limited** or **RepeatSafeStatic**. Planning is split from rendering, so every step after the
headless host can be tested on hand-built plans and synthetic PCM.

Every finished byte, summary, warning and progress event stays exactly as it is today. A
permanent characterization test, added first, proves it after every stage.

## Background

The [re-verification](0118-one-finishing-path/reverify.md) has the evidence, with file and line.

- **Three finishing paths share one job and little code.** A plain render, a one-shot and a loop
  preset each level, encode, verify and correct. Each does so in its own function, with its own
  MP3 flush, its own warning texts and its own summary shape.
- **The loop path bypasses the codec seam.** `finish_loop` calls `encode_once_with_loop` and
  `verify::decoded_true_peak_db` directly. Its correction loop (a shared static gain, lowered
  across files) has no forced-overshoot test. Only the limited path's loop is tested through the
  private `Codec` stub.
- **The MP3 flush is hidden in a codec variant.** `Real::Legacy` reaches `encode_mp3` (LAME
  `FlushNoGap`). `Real::Standalone` and every loop file reach `encode_mp3_with_loop`
  (`FlushGap`). That is the only byte difference between the two variants: WAV, Ogg and Opus
  encode identically through both.
- **Planning and rendering are one function.** `render_audio_for` calls `plan::plan` (V8)
  inline. Unsupported refusal, the isolated-body checks, tail frames, chunk counts, progress
  offsets and the stem budget can only be reached by evaluating a song.
- **The loop arrangement and its finishing share one file.** In `loop_export.rs`, the
  arrangement (copy the opening, fold the tail, join the opening, measure the wrap, cut the
  files) sits beside the finishing loop. The arrangement is pure PCM work, but it is tested only
  through three helper tests and end-to-end songs.
- **No test pins finished bytes across a code change.** `tests/pipeline.rs:96-99` checks that
  `finish` repeats itself within one run. Nothing fails if a refactor moves a byte.

### Current behaviour

| | Standard (`finish`) | One-shot | Loop presets (intro, tagged, folded) |
|---|---|---|---|
| Entry | `finish` → `finish_file(.., false)` | `trim_silent_post_roll` → `finish_file(.., true)` | arrangement → `finish_loop` |
| Encoder | `encode_with(&Real::Legacy)` | `encode_with(&Real::Standalone)` | `encode_once_with_loop`, outside the seam |
| MP3 flush | `FlushNoGap` | `FlushGap` | `FlushGap`, with ID3 `LOOPSTART`/`LOOPLENGTH` |
| Leveling | static gain to the target, then the true-peak limiter | same | one static gain shared by every file, capped by the program, handoff and wrap peak; no limiter |
| Correction | the limiter ceiling is lowered by overshoot + 0.1 dB | same | the shared gain is lowered by the worst decoded peak's overshoot + 0.1 dB |
| Passes | `MAX_ENCODES` 3 | same | same |
| Loudness off, hot input | "… the file will clip" | same | "… the file may clip" (program, handoff and wrap peak) |
| Silence warning | `SILENCE_WARNING` | same | "loudness: skipped normalization … the true-peak ceiling still applies" |
| Extra warning | none | none | MP3: "mp3 encoder delay and padding can cause gaps …" |
| Corrected-pass warnings | replace the first pass's (truncate) | same | computed once |
| `Encoded{pass}` | after the encode, before the decode | same | once per pass, after every file's encode and decode, before the correction check |
| Summary window, frames | the render's | the trimmed render's | each file's own bars and frames |
| Loudness summary | `limiterCeilingDbtp` and `limiterSamples` from the limiter | same | `limiterCeilingDbtp: null`, `limiterSamples: 0` |
| Loop points | none | none | input-rate frames, except Opus: resampled to 48 kHz |

## Scope

In:

1. **A permanent characterization test** (stage 001). It hashes finished bytes and records the
   summary JSON (timings zeroed), warnings, progress and loop points. Its inputs are synthetic
   PCM, run through every finishing path, format and loudness setting, plus forced overshoots.
   Its golden is refreshed only on an encoder crate bump.
2. **Split planning from rendering** (stage 002). `plan::plan` stays the only V8 caller. A new
   `render_planned` takes a `PlannedRender` and an injected chunk renderer.
3. **An explicit encode request** (stage 003). It carries the loop range and the MP3 flush.
   `Real` becomes a unit struct. Opus loop-point resampling moves beside the encoders.
4. **A pure loop splice module** (stage 004), moved out of `loop_export.rs`, with synthetic-PCM
   tests.
5. **One finishing loop with the leveling strategy switch** (stage 005). It replaces
   `finish_file`, `encode::encode`, `encode_standalone`, `encode_with` and `finish_loop`. The
   stub codec covers loop ranges, and forced-overshoot loop tests are added.
6. **Docs** (stage 006): the crate README, `lib.rs`, the `AGENTS.md` true-peak rule and Layout.
   `GLOSSARY.md` is updated with this plan.

Out:

- **Any byte, warning, summary or progress change.** Unifying the warning texts, the summary
  shapes or the flushes is a separate decision. Task #146 checks the standard MP3 tails and may
  move standard renders to `FlushGap`.
- **The public signatures** of `render`, `render_audio`, `finish` and `render_export`, the
  `ExportFile`, `ExportRole` and `LoopPoints` types, and `export::Pipeline`. These are
  unchanged.
- **The CLI, `SKILL.md` and the app.** The CLI surface does not change.
- **The planner and the host bundle.** No `src/lib/render/*` edit, so no `host:build`,
  `engine:refs` or `host:parity`. `loudness.ts` and `chunks.ts` are untouched, so there is no
  `render:vectors`.
- **Stems.** `render_stems` keeps its raw float WAV writer. It moves onto `render_planned` only
  through `render_audio_for`.

## Technical design

### Characterization (stage 001)

- **The test lives in `crates/strudeland-render/src/characterization.rs`** (`#[cfg(test)] mod
  characterization;` in `lib.rs`). It is in the crate because loop presets and one-shots are
  reached below planning.
- **The golden is `crates/strudeland-render/tests/golden/finish.json`:** a pretty-printed
  `{ "schema": 1, "cases": [...] }`, in case order. Each case has:
  - `name`;
  - `files`: `[{ role, bytes, sha256, loopPoints }]`;
  - `summaries`: each file's `Summary` as JSON, with `timings` set to `Timings::default()`
    (warnings included);
  - `progress`: each event's `Debug` text.

  An expected-error case (the error section below) records `{ name, error, progress }` instead.
- **Prettier:** `bun run lint` is `prettier --check .`, and serde's pretty JSON is not
  prettier's layout. Stage 001 adds `crates/strudeland-render/tests/golden/` to
  `.prettierignore`, beside the render crate's vectors entry.
- **`STRUDELAND_WRITE_FINISH_GOLDEN=1`** writes the file, and refuses (fails the test without
  writing) unless the target is macOS aarch64. Otherwise the test compares case by case and
  names every differing case and field.
- **Reference platform:** the comparison runs on macOS aarch64 only. The `jsmath` precedent
  (`crates/strudeland-engine/tests/v8_math.rs:31`) gates on `target_os = "macos"` alone; the
  golden's `macos` plus `aarch64` is stricter, because the lossy encoders' float paths differ by
  architecture as well as the platform's `pow` and `log10`. The macOS workflow
  (`.github/workflows/macos.yml`: `macos-15`, arm64, a debug `cargo test --locked --workspace`)
  enforces it on every push. Other platforms run every case (no panic, the expected `Ok` or
  error) and skip the comparison with a printed note. Stage 001 also runs the golden once under
  `--release`, to show the comparison holds in both profiles.
- **Seam:** stage 001 makes one move, with no behaviour change. The body of `render_export`
  after `render_audio_for` becomes `pub(crate) fn finish_export(pipeline: PipelineAudio, preset:
  ExportPreset, out: &OutputOptions, progress: &dyn Fn(Progress)) -> Result<Vec<ExportFile>,
  RenderError>`. Standard cases enter through the public `finish`. These two entry points are
  the test's only calls into the pipeline, and both survive every later stage. `PipelineAudio`
  and `LoopAudio` need no visibility change: they are private items of the crate root, so its
  descendant modules (`loop_export`, `characterization`) already see them and their fields.
- **`testing::audio(channels, sample_rate) -> RenderedAudio`:** a doc-hidden fixture with one
  linear window, no chunks and an empty structure. `testing::structure()` gives the empty
  structure. Both are compiled into non-test builds: no `unwrap` and no indexing. Stage 005's
  rewrite of `tests/encode.rs` also uses them.
- **The matrix:**
  - **Paths:** standard, one-shot, and the loop splices below.
  - **Loop splices:**
    - `intro-loop`, `tagged-loop` and `folded-loop`, each with an isolated body (loop start
      after bar 0);
    - `tagged-loop` and `folded-loop` with the loop at bar 0 (no isolated body, tail on the
      opening, longer than one body so it folds twice);
    - `intro-loop` at bar 0.

    Each is a 4-bar window at cps 2.0 (0.5 s per bar, a 2 s window). `testing::audio` gives a
    `0..1` window, so the loop fixtures override it explicitly, or `splice` fails with "loop
    start is outside the planned window": `summary.window.to_bar = 4`, `cps = 2.0`, a start bar
    inside `0..4`, and for the bar-0 tail case `music_frames` (the window) shorter than the
    channels (window plus tail).
  - **Formats:** wav16, wav24, wav f32, mp3 cbr, mp3 vbr V2, ogg q6 and opus 160.
  - **Loudness:** on (-14 LUFS, -1 dBTP) and off.
  - **Extra cases:**
    - silence (standard, one-shot, tagged-loop at bar 0; ogg and wav24, loudness on);
    - hot input over 0 dBTP with loudness off (standard and folded; wav24 and mp3 cbr);
    - **forced overshoots:** dense seeded noise at -3 LUFS / -1 dBTP for standard, one-shot,
      intro-loop and folded-loop, in mp3 cbr, ogg and opus.
    - **the wrap peak:** a folded (or tagged) loop whose repeat has a step at the wrap, so the
      wrap peak sets `gain_db`. The handoff in the program mirrors the wrap through the 10 ms
      blend, so the wrap wins only by the blend's residue: make the opening's last 10 ms oppose
      the repeat's tail, so the handoff is slightly lower than the wrap. WAV, loudness on with a
      target whose ideal gain exceeds the cap.
    - **an unmeasurable body:** an intro-loop whose loop cut is under 400 ms (its
      `input_lufs`/`output_lufs` are `-inf`) while the program is measurable. `LoopAudio`'s
      `start_frame` is set inside the last bar (for example `music_frames - 0.3 s`), since
      `splice` does not tie the frame to the bar.
    - **a silent two-cut intro-loop:** zeros in the loop fixture's shape, loudness on. The cap
      is `+inf` and the ideal 0 (unmeasurable), so `gain_db == 0.0`: the 0 dB shortcut, with
      `after = before`.
  - **The error section** (expected errors, recorded as `{ name, error, progress }`):
    - `Ogg { quality: 99 }` for standard, one-shot and intro-loop: an `Encode` error, with no
      `Encoded` event;
    - a NaN sample, for standard and one loop preset: `check_input`'s text ("the render
      contains a non-finite sample (frame …, channel …)"), with no `Encoded` event.

    The test asserts that exactly these cases error and every other case is `Ok`, so a fixture
    mistake is caught.
  - **The overshoot guard:** the test asserts that at least one Limited case and one loop case
    in the overshoot group have `encodes >= 2`. If the real codecs do not overshoot, raise the
    target until they do, so the correction paths are pinned.
  - **The cap guards,** read from the summaries alone (WAV cases, one pass, one cut, so
    `input_*` is the program's):
    - one loop case binds the cap on its own peak: `gain_db == ceiling_dbtp -
      input_true_peak_db`, bit for bit, and `gain_db < target_lufs - input_lufs`;
    - the wrap case takes `peak_db` from the wrap: `gain_db < ceiling_dbtp -
      input_true_peak_db` and `gain_db < target_lufs - input_lufs`.
- **The synthetic PCM:** 2.0 s stereo at 44.1 kHz. It is a seeded LCG noise bed (the
  `noise()` of `encode/mod.rs` tests) under two sines, so that LUFS is finite and the limiter
  engages at -14 LUFS for the hot variant. One-shot input has a decaying tail after
  `music_frames` and then exact zeros, so the trim is observable.
- **Dev-dependency:** `sha2 = "=0.10.9"`. The package is already in `Cargo.lock` through
  `strudeland-host` (beside `sha2 0.11.1`), so the lock gains only the `"sha2 0.10.9"` edge in
  `strudeland-render`'s dependency list, no new package. Commit that lock change: CI builds with
  `--locked`. `tests/licence.rs` (`every_dependency_is_pinned_exactly`) accepts a `= "=…"`
  dev-dependency line, and sha2 is MIT OR Apache-2.0.

### Planning split (stage 002)

```rust
/// Renders one chunk job. The real one is the engine; tests inject synthetic PCM.
pub(crate) trait ChunkRenderer: Sync {
    fn render(&self, job: &EngineJob, stems: Option<&StemLayout>) -> Result<Rendered, JobError>;
}
pub(crate) struct EngineChunks; // strudeland_engine::render / render_with_stems

pub(crate) fn render_planned(
    planned: PlannedRender,
    timing: PlanTiming,            // { started: Instant, plan_ms: f64 }
    opts: &RenderOptions,
    mode: AudioMode,
    renderer: &dyn ChunkRenderer,
    progress: &dyn Fn(Progress),
) -> Result<PipelineAudio, RenderError>;
```

- **`render_audio_for` becomes four steps:**
  1. `started = Instant::now()`;
  2. `render::validate_jobs`;
  3. `plan::plan`;
  4. `render_planned(.., &EngineChunks, ..)`.

  Lines `lib.rs:105-246` move into `render_planned` as a pure cut and paste, with two closure
  substitutions: the opening pass's `match &stems` closure (`:166-169`) becomes `|job|
  renderer.render(job, stems.as_ref())`, and the repeat pass's plain `strudeland_engine::render`
  (`:199`) becomes `|job| renderer.render(job, None)`. The repeat pass passes `None` as the stem
  layout because it does today. `total_ms` still runs from `started`.
- **The tests' expected values come from reading `lib.rs:105-246` before the move,** not from
  the moved code, so a slip in the paste cannot set its own expectations.
- **Error types:** `ChunkRenderer::render` returns `strudeland_engine::JobError`, as
  `render_window`'s closure does today. The per-hap errors a fake returns in `Rendered.errors`
  are `strudeland_engine::RenderError` (imported as `EngineRenderError`, as `render.rs:8`
  does).
- **`PipelineAudio`, `LoopAudio` and `AudioMode` stay as they are.**

### Encode request (stage 003)

```rust
// encode/mp3.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mp3Flush { NoGap, Gap }   // LAME FlushNoGap / FlushGap
pub fn encode_mp3(channels: &[Vec<f32>], sample_rate: u32, mode: Mp3Mode, quality: u8,
                  flush: Mp3Flush, loop_range: Option<Range<u64>>) -> Result<Vec<u8>, RenderError>;

// encode/mod.rs
pub use mp3::Mp3Flush;
pub struct EncodeRequest<'a> {
    pub channels: &'a [Vec<f32>],
    pub sample_rate: u32,
    pub encoder: &'a EncoderOptions,
    /// MP3 only: how the stream is finalized.
    pub mp3_flush: Mp3Flush,
    /// The loop in decoded frames (exclusive end): input-rate frames, 48 kHz for Opus.
    pub loop_range: Option<Range<u64>>,
}
pub fn encode_once(request: &EncodeRequest<'_>) -> Result<Vec<u8>, RenderError>;
/// The decoded loop range and its reported points: identity, or each end resampled for Opus.
pub(crate) fn loop_points(format: Format, range: &Range<usize>, sample_rate: u32)
    -> (Range<u64>, LoopPoints);
pub(crate) trait Codec {
    fn encode(&self, request: &EncodeRequest<'_>) -> Result<Vec<u8>, RenderError>;
    fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError>;
}
pub(crate) struct Real;
```

- **`encode_mp3_with_loop` and `encode_once_with_loop` are deleted.** `encode_once` takes the
  request.
- **Deliberate public-API changes of the crate.** `encode::encode` and `encode::Encoded`
  (deleted in stage 005), `encode_once` (new signature), `encode_mp3` (new signature) and
  `encode_mp3_with_loop` (deleted) are `pub` today. No caller outside `strudeland-render` uses
  them (`rg` over the workspace finds none in `strudeland-cli`, `src-tauri` or any other
  crate), so these are intended removals and changes, not breaks.
- **`Mp3Flush::NoGap` with `Some(range)` is an `Encode` error** ("mp3: loop metadata needs the
  gap flush"). No caller asks for it: a loop is only tagged on a stream a gapless decoder can
  trim.
- **Every call site to update in stage 003** (line numbers at the plan commit):
  - `tests/encode.rs`: the imports `:6` (`encode_mp3_with_loop`) and `:11` (`encode_once`);
    `encode_once` at `:228`, `:235`, `:247`, `:407`, `:415`, `:420`, `:431`, `:472`, `:567`,
    `:576`, `:581`, `:784` and `:834`; `encode_mp3_with_loop` at `:273-274`;
  - `tests/pipeline.rs`: the import `:9` and the call `:551`.
- **The flush stays crate-internal to rendering.** It is a field of the encode request, not of
  the user-facing `EncoderOptions`, so the CLI, the app and `RenderOptions` are untouched.
- **The flush per path:** `finish` → `NoGap`; one-shot → `Gap`; loop files → `Gap`.
- **`loop_points`** is lifted verbatim from `loop_export.rs:361-380`.

### Loop splice (stage 004)

```rust
// splice.rs
pub(crate) struct FileCut<'a> {
    pub role: ExportRole,
    pub channels: Cow<'a, [Vec<f32>]>,
    /// The authored bars the file covers.
    pub bars: Range<u32>,
    /// The loop in input-rate frames (exclusive end).
    pub loop_range: Option<Range<usize>>,
}
pub(crate) struct Splice {
    pub cuts: Vec<FileCut<'static>>,
    /// The measure of the whole opening-plus-repeat buffer.
    pub program: Measure,
    /// `program.true_peak_db` and the wrap peak, whichever is higher.
    pub peak_db: f64,
}
pub(crate) fn splice(channels: Vec<Vec<f32>>, music_frames: u64, window: PlannedWindow,
                     sample_rate: u32, loop_audio: LoopAudio, preset: ExportPreset)
    -> Result<Splice, RenderError>;
```

- **What moves to `splice.rs`:** `copy_opening`, `fold_tail`, `join_opening` and `wrap_peak`,
  with their tests, and `loop_export.rs:69-147`. Every error text stays the same.
- **`FileCut` is defined in `splice.rs` and stays there.** Stage 005's `finish.rs` imports it
  (and adds `FileCut::whole` in an `impl` there or in `splice.rs`); it does not move between
  stages.
- **The error order is kept exactly** (`loop_export.rs:74-107`):
  1. "the plan has no loop start" (stays in `finish_export`);
  2. "loop start is outside the planned window";
  3. "loop start is too large" (the start frame's conversion);
  4. "loop end is too large" (the end frame's conversion);
  5. "loop has no frames";
  6. "the combined loop is too large";
  7. then either `copy_opening`'s error (bar 0) or "the plan has inconsistent loop history";
  8. then the fold, the join and `check_input`.
- **What `finish_export` keeps:** the measure timing (`measure_start` and `summary.timings.level_ms`)
  around the call.
- **`trim_silent_post_roll` stays** in `loop_export.rs`.

### One finishing loop (stage 005)

```rust
// finish.rs
pub(crate) enum Leveling {
    /// One cut: the static gain to the target, then the true-peak limiter. A lossy overshoot
    /// lowers the limiter's ceiling; the gain never changes.
    Limited,
    /// The cuts of a loop preset: one static gain shared by every cut, capped by `peak_db` and
    /// each cut's own peak, with no limiter. A lossy overshoot lowers the shared gain.
    RepeatSafeStatic { program: Measure, peak_db: f64 },
}
pub(crate) struct Finishing<'a> {
    pub cuts: Vec<FileCut<'a>>,  // FileCut is imported from splice.rs
    /// The render's summary: the base of every file's.
    pub summary: Summary,
    pub leveling: Leveling,
    pub mp3_flush: Mp3Flush,
}
pub(crate) fn finish_files(finishing: Finishing<'_>, out: &OutputOptions, codec: &dyn Codec,
                           progress: &dyn Fn(Progress)) -> Result<Vec<ExportFile>, RenderError>;
```

**The loop, as one sequence:**

1. **Set up the strategy once.**
   - **Limited:** fewer or more than one cut is `Render("limited leveling finishes one file")`.
     Otherwise `level::prepare(cut, sample_rate, &out.loudness)?` runs, with no setup warnings;
     the prepared warnings are pushed by every `level`.
   - **RepeatSafeStatic:** builds `before` (`[program]` for one cut, else `measure` of each
     cut) and folds `peak_db` with each `before.true_peak_db`. Then it computes `target`,
     `ceiling` and `gain_db`, with the off ("may clip") and unmeasurable setup warnings,
     exactly as `loop_export.rs:298-331`.
   - **MP3 with a loop:** when the format is MP3 and any cut has a loop range, push the MP3 loop
     warning after the strategy's setup warnings. This equals today's unconditional `format ==
     Format::Mp3` in `finish_loop` (`loop_export.rs:332-334`): every loop preset has a cut with
     a range (the intro-loop's loop cut, the tagged or folded cut), and no Limited cut has one.
2. **Each pass, 1 to `MAX_ENCODES`, visits the cuts in order.** For each cut:
   1. Level it:
      - **Limited:** `prepared.level(trim, &mut pass_warnings)`.
      - **Static:** the gain `10f64.powf(gain_db / 20.0)` applied as `(f64::from(x) * g) as
        f32`, skipped when `gain_db == 0.0`. `after` is `before` when the gain is 0, else
        `measure`.
   2. Encode through `codec.encode` with `mp3_flush` and `loop_points(format, range, rate)`.
   3. **Limited only:** emit `Progress::Encoded { pass }` right after the encode, before the
      decode (`encode/mod.rs:165`). For WAV that is the last thing the pass does.
   4. If the format is not WAV, decode through `codec.decoded_true_peak_db`. Track the worst
      decoded peak.
   5. Build the file's `LoudnessSummary`:
      - **Limited:** the leveled summary, with `decoded_true_peak_db` and `encodes` set.
      - **Static:** `limiter_ceiling_dbtp: None` and `limiter_samples: 0`, as
        `loop_export.rs:399-412`.

   **RepeatSafeStatic only:** after every cut of the pass is encoded and decoded, and before
   the correction check, emit `Progress::Encoded { pass }` once (`loop_export.rs:419`). So an
   encode or decode error on any cut in pass `p` leaves only `Encoded{1..p-1}`.
3. **Correct after the pass.** With `Loudness::On`, a worst peak over the ceiling, and a pass
   below `MAX_ENCODES`:
   - **Limited:** `trim -= worst - ceiling + CORRECTION_MARGIN_DB`.
   - **Static:** `gain_db -= worst - ceiling + CORRECTION_MARGIN_DB`.

   Clear `pass_warnings`, then run the next pass. At `MAX_ENCODES`, push the "lossy encode:
   decoded true peak {worst:.2} dBTP is above the ceiling {ceiling:.1} dBTP after 2 corrections"
   warning.
4. **Finish each file's summary.**
   - **Limited:** keeps the render's window, frames and seconds.
   - **Static:** sets them from the cut (`bars`, `channels[0].len()`, `frames / sample_rate`).
   - **Both:** `format` and `bytes` are set. `level_ms = summary.timings.level_ms + finishing
     level time`, and `encode_ms` is set. `total_ms += level_ms + encode_ms`. Warnings are the
     setup warnings, then the last pass's warnings, then the exhausted warning, appended after
     the render's.

**Progress is unchanged per strategy.** On every real path `Planned` and the
`ChunkRendered*` events come first, from `render_audio_for`; the characterization enters below
planning, so it records only the finishing events. The finishing events, today and after stage
005:

| Path | Outcome | Finishing events |
|---|---|---|
| Limited (standard, one-shot) | lossy, ends in pass `n` (1 to 3) | `Encoded{1}` … `Encoded{n}` |
| Limited | WAV (emits, then returns undecoded) | `Encoded{1}` |
| Limited | loudness off, lossy | `Encoded{1}` |
| Limited | overshoot after 2 corrections | `Encoded{1}`, `Encoded{2}`, `Encoded{3}`, then the exhausted warning |
| Limited | `prepare` or `check_input` error (NaN, ragged), one-shot trim error | none |
| Limited | encode error in pass `p` | `Encoded{1}` … `Encoded{p-1}` |
| Limited | decode error in pass `p` | `Encoded{1}` … `Encoded{p}` (emitted before the decode) |
| Loop presets | splice error (no loop start, window, frames, history, fold, join) or `check_input` | none, before any `Encoded` |
| Loop presets | ends in pass `n` (WAV and loudness off: `n = 1`) | `Encoded{1}` … `Encoded{n}` |
| Loop presets | overshoot after 2 corrections | `Encoded{1}`, `Encoded{2}`, `Encoded{3}`, then the exhausted warning |
| Loop presets | encode or decode error on any cut in pass `p` | `Encoded{1}` … `Encoded{p-1}` |

**Callers:**

- **`finish`:** a single `FileCut` borrowing `audio.channels` (`Cow::Borrowed`, role `Single`,
  bars = window, no loop), with `Limited` and `NoGap`. It returns the one file's `output`.
- **`finish_export`:**
  - one-shot: trim, then one cut with `Limited` and `Gap`;
  - loop presets: `splice`, then `RepeatSafeStatic { program, peak_db }` with `Gap`.
- **Deleted:** `finish_file`, `finish_loop`, `encode::encode`, `encode_standalone`,
  `encode_with`, `Encoded`, and `Part` (now `FileCut`).
- **Constants:** `MAX_ENCODES` and `CORRECTION_MARGIN_DB` stay in `encode`.

**Byte-identity arguments, for review:**

- **The Static gain expression** equals `level::linear_gain` with `limiter::apply_one`. Reuse
  them, keeping the `== 0.0` shortcut.
- **The two corrections are the same `a -= b - c + d` form.** Keep that order.
- **Limited needs no setup warnings,** because truncating per pass equals clearing
  `pass_warnings`.
- **WAV:** Limited returned before decoding; Static's worst peak stayed `-inf`. Both mean no
  decode and no correction.
- **One-shot:** the trim already set frames and seconds to the cut's, and Limited leaves them.
- **The `level_ms` base is 0** for every Limited input today: `render_audio_for` sets 0, and the
  trim adds to `render_ms`. So `finish_files`'s `base + level time` equals `finish_file`'s plain
  assignment for Limited, and `finish_loop`'s `summary.timings.level_ms + …` for Static.
- **Progress:** each strategy keeps its own emit point (the table above), so no event moves.

## Tests

**Replace, don't layer.** Every test of deleted code moves to its replacement.

- **Stage 001:** `characterization::finished_output_is_frozen`, the matrix above.
- **Stage 002, `lib.rs` (or `pipeline_audio.rs`) tests** on hand-built `PlannedRender`s with a
  fake `ChunkRenderer`. Each chunk returns a ramp, which identifies the chunk and frame. The
  jobs use the minimal job JSON
  `{"schema":2,"sampleRate":44100,"frames":N,"seed":0,"cps":2,"channels":{"count":2,"multiChannelOrbits":false},"orbits":[{"orbit":1,"channels":[0,1]}],"pauses":[],"values":[]}`
  (check the engine's validation and add fields if needed). Cases:
  - **Unsupported refusal:** a pause with one hap whose value is a sample (`{"s":"bd"}`;
    signatures come from haps, `controls.rs:1816`). The result is
    `Unsupported`, `Planned` is the only event, and the renderer was never called.
  - **Isolated-body errors, with exact text:**
    - "the isolated body has no loop start";
    - "loop begins past the opening";
    - "isolated body length differs from the opening loop".
  - **Tail frames:** `export.tail_frames` extends the channels and `summary.frames`.
  - **`music_frames` and `loop_start_frame` absent:** both fall back to `window_frames`.
  - **Chunk counts and progress:** `Planned{chunks: opening + repeat}`. `ChunkRendered` indices
    are offset by the opening count for repeat chunks, and every `of` is the total.
    `summary.chunks` and `summary.jobs = max(workers, repeat_workers)`.
  - **The stem budget:** too many stems, or too many frames, gives the `stems::validate_budget`
    errors before any chunk renders. A stem render passes `Some(layout)` to the renderer.
  - **Engine errors:** a renderer returning `errors` gives `render:` warnings in plan-then-render
    order.
- **Stage 003:**
  - `tests/encode.rs:273-274` moves to `encode_mp3(.., Mp3Flush::Gap, ..)`.
  - **New:** `NoGap` and `Gap` give different tails for the same input, and `encode_once` with
    `Gap` and no range equals the old standalone bytes (the characterization covers this too).
  - **`loop_points` unit tests:** Opus at 44.1 kHz (147..294 → 160..320, length 160, rate
    48000); WAV, Ogg and MP3 are the identity.
  - **`Mp3Flush::NoGap` with a loop range** is the `Encode` error, with its exact text.
  - The `encode/mod.rs` stub implements the request signature.
- **Stage 004, `splice.rs` tests:**
  - the three moved helper tests;
  - **intro-loop:** two cuts, the intro truncated to `music_frames` with bars `from..to`, and
    the loop with bars `start..to` and range `0..body`;
  - **tagged and folded:** one cut each, with the range `end..end+body` and roles from
    `roles()`;
  - **bar 0 with a two-body tail:** folded twice, so the body equals the sum;
  - **errors, with exact text:** "the plan has no loop start" stays in `finish_export`. The
    splice errors are "loop start is outside the planned window", "loop has no frames" and "the
    plan has inconsistent loop history" (`Some` body at start 0, `None` body at start > 0), in
    the order of "Loop splice (stage 004)";
  - `peak_db >= program.true_peak_db`, and equality when the wrap is quieter.
- **Stage 005, `finish.rs` tests** with the stub, extended to record each request's frames,
  loop range and flush, and to boost by decode index:
  - **All eight `encode/mod.rs` stub tests move** onto `finish_files` with `Limited` and one
    cut. Their assertions are unchanged.
  - **Shared gain moves both files:** intro-loop, ogg, boosts `[0.5, 0.0, 0.0, 0.0]`. Both files
    have `encodes == 2`, and both have the same `gain_db`, equal to the first pass's gain
    `- (overshoot + 0.1)`. Progress is `[1, 2]` and there are 4 decodes.
  - **The warning after two corrections:** boosts `[1.0, 2.5, 4.0]` on one cut give the exact
    text with the worst peak, `encodes == 3` and 3 decodes.
  - **Loudness off bypasses everything:** the bytes equal the stub encode of the cut, there is
    one pass, and the hot input gets the exact "may clip" text.
  - **WAV is never decoded,** for either strategy: 0 decodes and `decoded_true_peak_db: None`.
  - **The folded wrap peak dominates:** `peak_db` above every cut's own peak sets `gain_db =
    ceiling - peak_db` when the ideal gain is larger.
  - **The unmeasurable loop warning:** a short loop gets the exact "skipped normalization" text
    and no `SILENCE_WARNING`.
  - **The MP3 loop warning** appears for a cut with a range and not for a one-shot.
  - **Flush and range per caller:**
    - `finish` records `NoGap` with no range;
    - the one-shot through Limited records `Gap` with no range;
    - loop cuts record `Gap` with their ranges, resampled for Opus.
  - **Limited with two cuts** is an error.
  - **Progress keeps each strategy's emit point** (the progress table):
    - **Limited:** a stub decode error on the only cut still recorded `Encoded{1}`.
    - **Static, one-cut loop:** a decode failure on the only cut gives `Err` and no events.
    - **Static, intro-loop:** a decode failure on cut 2 of pass 1 gives `Err` and no events.
    - **Static, pass 2:** an overshoot in pass 1, then a decode failure in pass 2, gives `Err`
      and `[Encoded{1}]`.
- **`tests/encode.rs:739-847`** (`encode_loop_*` through the public `encode::encode`) move to
  `strudeland_render::finish(&testing::audio(input, SR), ..)` and read `summary.loudness` and
  `summary.warnings`.
- **`tests/pipeline.rs`** stays as it is, except for the `encode_once` import (request form).

## Dependencies

- **After #117** (shared export module, merged). `export::Pipeline` stays the caller-facing seam.
- **Before #146** (standard MP3 tails), which may flip the standard flush. That would be a golden
  refresh made deliberately in #146, not here.
- **No planner, bundle or vector regeneration.**

## Risks

- **Float-order drift.** Rewriting the gain expression, the correction, or the 0 dB shortcut
  moves bytes. Reuse the existing expressions verbatim; the characterization test catches any
  drift.
- **Warnings per pass versus once.** Limited's prepared warnings must not accumulate across
  passes, and Static's setup warnings must not repeat. Pinned by the golden and the stub tests.
- **Opus loop points.** The resampling is per end (`resampled_len(start)..resampled_len(end)`),
  never by length. The golden includes Opus loop cases.
- **Platform libm.** The golden is judged on macOS aarch64 only. Other CI platforms run the
  cases without comparing.
- **The crate lints** deny `unwrap`, `expect` and indexing outside `cfg(test)`. `testing.rs`,
  `splice.rs` and `finish.rs` are non-test code.
- **Windows `clippy::result_large_err`.** Box any new large `Err` payload; `export/error.rs:40`
  is the precedent. No new error type is planned.
- **Test runtime.** About 120 cases with LAME, Vorbis and Opus. `profile.dev.package."*"` (opt
  level 3) applies to dependencies only, so the encoders are optimised in a debug build but the
  render crate itself (measuring, limiting, true peak, the splice) is not. The loop fixtures use
  cps 2.0, so a 4-bar window is 2 s rather than 8 s. Measure the run time in stage 001 and
  shorten the inputs (keeping them over 400 ms) if it is over 30 s.

## Complexity

Medium. There are about 12 files in one crate, plus the docs. There are six commits; each is
green on its own, with the golden unchanged.

## Decisions

1. **(User) All bytes stay identical.** The MP3 flush becomes an explicit option of the encode
   request: `NoGap` for standard renders, `Gap` for one-shots and loop files. Task #146 checks
   the standard MP3 tails.
2. **(User) Both leveling strategies stay:** **Limited** (limiter-ceiling correction) and
   **RepeatSafeStatic** (shared static gain correction). They sit behind one finishing loop with
   an explicit, tested switch.
3. **(User) `AGENTS.md`'s true-peak rule states both correction rules.** "Never loosen" stays.
4. **(User) A permanent characterization test.** It covers bytes, summary JSON, warnings and
   progress over synthetic PCM, every path, format and loudness setting, and forced overshoots.
   It is refreshed only on encoder crate bumps, and must pass unchanged after every stage.
5. **Per-strategy warning texts, summary shapes and progress granularity are kept,** with one
   `Encoded{pass}` per pass per group.
6. **Progress is unchanged per strategy, with no visible change.** The emit point is a property
   of the strategy: Limited emits `Encoded{pass}` right after the encode and before the decode
   (WAV emits, then returns); RepeatSafeStatic emits it once after the whole pass's encode and
   decode loop, before the correction check. The success and failure orders in the progress
   table are today's, and the stage 005 stub tests pin both.
7. **The flush lives on `EncodeRequest`, not `EncoderOptions`.** This keeps the CLI, the app and
   `RenderOptions` untouched.
8. **The glossary names:**
   - **file cut** (`FileCut`), for what `Part` was;
   - **loop splice** (`splice`/`Splice`), for the arrangement;
   - **finishing**;
   - **leveling strategy.**

   "Part" and "arrangement" are avoided per the glossary, and so is "layout", the transport's
   term.
9. **The golden is judged on macOS aarch64 only,** following and tightening the `jsmath`
   precedent (which gates on macOS alone). The macOS workflow runs it on every push.
10. **The characterization enters through `finish` and `finish_export`.** These are the two
    seams that survive every stage, so its test file is not edited after stage 001.
