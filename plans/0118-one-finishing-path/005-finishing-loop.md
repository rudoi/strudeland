# 005: one finishing loop

## Goal

Every render finishes through `finish::finish_files`. That is one loop over file cuts and
passes, through the `Codec` seam, with the `Leveling` switch between **Limited** and
**RepeatSafeStatic**. The following are deleted:

- `finish_file`;
- `encode::encode`, `encode_standalone`, `encode_with` and `Encoded`;
- `finish_loop`.

The loop presets' correction gets forced-overshoot tests. There is no behaviour change.

## Prerequisites

- **Stage 004 is committed,** and the characterization passes.
- **Read the proposal's** "One finishing loop (stage 005)", including its progress table and
  byte-identity arguments, and Decisions 2, 5 and 6.
- **Keep three reference functions open:** `encode_with` (`encode/mod.rs`), `finish_loop`
  (`loop_export.rs`) and `Prepared::level` (`level/mod.rs`). Each expression below is lifted
  from them.

## Implementation

1. **Create `src/finish.rs`** (`mod finish;` in `lib.rs`), with the module doc: "Finishing: level,
   encode, verify and correct every file cut of a render. One loop, two leveling strategies."
   - **Import `FileCut`** from `splice` (it stays defined there). Add `FileCut::whole(audio:
     &RenderedAudio) -> FileCut<'_>`: `Single`, `Cow::Borrowed(&audio.channels)`, bars
     `window.from_bar..to_bar`, no range.
   - Add `Leveling`, `Finishing` and `finish_files`, as in the proposal.
   - **The strategy state** is a private enum:

     ```rust
     enum Level<'a> {
         Limited { prepared: Prepared<'a>, trim: f64 },
         Static { before: Vec<Measure>, gain_db: f64, target: Option<f64>, ceiling: Option<f64> },
     }
     ```

     It has three methods:
     - `setup(cuts, leveling, out, sample_rate, warnings: &mut Vec<String>) -> Result<Self>`;
     - `level(&self, index, cut, pass_warnings) -> Result<(Cow<[Vec<f32>]>, LoudnessSummary)>`;
     - `correct(&mut self, worst: f64, ceiling: f64)`.

     The `LoudnessSummary` from `level` has `decoded_true_peak_db: None` and `encodes: 0`.
   - **Limited `setup`:** `let [cut] = cuts else { return Err(render_error("limited leveling
     finishes one file")) }`, then `level::prepare(&cut.channels, sample_rate,
     &out.loudness)?`, with `trim: 0.0`.
   - **Static `setup`:** lift `loop_export.rs` `finish_loop` from `before` through `gain_db`
     verbatim. It pushes the "may clip" and "skipped normalization" warnings into the setup
     warnings.
   - **Limited `level`:** `prepared.level(trim, pass_warnings)` gives `(leveled.channels,
     leveled.summary)`.
   - **Static `level`:** `before[index]` gives `after`. With `gain_db == 0.0`, the channels are
     the borrowed cut and `after = before`. Otherwise:
     - `g = 10f64.powf(gain_db / 20.0)`;
     - each sample is `(f64::from(x) * g) as f32`, built as `Cow::Owned`;
     - `after = measure`.

     The summary has `enabled`, `target_lufs`, `ceiling_dbtp`, `limiter_ceiling_dbtp: None`,
     `input_*` from `before`, `gain_db`, `limiter_samples: 0` and `output_*` from `after`.
   - **`correct`:**
     - Limited: `*trim -= worst - ceiling + CORRECTION_MARGIN_DB`.
     - Static: `*gain_db -= worst - ceiling + CORRECTION_MARGIN_DB`.

     Keep exactly that operand order.
2. **`finish_files`,** following the proposal's four-step sequence:
   - `setup_warnings` from `setup`;
   - then the MP3 loop warning, if `format == Mp3 && cuts.iter().any(|c| c.loop_range.is_some())`.
     This equals today's unconditional `format == Format::Mp3` in `finish_loop`
     (`loop_export.rs:332-334`): every loop preset has a cut with a range, and no Limited cut
     has one;
   - then `for pass in 1..=MAX_ENCODES`:
     - `pass_warnings.clear()`, `worst = NEG_INFINITY`, and a vector of files.
     - **For each `(index, cut)`:**
       1. time the level;
       2. encode through `codec.encode(&EncodeRequest { .., mp3_flush, loop_range })`, using
          `encode::loop_points` for the range and points;
       3. **Limited only:** emit `progress(Progress::Encoded { pass })` right after the encode
          and before the decode, as `encode/mod.rs:165` does. For WAV this is the pass's last
          step;
       4. if the format is not WAV, drop the leveled channels and decode
          (`codec.decoded_true_peak_db`), then `worst = worst.max(dp)`;
       5. time the encode and decode;
       6. build the file's `Summary` from `finishing.summary.clone()`:
          - RepeatSafeStatic sets `window.from_bar`/`to_bar` from `cut.bars`, `frames`, and
            `seconds = frames as f64 / f64::from(sample_rate)`;
          - Limited leaves them;
          - both set `format`, `bytes`, and `loudness` with `decoded_true_peak_db` and
            `encodes = pass`.
     - **After the cuts, RepeatSafeStatic only:** emit `progress(Progress::Encoded { pass })`
       once, after every cut's encode and decode and before the correction check, as
       `loop_export.rs:419` does. An encode or decode error on any cut in pass `p` therefore
       leaves only `Encoded{1..p-1}`.
     - **Then the correction check:** if `out.loudness` is `On { true_peak_db: ceiling, .. }`
       and `worst > ceiling`:
       - with `pass < MAX_ENCODES`, call `level.correct(worst, ceiling)` and `continue`;
       - otherwise push the exhausted warning (the exact text from `encode_with` and
         `finish_loop`, with `{worst:.2}` and `{ceiling:.1}`).
     - **Then finish every file:**
       - `timings.level_ms = finishing.summary.timings.level_ms + level_ms`;
       - `encode_ms = encode_ms`;
       - `total_ms += level_ms_total + encode_ms`;
       - warnings extend with setup, then pass, then exhausted.

       Return the files.
   - **Afterwards:** `Err(render_error("no export encode pass was attempted"))`, as today.
   - **Timing:** Limited's setup time (`prepare`) counts toward `level_ms`, as in
     `encode_with`. Static's `before` measure counts too, as in `finish_loop`.
3. **Callers:**
   - **`lib.rs` `finish`:** `finish_files(Finishing { cuts: vec![FileCut::whole(audio)],
     summary: audio.summary.clone(), leveling: Leveling::Limited, mp3_flush: Mp3Flush::NoGap },
     out, &Real, progress)`. Then take the single file's `output`; zero files or several is
     `Render("finishing returned no file")`.
   - **Delete `finish_file`.** Update the `finish` doc (`lib.rs:273-276`) so it names the
     finishing loop, not `encode::encode`.
   - **`loop_export.rs` `finish_export`:**
     - one-shot: `trim_silent_post_roll(&mut audio)?`, then `finish_files(Finishing { cuts:
       vec![FileCut::whole(&audio)], summary: audio.summary.clone(), leveling: Limited,
       mp3_flush: Gap }, ..)`;
     - loops: `finish_files(Finishing { cuts: spliced.cuts, summary, leveling:
       RepeatSafeStatic { program: spliced.program, peak_db: spliced.peak_db }, mp3_flush: Gap
       }, ..)`.
   - **Delete `finish_loop`** and its imports.
   - **`encode/mod.rs`:** delete `Encoded`, `encode`, `encode_standalone`, `encode_with` and
     the local `ms_since`. Keep `encode_once`, `EncodeRequest`, `Mp3Flush`, `loop_points`,
     `Codec`, `Real`, `MAX_ENCODES` and `CORRECTION_MARGIN_DB`. Update the module doc: "the
     correction loop" is now in `finish`.
4. **Stub:**
   - Move the `encode/mod.rs` test `Stub` into `finish.rs`'s tests.
   - It records each request as `(frames, loop_range, mp3_flush)` in a `RefCell<Vec<_>>`.
   - `boost_db` is indexed by decode call: the entry at the decode count, else the last.
   - Add `fail_decode_at: Option<usize>`, which returns `Err(Encode("stub"))` on that decode
     (0-based over the whole run, across cuts and passes).

## Tests

**`finish.rs` `#[cfg(test)]`:**

- **Moved from `encode/mod.rs`, with their assertions unchanged:** run them through
  `finish_files` with `Limited`, one `FileCut` over the noise, `NoGap`, and a summary carrying
  the warning `"render: earlier"`. Read `loudness` and `warnings` from the file's summary.
  - `..._one_pass_when_the_decoded_peak_is_under_the_ceiling`;
  - `..._corrects_one_overshoot`;
  - `..._converges_on_a_limited_signal_with_a_steady_overshoot`;
  - `..._stops_after_two_corrections_and_warns`;
  - `..._never_corrects_with_loudness_off`;
  - `..._does_not_decode_wav`;
  - `..._silence_gets_no_gain_and_no_correction`;
  - `..._errors_pass_through`.

  Rename the prefix from `encode_loop_` to `limited_`.
- **`static_shared_gain_moves_both_files`:**
  - intro-loop cuts (two noise cuts), ogg, `LOUD`, boosts `[0.5, 0.0, 0.0, 0.0]`.
  - Both files have `encodes == 2`, and their `gain_db` values are equal.
  - That gain is `first_gain - (dp1 - -1.0 + CORRECTION_MARGIN_DB)`, where `first_gain` is the
    gain of a run with boosts `[0.0]` and `dp1` is pass 1's worst decoded peak.
  - Progress is `[Encoded{1}, Encoded{2}]`, there are 4 decodes, and `limiter_ceiling_dbtp` is
    `None`.
- **`static_stops_after_two_corrections_with_the_worst_peak`:** one loop cut, boosts `[1.0,
  2.5, 4.0]`. `encodes == 3`, there are 3 decodes, and the final warning is the exact
  `format!("lossy encode: decoded true peak {worst:.2} dBTP is above the ceiling -1.0 dBTP after
  2 corrections")`.
- **`static_loudness_off_is_a_pcm_bypass`:** hot noise (× 2.5), `Off`, boost 3.0.
  - Bytes equal the stub encode of the cut.
  - There is one pass, `gain_db == 0.0`, and one warning that equals `format!("loudness off:
    true peak is {peak:.1} dBTP (above 0 dBFS); the file may clip")`.
- **`neither_strategy_decodes_wav`:** both strategies with WAV and boost 10: 0 decodes and
  `decoded_true_peak_db == None`.
- **`the_wrap_peak_caps_the_shared_gain`:** one cut, with `peak_db` 3 dB above its own true
  peak and a target whose ideal gain is larger. `gain_db == -1.0 - peak_db` (the ceiling minus
  the peak).
- **`an_unmeasurable_loop_skips_normalization_but_keeps_the_ceiling`:** a 0.2 s hot cut. The
  warning is the exact "loudness: skipped normalization … still applies", with no
  `SILENCE_WARNING`, `gain_db < 0` and `output_true_peak_db <= -1.0 + 1e-5`.
- **`mp3_warns_about_loop_gaps_only_for_looped_cuts`:** an MP3 encoder and the stub. A cut with
  a range has the warning (after the setup warnings); a one-shot cut has none.
- **`each_caller_passes_its_flush_and_range`:**
  - `Limited` + `NoGap` + `whole` records `[(n, None, NoGap)]`.
  - `Limited` + `Gap` records `[(n, None, Gap)]`.
  - Static intro-loop cuts record `[(.., None, Gap), (.., Some(0..body), Gap)]`.
  - Opus records the resampled range, and the file's `loop_points` has `sample_rate: 48_000`.
- **`limited_finishes_exactly_one_cut`:** two cuts give the exact error.
- **`progress_precedes_the_last_decode`** (Limited only): `fail_decode_at: Some(0)` on the one
  cut gives `Err`, and the events were `[Encoded{1}]`.
- **`static_progress_follows_the_whole_pass`** (RepeatSafeStatic, ogg):
  - a one-cut loop with `fail_decode_at: Some(0)` (the only cut) gives `Err`, and the events
    were `[]`;
  - intro-loop cuts with `fail_decode_at: Some(1)` (cut 2 of pass 1) give `Err` and `[]`;
  - a one-cut loop with boosts `[1.0, 0.0]` (pass 1 overshoots) and `fail_decode_at: Some(1)`
    (pass 2) gives `Err` and `[Encoded{1}]`.

**`tests/encode.rs`:** move `encode_loop_corrects_a_lossy_overshoot_within_two_passes` and
`encode_loop_reports_but_never_corrects_with_loudness_off` off `encode::encode`. The `encode_loop`
helper becomes `strudeland_render::finish(&testing::audio(input.to_vec(), SR), &OutputOptions {
loudness, encoder }, &|p| ..)`. Read `out.summary.loudness.unwrap()`, `out.summary.warnings` and
`out.bytes`. The assertions are unchanged.

## Validation

- **Targeted:** `cargo test -p strudeland-render finish::` and `--test encode`.
- **The stage gates,** logged once. `characterization` passes, and the golden is untouched.
- **`rg -n "finish_file|finish_loop|encode_with|encode_standalone|encode::encode\b|struct Encoded|struct Part" crates/`**
  finds nothing.
- **`rg -n "decoded_true_peak_db\(" crates/strudeland-render/src`** shows only `encode/verify.rs`,
  `Real` and the stub.

## Commit message

```
refactor(render): finish every render through one loop

Plain renders, one-shots and loop presets now level, encode, verify and
correct their file cuts through one finishing loop and the codec seam.
An explicit leveling switch keeps the two rules: a limited file lowers
its limiter ceiling on a lossy overshoot, and a loop preset lowers the
static gain its files share. The loop correction now has forced-
overshoot tests. Bytes, warnings, summaries and progress are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
