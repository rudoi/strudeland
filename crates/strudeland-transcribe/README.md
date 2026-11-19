# strudeland-transcribe

Melody transcription for a monophonic voice: a sung or hummed take in, notes quantized to a bar
grid and Strudel mini-notation out. A pure library (no Tauri, V8, audio I/O, threads, global state
or `unsafe`); `strudeland transcribe` (`crates/strudeland-cli`) is its only caller. It takes samples
and a `GridHint`, never paths or a sidecar, so other callers (drum transcription, an app command)
can use it on any audio.

## Stages

`transcribe(samples, sample_rate, &GridHint, &Options)` runs, in order:

| Stage | Module | What it does |
|---|---|---|
| load | `load` | `load_wav_span` reads only the analysed span of a WAV (any PCM or float format, channels mixed to mono); `resample_to_analysis` resamples to 16 kHz with rubato's FFT resampler and trims its startup delay |
| pitch | `pitch` | `PitchTracker` and `PyinTracker`: pYIN (Mauch and Dixon 2014), 1024-sample frames, 10 ms hop, a 504-state voiced/unvoiced HMM over 20-cent bins, banded Viterbi, f0 refined to the nearest YIN candidate |
| tuning | `tuning` | the global offset in cents (positive: sharp), a voiced-probability-weighted circular mean over stable frames |
| onsets | `onset` | spectral flux (SuperFlux maximum filter, decay gate) plus RMS rise, adaptive peak picking |
| segment | `segment` | notes from voicing runs, sustained pitch changes (snapped back to their onset), glides merged into the note they reach, drifts rejoined, onset splits for repeated pitches at a loudness dip, short notes merged or dropped; confidence components |
| grid | `grid` | flags, then the sidecar, then inference: tempo from the onset autocorrelation, phase from onset alignment |
| quantize | `quantize` | onsets and offsets snapped to the `--grid` steps, overlaps cut, collisions dropped, barline crossings split and re-attacked, snap errors and the timing spread |
| suggest | `suggest` | a `--grid` division the rhythm fits clearly better (onset gaps against whole steps), and a `--shift` the notes share within a step; warnings and `grid.suggested*` only |
| key | `key` | Krumhansl–Schmuckler over the 24 major and minor keys, with downbeat and final-note bonuses; or `--key` |
| notation | `notation` | `note("…")` names and `n("…").scale("…")` degrees, one cycle per bar from the bar the span starts in (`firstBar`), or from bar 0 when it has a pickup |
| report | `report` | `Transcription` and every serde type (camelCase, nulls kept) |

`eval` is test support, in the library so the tests, the example and other crates share it:
`synth` (a seeded generator of tonal voice-like melodies with known notes), `score` (COnP and the
exact-notation check) and `cases` (the 20-case evaluation set and its runner). It is behind the
`eval` feature (off by default, so the shipped `strudeland` binary does not carry it) and always
compiled for the crate's own unit tests. `tests/eval.rs`, `tests/perf_budget.rs` and
`examples/eval.rs` require it; `strudeland-cli` turns it on in its dev-dependency, so `cargo test
--workspace` runs the evaluation tiers too.

## Evaluation

```text
cargo run --release -p strudeland-transcribe --features eval --example eval
cargo test -p strudeland-transcribe --features eval --test eval
```

25 seeded cases (seeds 1000 to 1024), each 8 to 16 bars of a tonal melody of 4 to 8 notes per bar
in a random major or minor key, 90 to 130 bpm in 4/4 (12/8 in three hummed cases), rendered at 48 kHz so the resampler is in the
loop. The grid comes from the case (as a sidecar gives it) except in the no-grid tier. Scores are
COnP (onset within 50 ms, pitch within 50 cents, one to one), micro-averaged per tier. "Exact
notation" is string equality with the generator's own notation rendered with the transcription's
key. Key accuracy compares the detected key with the generator's: exact, related (relative,
parallel, dominant or subdominant), or none (the detector declined: fewer than 6 notes or the best
two keys too close). No case detected a wrong key.

| Tier | Synth settings |
|---|---|
| clean | harmonic voice with formants, no vibrato, no noise, no detune, exact grid timing |
| vibrato + glide | +-50 cents at 5.5 Hz, 60 ms attack glides, 60 ms portamento, repeated notes |
| noisy | breath noise at -30 dB, a noise floor at -45 dB, metronome click bleed at -35 dB |
| detuned + jittered | a global detune uniform within +-35 cents, onset jitter uniform within +-20 ms |
| no-grid inference | the clean voice, an empty hint: tempo and phase inferred |
| hummed | a low voice (G2 to E4) with 60 ms attacks and 30 ms releases, a loudness wobble of +-1.5 to 3 dB (three sinusoids at 2 to 8 Hz), a pitch drift within each note of +-30 to 40 cents, repeated notes, a detune within +-20 cents and onset jitter within +-15 ms; seeds 1021, 1023 and 1024 in 12/8, 1022 and 1023 with a pickup in bar 0 |

Measured on 2026-11-14 (the example's output):

| tier | cases | precision | recall | F1 | exact notation | key exact | key related | key none |
|---|---|---|---|---|---|---|---|---|
| clean | 5 | 1.000 | 1.000 | 1.000 | 5/5 | 3/5 | 0/5 | 2/5 |
| vibrato + glide | 4 | 0.997 | 0.997 | 0.997 | 4/4 | 4/4 | 0/4 | 0/4 |
| noisy | 4 | 1.000 | 1.000 | 1.000 | 4/4 | 4/4 | 0/4 | 0/4 |
| detuned + jittered | 4 | 1.000 | 1.000 | 1.000 | 4/4 | 4/4 | 0/4 | 0/4 |
| no-grid inference | 3 | 1.000 | 1.000 | 1.000 | 3/3 | 3/3 | 0/3 | 0/3 |
| hummed | 5 | 0.997 | 1.000 | 0.998 | 4/5 | 3/5 | 0/5 | 2/5 |

| tier | seed | tp | fp | fn | F1 | exact | key (truth) | key (detected) | tempo error | phase error |
|---|---|---|---|---|---|---|---|---|---|---|
| clean | 1000 | 64 | 0 | 0 | 1.000 | yes | E minor | E minor (exact) | - | - |
| clean | 1001 | 86 | 0 | 0 | 1.000 | yes | E major | none (none) | - | - |
| clean | 1002 | 83 | 0 | 0 | 1.000 | yes | A major | none (none) | - | - |
| clean | 1003 | 56 | 0 | 0 | 1.000 | yes | E major | E major (exact) | - | - |
| clean | 1004 | 59 | 0 | 0 | 1.000 | yes | D minor | D minor (exact) | - | - |
| vibrato + glide | 1005 | 74 | 0 | 0 | 1.000 | yes | G minor | G minor (exact) | - | - |
| vibrato + glide | 1006 | 69 | 0 | 0 | 1.000 | yes | F minor | F minor (exact) | - | - |
| vibrato + glide | 1007 | 93 | 1 | 1 | 0.989 | yes | C# minor | C# minor (exact) | - | - |
| vibrato + glide | 1008 | 94 | 0 | 0 | 1.000 | yes | E minor | E minor (exact) | - | - |
| noisy | 1009 | 89 | 0 | 0 | 1.000 | yes | F# minor | F# minor (exact) | - | - |
| noisy | 1010 | 72 | 0 | 0 | 1.000 | yes | C# major | C# major (exact) | - | - |
| noisy | 1011 | 56 | 0 | 0 | 1.000 | yes | D minor | D minor (exact) | - | - |
| noisy | 1012 | 82 | 0 | 0 | 1.000 | yes | C major | C major (exact) | - | - |
| detuned + jittered | 1013 | 62 | 0 | 0 | 1.000 | yes | E minor | E minor (exact) | - | - |
| detuned + jittered | 1014 | 57 | 0 | 0 | 1.000 | yes | G minor | G minor (exact) | - | - |
| detuned + jittered | 1015 | 95 | 0 | 0 | 1.000 | yes | G major | G major (exact) | - | - |
| detuned + jittered | 1016 | 58 | 0 | 0 | 1.000 | yes | D major | D major (exact) | - | - |
| no-grid inference | 1017 | 51 | 0 | 0 | 1.000 | yes | B major | B major (exact) | -0.02% | +6.5 ms |
| no-grid inference | 1018 | 82 | 0 | 0 | 1.000 | yes | G# minor | G# minor (exact) | +0.00% | +8.1 ms |
| no-grid inference | 1019 | 51 | 0 | 0 | 1.000 | yes | C# major | C# major (exact) | +0.04% | +14.7 ms |
| hummed | 1020 | 68 | 0 | 0 | 1.000 | yes | C# minor | none (none) | - | - |
| hummed | 1021 | 68 | 0 | 0 | 1.000 | yes | B major | none (none) | - | - |
| hummed | 1022 | 59 | 0 | 0 | 1.000 | yes | D major | D major (exact) | - | - |
| hummed | 1023 | 75 | 0 | 0 | 1.000 | yes | B minor | B minor (exact) | - | - |
| hummed | 1024 | 57 | 1 | 0 | 0.991 | no | G# major | G# major (exact) | - | - |

Pins in `tests/eval.rs` (measured minus 0.03, rounded down to a multiple of 0.01; never lowered to
absorb a defect):

| Pin | Measured | Pinned |
|---|---|---|
| `MIN_F1_CLEAN` (and exact notation on every clean case) | 1.000 | 0.97 |
| `MIN_F1_VIBRATO_GLIDE` | 0.997 | 0.96 |
| `MIN_F1_NOISY` | 1.000 | 0.97 |
| `MIN_F1_DETUNED_JITTERED` | 1.000 | 0.97 |
| `MIN_F1_NO_GRID` (and tempo within 2%, phase within 50 ms modulo one beat, per case) | 1.000 | 0.97 |
| `MIN_F1_HUMMED` (and a pickup case's notation starts at bar 0) | 0.998 | 0.96 |
| `MIN_KEY_ACCURACY_CLEAN` (exact keys over clean cases) | 0.60 | 0.57 |

As a check against tuning to the pinned seeds, 30 held-out seeds per tier (3000 to 3029, not
committed as a test) measured: clean, noisy, detuned and no-grid F1 1.000 with every notation
exact; vibrato + glide F1 0.985 (0.913 before the segmentation changes below), 13 of 30 exact;
exact keys 25 to 29 of 30 per tier.

### Performance

```text
cargo test --release -p strudeland-transcribe --features eval --test perf_budget -- --ignored --nocapture
```

A minute of the vibrato tier's voice at 48 kHz through `transcribe`, the median of three runs,
must stay under `PERF_BUDGET_SECONDS_PER_MINUTE` (1.0 s). Measured on 2026-11-14 (Apple silicon,
release): 0.155 s per minute; after the hummed-memo changes 0.238 s (the code before them measured 0.192 s on the same machine that day). The debug-build `tests/eval.rs` takes about 16 s (the workspace
builds dependencies at `opt-level = 3`), so it needs no profile override.

## Tuning log

Every provisional constant changed during evaluation, and the generator and logic changes made
before pinning. Each change was re-measured on all tiers; none lowered another tier.

| Change | Old | New | Tiers affected | Why |
|---|---|---|---|---|
| synth melodies (`eval::synth`) | random walk | tonal: start and end on the tonic, four-bar I, IV/vi/ii, V, I phrases, chord tones on strong beats | key accuracy, all tiers | the random walk did not behave like a melody, so even a correct detector found the dominant or relative key; the key is still a random draw the detector never sees |
| `KEY_DOWNBEAT_BONUS` (new) | - | 1.0 | key accuracy, all tiers | metre as a key cue (Temperley 2001): notes on downbeats weigh more. Exact keys over the 20 cases: 5 before both bonuses and the generator change, 18 after (and after the segmentation changes) |
| `KEY_FINAL_BONUS` (new) | - | 6.0 | key accuracy, all tiers | melodies end on their tonic. In mean note weights, so it weighs the same in short and long melodies (a multiplier made one held tonic swamp a short melody's mode). Over 1000 generated melodies' true notes: exact 0.62 to 0.92, wrong under 0.01, declined 0.25 to 0.07 |
| `PITCH_SNAP_MS` (new) | - | 80 ms | vibrato + glide 0.901 to 0.970 | a pitch change is seen only after it has held, 50 to 70 ms after the attack under vibrato or portamento; boundaries now move back to the onset |
| `GLIDE_MAX_MS` (new) | - | 100 ms | vibrato + glide 0.970 to 0.997 | an attack glide or portamento cut off by a pitch change came out as a short note a semitone out; it is merged into the note it reaches |
| `GLIDE_INSIDE` (new) | - | 0.1 | vibrato + glide | a glide's median lies inside its travel, so a held note whose end a late pitch change left in its piece is not mistaken for one (0.2 measured 0.992, 0.1 measured 0.997) |
| `PHASE_DOWNBEAT_WEIGHT` | 1.0 | 3.0 | no-grid 0.992 to 1.000 | with the tonal melodies, seed 1019's phase locked a sixteenth late (-108 ms): attacks after silence are detected a few ms early and legato ones 10 to 15 ms late, so the beat term alone favoured the wrong sixteenth; 2.0 fixed 100 held-out seeds but not 1019, 3.0 both |
| hummed tier's loudness wobble (`HUM_WOBBLE_DB`, `eval::cases`) | +-3 to 6 dB | +-1.5 to 3 dB (a swing of 3 to 6 dB) | hummed | at +-3 to 6 dB the generated wobble fell to 0.4 to 0.6 of the level within 60 ms, deeper than any re-articulation of the synth's other tiers and than the real memo's wobble, which swung 1 to 3 dB either way within held notes (0.76 or more of the level within 30 ms) |
| re-articulation rule (new: `REARTICULATION_WIN` 320, `REARTICULATION_DIP` 0.77, `REARTICULATION_LOOK_MS` 60, `REARTICULATION_LEVEL_MS` 30, `REARTICULATION_SLOW_DIP` 0.5, `REARTICULATION_SLOW_MS` 150) | every onset at or above `ONSET_SPLIT_MIN` split a held pitch | an onset splits a held pitch only at a loudness dip | hummed 0.929 to 0.998 (precision 0.867 to 0.997, exact notation 0/5 to 4/5); the other tiers unchanged | motivated by the first real memo (a hummed take over headphones): loudness wobble inside held notes gave onset peaks of 0.27 to 0.57 and re-attacked them (`c3 c3 c3`). On a 20 ms RMS, the synth's repeated notes (10 ms ramps) dip to 0.59 to 0.74 of the level within 30 ms and the hummed ones to 0.13 to 0.39, while the hummed tier's wobble stays at 0.79 or more; 0.77 sits between. In the memo the wobble measured 0.76 to 1.13 fast, and one re-sung `b2 b2` only showed over 150 ms (0.41), hence the slow rule; the wobble there stayed above 0.5 over 150 ms. The 32 ms onset window smoothed the synth's dips to 0.75 to 0.83, too close to the wobble |
| drift rejoin (new: `DRIFT_MERGE_ST` 1.0, `DRIFT_MAX_MS` 200) | a pitch-change split stood unless the pieces rounded alike or lay within `PITCH_CHANGE_ST` | pieces within a semitone, the shorter under 200 ms, rejoin when the boundary has no loudness dip | none (every synth note starts from a gain ramp, so each boundary dips; all tiers unchanged) | motivated by the first real memo only: a hummed pitch drifted 0.8 to 1.0 semitone at the ends of notes and in the pickup's slow glide (`e3 f3`, `g#2 a2`), and the real note changes there all dipped. Unmeasured against a voice that slurs a semitone legato without a dip, which it would merge |
| drift veto (new: `DRIFT_ONSET_VETO` 0.6; `DRIFT_MERGE_ST` now exclusive, still 1.0) | pieces within a semitone (inclusive) rejoined whenever the boundary had no loudness dip | pieces less than a semitone apart rejoin only without a dip and without an onset peak of 0.6 or more within a frame of the boundary | none (all tiers unchanged; the second real memo's F1 unchanged, one `a#2` there now `a2`) | a legato half-step without a loudness dip (E4 to a 125 ms F4, measured 0.8 to 1.2 semitones) was rejoined as one note. A lower `DRIFT_MERGE_ST` cannot separate the two: the second real memo's ten drift rejoins were 0.79 to 0.99 semitones apart (pieces within `PITCH_CHANGE_ST`, 0.6, are already joined), and 0.95, 0.9 and 0.75 each lost real drifts (COnP F1 at 50/100 ms 0.493/0.725 to 0.457/0.686, 0.451/0.676 and 0.432/0.622). An onset peak tells them apart: three of those drift boundaries had peaks at the boundary (0.34, 0.36, 0.54; the first memo's wobble peaked at 0.27 to 0.57), so a veto at `ONSET_SPLIT_MIN` (0.25) lost them (0.444/0.667), while a synth legato change attacked over 10 ms peaks at 0.67 and a re-sung one at 0.93 or more. A slur with no attack at all is still merged |
| `AMBIGUOUS_CENTS` (new) | - | 30 | none (a report field; notes and notation unchanged) | motivated by the first real memo: its singer's intonation varied by up to +-50 cents per note, and a c#3 read as c3 + 31 cents after tuning; a note past 30 cents now names the neighbour it leans toward |
| grid and shift suggestions (new: `GRID_FIT_MIN_PAIRS` 8, `GRID_MISFIT` 0.32, `GRID_SUGGEST_RATIO` 0.75, `SHIFT_MIN_CONSISTENCY` 0.6, `SHIFT_MIN_NOTES` 8, `SHIFT_MIN_MS` 20, `SHIFT_SUGGEST_RATIO` 0.7) | - | suggestions only | none (no case of any tier gets a suggestion; notes and notation unchanged) | motivated by the first real memo: dotted eighths in 12/8 at `--grid 8` measure a rhythm fit of 0.357 (worse than chance, 0.25) and 0.195 at `--grid 16`, so 16 is suggested. Onsets at random (gaps of 0.2 to 0.6 s, 20 seeds) measured 0.20 to 0.37 per division, so the misfit gate sits above their eighth-note fits (at most 0.28) and the absolute comparison alone (best under 0.75 of the requested) suggested a division for random onsets. The memo's notes show no shared offset within a sixteenth (phase consistency 0.21), so no shift is suggested for it: its downbeat sits 1.8 sixteenths after the sidecar's, a placement this search cannot see |

## Constants

Every constant is in [`src/consts.rs`](src/consts.rs) with a one-line source. "Provisional" values
were chosen for synthetic audio and are retuned only against real memos.

| Constant | Value | Provisional |
|---|---|---|
| `ANALYSIS_RATE` | `16_000` |  |
| `MAX_WHOLE_SECONDS` | `600.0` |  |
| `FRAME` | `1024` |  |
| `HOP` | `160` |  |
| `FMIN` | `60.0` |  |
| `FMAX` | `1100.0` |  |
| `BIN_CENTS` | `20.0` |  |
| `N_BINS` | `252` |  |
| `BETA_A` | `2.0` |  |
| `BETA_B` | `18.0` |  |
| `N_THRESHOLDS` | `100` |  |
| `MAX_TRANSITION_OCT_PER_SEC` | `35.0` |  |
| `VOICING_SWITCH` | `0.01` |  |
| `NO_TROUGH_PROB` | `0.01` |  |
| `UNVOICED_EMISSION_BINS` | `N_BINS` | yes |
| `MIN_VOICED_PROB_EMIT` | `0.5` | yes |
| `TUNING_SKIP_MS` | `30` |  |
| `TUNING_STABLE_ST` | `0.3` |  |
| `TUNING_STABLE_MS` | `50` |  |
| `TUNING_CENTRE_MS` | `200` | yes |
| `TUNING_WARN_CENTS` | `40.0` |  |
| `MEDIAN_FRAMES` | `5` | yes |
| `GAP_BRIDGE_MS` | `40` |  |
| `PITCH_CHANGE_ST` | `0.6` |  |
| `PITCH_CHANGE_MS` | `50` |  |
| `PITCH_SNAP_MS` | `80` | yes |
| `GLIDE_MAX_MS` | `100` | yes |
| `GLIDE_INSIDE` | `0.1` | yes |
| `DRIFT_MERGE_ST` | `1.0` | yes |
| `DRIFT_MAX_MS` | `200` | yes |
| `DRIFT_ONSET_VETO` | `0.6` | yes |
| `MIN_NOTE_MS` | `60` |  |
| `ONSET_WIN` | `512` | yes |
| `ONSET_HOP` | `HOP` | yes |
| `ONSET_BAND_HZ` | `(80.0, 4000.0)` | yes |
| `ONSET_LOG_GAIN` | `100.0` | yes |
| `ONSET_MAX_FILTER_BINS` | `1` | yes |
| `ONSET_DECAY_LOOK_MS` | `20` | yes |
| `ONSET_RMS_WEIGHT` | `0.5` | yes |
| `ONSET_NORM_PERCENTILE` | `0.95` | yes |
| `ONSET_NORM_CLAMP` | `1.5` | yes |
| `ONSET_NORM_FLOOR` | `0.2` | yes |
| `ONSET_THRESH_WINDOW_MS` | `250` | yes |
| `ONSET_THRESH_DELTA` | `0.12` | yes |
| `ONSET_MIN_DISTANCE_MS` | `50` | yes |
| `ONSET_SPLIT_MIN` | `0.25` | yes |
| `REARTICULATION_WIN` | `320` | yes |
| `REARTICULATION_DIP` | `0.77` | yes |
| `REARTICULATION_LOOK_MS` | `60` | yes |
| `REARTICULATION_LEVEL_MS` | `30` | yes |
| `REARTICULATION_SLOW_DIP` | `0.5` | yes |
| `REARTICULATION_SLOW_MS` | `150` | yes |
| `PHASE_WINDOW_MS` | `25` |  |
| `PHASE_STEP_MS` | `5` | yes |
| `TEMPO_MIN` | `60.0` |  |
| `TEMPO_MAX` | `180.0` |  |
| `TEMPO_AGREE` | `0.04` |  |
| `PHASE_DOWNBEAT_WEIGHT` | `3.0` | yes |
| `PHASE_MAX_CANDIDATES` | `20_000` |  |
| `MIN_ONSETS` | `4` |  |
| `STRONG_ONSET` | `0.5` | yes |
| `TEMPO_SMOOTH_MS` | `20` | yes |
| `TEMPO_SUBDIV_WEIGHTS` | `[(0.25, 0.5), (0.5, 0.5), (0.75, 0.5)]` | yes |
| `TEMPO_REFINE_BEATS` | `[4, 16]` | yes |
| `TEMPO_PRIOR_BPM` | `120.0` | yes |
| `TEMPO_PRIOR_OCTAVES` | `0.7` | yes |
| `MAX_NOTATION_BARS` | `100_000` |  |
| `KEY_MIN_NOTES` | `6` |  |
| `KEY_DOWNBEAT_BONUS` | `1.0` | yes |
| `KEY_FINAL_BONUS` | `6.0` | yes |
| `KEY_MARGIN_MIN` | `0.05` | yes |
| `AMBIGUOUS_CENTS` | `30.0` | yes |
| `LOW_CONFIDENCE` | `0.5` | yes |
| `SNAP_PENALTY_MS` | `50.0` | yes |
| `TIMING_SPREAD_WARN_MS` | `60.0` | yes |
| `GRID_FIT_MIN_PAIRS` | `8` | yes |
| `GRID_MISFIT` | `0.32` | yes |
| `GRID_SUGGEST_RATIO` | `0.75` | yes |
| `SHIFT_SUGGEST_RATIO` | `0.7` | yes |
| `SHIFT_MIN_CONSISTENCY` | `0.6` | yes |
| `SHIFT_MIN_NOTES` | `8` | yes |
| `SHIFT_MIN_MS` | `20.0` | yes |
| `CONF_W_VOICED` | `0.4` | yes |
| `CONF_W_STABILITY` | `0.3` | yes |
| `CONF_W_FIT` | `0.3` | yes |
| `STABILITY_FULL_ST` | `0.5` | yes |
| `PERF_BUDGET_SECONDS_PER_MINUTE` | `1.0` |  |

## Reading `--debug`

`strudeland transcribe NAME --json --debug` adds two arrays to the JSON:

- `frames`: one per 10 ms pitch frame, `{t, f0, voiced, rms}`. `t` is seconds from the start of
  the analysed span (add the span's start for memo time), `f0` the refined fundamental in Hz or
  null when the decoded state is unvoiced, `voiced` pYIN's voiced probability, `rms` the frame's
  RMS. Notes come from voiced runs, so a note that is missing usually shows as `f0: null` frames
  or a `voiced` probability under `MIN_VOICED_PROB_EMIT`; a note a semitone out shows as f0 frames
  near the wrong semitone (compare `12 * log2(f0 / 440) + 69` with the note's `midi` and
  `tuning.cents`).
- `onsets`: the picked onset peaks, `{seconds, strength}`, seconds from the start of the analysed
  span, strength 0..1. A peak at or above `ONSET_SPLIT_MIN` splits a held pitch (repeated notes)
  and anchors a pitch change; inferred grids align their beats with these peaks. Two repeated
  notes heard as one usually lack a peak between them; a split note has an extra one.

Each note's `components` (`voiced`, `stability`, `fit`, `onset`) show why its confidence is low,
and `snapErrorMs` / `timingSpread` show a wrong grid or phase.

## Decisions

- **Fresh pYIN.** Implemented from the paper with librosa's published defaults as constants (beta
  prior 2/18, 100 thresholds, 35 octaves per second, voicing switch 0.01, `no_trough_prob` 0.01);
  no librosa code is ported, so there is no licence file and no `// Port of` marker.
- **No librosa vectors.** No Python in the repo; synthetic ground truth checks the tracker.
- **Monophonic only.** One voice, one note at a time.
- **Synthetic evaluation.** Every test generates its audio from a seed; no audio file is
  committed.

## Known differences and limits

- **Synthetic-only evaluation.** The tiers are generated voices (the hummed tier imitates the
  first real memo's loudness wobble and drift, measured from it, but the memo is not committed); real hummed or sung memos are
  messier (breathy onsets, scoops, consonants, room noise). The provisional constants need
  evaluation on real memos.
- **pYIN only.** Evaluate any alternative pitch tracker on this set before replacing pYIN.
- **One tempo per take.** No tempo changes or rubato inside a memo.
- **Inferred downbeat.** Phase inference finds the beat grid, but bar 1 is the beat of the first
  strong onset: a melody with a pickup gets its bars shifted. The warning says so and the skill
  asks the user (or pass `--bar1`).
- **20-cent bins, refined.** The HMM decodes 20-cent bins; each voiced frame's f0 is the
  interpolated YIN candidate nearest the decoded bin (the bin centre when none is within one
  bin), so the output is not limited to the bin grid.
- **Resampler delay trim.** rubato's FFT resampler delays its output; `resample_to_analysis` trims
  that startup delay so analysis times match memo times, and a 16 kHz source skips the resampler.
- **Key detection declines** when the best two keys are within `KEY_MARGIN_MIN` (2 of the 5 clean
  cases): the scale-degree form is then omitted with a warning, and `--key` sets it.
- **Very short glided notes.** Under vibrato and portamento a note of one sixteenth at a fast
  tempo (under `GLIDE_MAX_MS` once detected) that glides into the next note can be merged into
  it.
