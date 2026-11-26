# 001: the finishing characterization

## Goal

Pin every finished output before anything moves. This means bytes, summaries (timings zeroed),
warnings, progress and loop points, for every finishing path, format and loudness setting, plus
forced overshoots. The only production change is a pure extraction: `finish_export` out of
`render_export`. There is no behaviour change.

## Prerequisites

- **The branch is at the plan commit,** with a clean tree.
- **Read the proposal's sections:** "Characterization (stage 001)" and Decisions 4, 9 and 10.
- **Have a macOS aarch64 machine** (the reference platform) to write the golden.

## Implementation

1. **`src/loop_export.rs`: extract the seam.** Move everything in `render_export` after the
   `render_audio_for` call into:

   ```rust
   pub(crate) fn finish_export(
       pipeline: PipelineAudio,
       preset: ExportPreset,
       out: &OutputOptions,
       progress: &dyn Fn(Progress),
   ) -> Result<Vec<ExportFile>, RenderError>
   ```

   `render_export` becomes `finish_export(render_audio_for(code, opts,
   AudioMode::Preset(preset), progress)?, preset, &opts.output, progress)`. Keep the statement
   order exactly. `PipelineAudio` and `LoopAudio` need no visibility change: they are private
   items of the crate root, which its descendant modules (`loop_export`, `characterization`)
   already see, fields included.
2. **`src/testing.rs`: add the fixtures.** No `unwrap` and no indexing:
   - `pub fn structure() -> Structure`: 120 bpm, 4/4, cps 0.5, no sections, `loops: 1`, and
     zero bars.
   - `pub fn audio(channels: Vec<Vec<f32>>, sample_rate: u32) -> RenderedAudio`:
     - `music_frames` and `summary.frames` are the first channel's length;
     - `summary.seconds` is `frames / sample_rate`;
     - `cps` 0.5 (the structure's), window `0..1` linear, `chunks: vec![]`, `events: None`
       and `plan_errors: vec![]`;
     - the summary is `summary(&[])`, with `format: None`, `bytes: None` and the frames above.

   Re-export the needed host types where `testing` already uses `strudeland_host`.
3. **`Cargo.toml`:** add `sha2 = "=0.10.9"` to `[dev-dependencies]`.
   - `Cargo.lock` gains no new package, only the `"sha2 0.10.9"` edge in `strudeland-render`'s
     dependency list (the lock holds `sha2 0.10.9` and `0.11.1`, so the edge names the
     version). Commit it: CI runs `cargo test --locked`.
   - `tests/licence.rs` (`every_dependency_is_pinned_exactly`) accepts the `= "=…"` line, and
     sha2 is MIT OR Apache-2.0.
4. **`.prettierignore`:** add `crates/strudeland-render/tests/golden/`, with a comment, beside
   the `crates/strudeland-render/tests/vectors/` entry. `bun run lint` is `prettier --check .`,
   and serde's pretty JSON is not prettier's layout.
5. **`src/characterization.rs`:** declare it with `#[cfg(test)] mod characterization;` in
   `lib.rs`.
   - **Inputs** (seeded and deterministic, at 44.1 kHz):
     - **`bed()`:** 2.0 s stereo, made of LCG noise (the `encode/mod.rs` test `noise()` state
       and constants) at 0.05, plus 220 Hz on the left and 330 Hz on the right at 0.3.
     - **`hot()`:** `bed()` times 3.
     - **`silence()`:** 1.0 s of zeros.
     - **`dense()`:** LCG noise at 0.5 for 3 s, the `tests/encode.rs` `noise(3)` recipe.
     - **One-shot input:** `music_frames` is 1.5 s, followed by a 0.3 s decaying sine tail and
       then 0.2 s of exact zeros. The trim keeps the tail and drops the zeros.
   - **Loop fixtures** (`PipelineAudio`, `stems: None`). Give each a window of 4 bars at cps
     2.0: 0.5 s per bar and a 2 s window, not the 8 s window cps 0.5 would give. Frames per bar
     are `join::window_frames(0, 1, 2.0)`. `audio.music_frames` is the whole window.
     - **Override `testing::audio`'s defaults explicitly,** or `splice` fails with "loop start
       is outside the planned window": `summary.window.to_bar = 4`, `cps = 2.0`, a start bar
       inside `0..4`, and for the bar-0 tail case `music_frames` (the window) shorter than the
       channels (window plus tail). `summary.frames` and `summary.seconds` follow the channels.
     - **Loop after bar 0** (start bar 2):
       - `loop_audio = LoopAudio { start_bar: 2, start_frame: window_frames(0, 2, cps),
         isolated_body: Some(body) }`.
       - `audio.channels` is exactly `music_frames` long, because `join_opening` requires it.
       - `body` is `music_frames - start_frame + tail` frames, with a tail of 0.4 s (1 s of
         loop plus the tail).
     - **Loop at bar 0:**
       - `LoopAudio { start_bar: 0, start_frame: 0, isolated_body: None }`.
       - `audio.channels` is `music_frames + tail`, with a tail of 2.5 bodies (5 s), so it
         folds twice.
     - Build intro-loop, tagged-loop and folded-loop after bar 0, and all three at bar 0.
   - **Cases**, in this order, each named `"<path>/<input>/<format>/<loudness>"`:
     - **Paths:** `standard` (`crate::finish(&testing::audio(..), ..)`), `one-shot`
       (`finish_export(.., OneShot, ..)`), and the six loop fixtures above.
     - **Formats:**
       - `Wav{B16}`, `Wav{B24}` and `Wav{F32}`;
       - `Mp3{Cbr, 0}` and `Mp3{Vbr, 2}`;
       - `Ogg{6}` and `Opus{160}`.
     - **Loudness:** `On{-14, -1}` and `Off`.
     - **Extra cases:**
       - silence (standard, one-shot, tagged-loop at bar 0; `Wav{B24}` and `Ogg{6}`; loudness
         on);
       - hot (standard and folded after bar 0; `Wav{B24}` and `Mp3{Cbr,0}`; loudness off);
       - **overshoot:** `dense()` as standard, one-shot, intro-loop after bar 0 and folded
         after bar 0, in `Mp3{Cbr,0}`, `Ogg{6}` and `Opus{160}`, with `On{-3, -1}`;
       - **the wrap peak:** folded after bar 0 (or tagged), `Wav{B24}`, loudness on with a
         target whose ideal gain exceeds the cap (record it in a comment). The isolated body
         has a step at the wrap: it ends near +0.9 and starts near -0.9. The program's handoff
         mirrors the wrap through `join_opening`'s 10 ms blend, so the wrap wins only by the
         blend's residue: make the opening's last 10 ms oppose the repeat's tail (for example
         near -0.9), which pulls the handoff slightly below the wrap;
       - **the cap on the program peak:** `hot()` folded after bar 0, `Wav{B24}`, `On{-14,
         -1}`;
       - **an unmeasurable body:** intro-loop after bar 0, `Wav{B24}` and `Ogg{6}`, `On{-14,
         -1}`, with `LoopAudio { start_bar: 3, start_frame: music_frames - 0.3 s, .. }` and a
         0.3 s isolated body (no tail). `splice` does not tie the frame to the bar, so the loop
         cut is under 400 ms (its `input_lufs`/`output_lufs` are `-inf`) while the program
         (2.3 s) is measurable;
       - **a silent two-cut intro-loop:** intro-loop after bar 0 over zeros, `Wav{B24}` and
         `Ogg{6}`, `On{-14, -1}`. The ideal is 0 (unmeasurable) and the cap `+inf`, so
         `gain_db == 0.0`: the 0 dB shortcut, `after = before`.
     - **The error section,** last, recorded as `{ "name", "error": e.to_string(), "progress"
       }`:
       - `Ogg { quality: 99 }`, `On{-14, -1}`, for standard, one-shot and intro-loop after bar
         0: an `Encode` error ("encode failed: ogg: quality 99 is not -1 to 10"), with no `Encoded`
         event;
       - a NaN sample (in the opening's first second), `Wav{B24}`, `On{-14, -1}`, for standard
         and tagged-loop after bar 0: `check_input`'s text ("render failed: the render contains
         a non-finite sample (frame …, channel …)"), with no `Encoded` event.
   - **What each case records:** the progress events (`format!("{p:?}")`) and, for each
     `ExportFile` (the standard path wraps its `RenderOutput` as role `single`):
     - `role`;
     - `bytes.len()`;
     - the lowercase hex SHA-256 of the bytes;
     - `loop_points` (serde);
     - the summary as `serde_json::Value`, after `summary.timings = Timings::default()`.
   - **Errors:** assert that exactly the error-section cases error and every other case is
     `Ok`, so a fixture mistake is caught. An unexpected error is reported by name.
   - **Golden:** `tests/golden/finish.json`, schema 1, pretty-printed, with a trailing newline.
     - With `STRUDELAND_WRITE_FINISH_GOLDEN=1`, write it and pass, on macOS aarch64 only. On
       any other target the variable fails the test without writing, naming the reference
       platform.
     - Otherwise, on macOS aarch64 (`cfg!(all(target_os = "macos", target_arch =
       "aarch64"))`), parse it and compare case by case. Name the cases and the first differing
       field, then `assert!`. The `jsmath` precedent (`strudeland-engine/tests/v8_math.rs:31`)
       gates on `target_os = "macos"` alone; this gate is stricter because the encoders' float
       paths also differ by architecture. `macos.yml` (`macos-15`, arm64, a debug `cargo test
       --locked --workspace`) enforces it on every push.
     - On other platforms, `eprintln!` that the comparison is skipped.
   - **The overshoot guard:** assert that some overshoot `standard` or `one-shot` case and some
     overshoot loop case have `summaries[*].loudness.encodes >= 2`. If not, lower the overshoot
     target toward -1 LUFS until both hold, and record the target in a comment.
   - **The cap guards,** read from the summaries alone (WAV, one pass, one cut, so `input_*` is
     the program's measure):
     - the program-peak case binds the cap: `gain_db == ceiling_dbtp - input_true_peak_db`, bit
       for bit (the same f64 expression as `finish_loop`), and `gain_db < target_lufs -
       input_lufs`;
     - the wrap case takes `peak_db` from the wrap: `gain_db < ceiling_dbtp -
       input_true_peak_db` and `gain_db < target_lufs - input_lufs`.

     If either fails, adjust that fixture (not the guard) and note why.
   - **Timing:** measure the test's run time in a debug build. If it is over 30 s, shorten the
     inputs (keep them over 400 ms) and note it.
6. **Write the golden** on macOS aarch64:

   ```sh
   STRUDELAND_WRITE_FINISH_GOLDEN=1 cargo test -p strudeland-render characterization
   ```

   Then run it again without the variable, and run it twice more to prove determinism. Run it
   once under `--release` too (`cargo test --release -p strudeland-render characterization`):
   the comparison must hold in both profiles.
   Skim the golden:
   - the mp3 one-shot sha differs from the mp3 standard sha for the same input (Gap versus
     NoGap);
   - loop summaries show `limiterCeilingDbtp: null`;
   - the hot cases show "will clip" (standard) and "may clip" (folded);
   - Opus loop points are at 48000;
   - the error cases have no `Encoded` event;
   - the unmeasurable body's loop file shows `-inf` loudness and the silent intro-loop shows
     `gainDb: 0.0`.
7. **`AGENTS.md`, Rules, render pipeline bullet:** add one sentence:
   "`crates/strudeland-render/tests/golden/finish.json` pins finished bytes, summaries,
   warnings and progress; it is written only by `STRUDELAND_WRITE_FINISH_GOLDEN=1 cargo test -p
   strudeland-render characterization` (on macOS aarch64, where it is judged) after an encoder
   crate bump, and `.prettierignore` keeps prettier off it."

## Tests

- **`characterization::finished_output_is_frozen`,** as above.
- **No existing test changes.**

## Validation

- **The stage gates,** logged once as in 000-prompt.
- **`git diff --stat`** shows only:
  - `crates/strudeland-render/{Cargo.toml, src/lib.rs, src/loop_export.rs, src/testing.rs,
    src/characterization.rs, tests/golden/finish.json}`;
  - `Cargo.lock`, only the `"sha2 0.10.9"` line in `strudeland-render`'s dependency list;
  - `.prettierignore`;
  - `AGENTS.md`.
- **`bun run lint`** passes (prettier skips the golden).

## Commit message

```
test(render): pin finished output across every finishing path

Add a characterization test that hashes the finished bytes and records
the summary, warnings, progress and loop points for plain renders,
one-shots and every loop preset, over every format and loudness
setting, plus forced lossy overshoots. The inputs are synthetic PCM,
reached through finish and a finish_export seam split out of
render_export. The golden is judged on macOS aarch64 and refreshed only
after an encoder crate bump.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
