# 0118 re-verification (step 0)

Checked on `llm/andrew/strudeland/0118-one-finishing-path` at 77e210f (main after #117). All
paths are under `crates/strudeland-render/` unless stated.

## Confirmed

### Three finishing paths

- **Standard:**
  - `finish` (`src/lib.rs:277-283`) calls `finish_file(.., false)` (`:285-316`).
  - That selects `encode::encode` (`:293-297`), which is `encode_with(&Real::Legacy, ..)`
    (`src/encode/mod.rs:112-120`).
  - `Real::Legacy` encodes with `encode_once` (`:93`). For MP3, `encode_once` calls
    `mp3::encode_mp3` (`:20-22`), which uses `FlushNoGap` with no loop (`src/encode/mp3.rs:39-46`).
- **One-shot:**
  - `render_export` (`src/loop_export.rs:60-67`) calls `trim_silent_post_roll` (`:208-233`),
    then `finish_file(.., true)`.
  - That is `encode_standalone` → `encode_with(&Real::Standalone)` (`src/encode/mod.rs:122-137`).
  - `Real::Standalone` calls `encode_once_with_loop(.., None)` (`:94`). For MP3 this is
    `encode_mp3_with_loop` with `FlushGap` (`src/encode/mp3.rs:50-59`).
- **Loop presets:**
  - The arrangement is `src/loop_export.rs:69-147`: `copy_opening` `:157-170`, `fold_tail`
    `:235-251`, `join_opening` `:174-206`, `wrap_peak` `:255-270`, and `Part` `:272-277`.
  - Then `finish_loop` (`:279-439`) runs.

### Only MP3 bytes differ between `Legacy` and `Standalone`

- **WAV:** `encode_wav_with_loop(None)` returns `encode_wav` (`src/encode/wav.rs:95-97`).
- **Ogg:** `encode_ogg` is `encode_ogg_with_loop(.., None)` (`src/encode/ogg.rs:23-30`).
- **Opus:** `encode_opus` is `encode_opus_with_loop(.., None)` (`src/encode/opus.rs:109-116`).

### The limited correction loop (`encode_with`, `src/encode/mod.rs:139-210`)

- **Setup:** `prepare` runs once (`:155`), and every pass truncates the warnings to `base`
  (`:153`, `:158`).
- **Progress:** `Progress::Encoded { pass }` is emitted after the encode, before the decode
  (`:165`).
- **WAV** returns before decoding (`:168-176`). **Loudness off** returns after the decode with no
  correction (`:183-193`).
- **The correction** is `trim -= dp - ceiling + CORRECTION_MARGIN_DB` (`:207`).
- **The limits:** `MAX_ENCODES = 3` (`:50`) and `CORRECTION_MARGIN_DB = 0.1` (`:53`).
- **The exhausted warning** is at `:195-197`.

### The loop correction loop (`finish_loop`)

- **There is no limiter.** One `gain_db` is shared (`src/loop_export.rs:305-331`), capped by
  `true_peak_db - program_peak`. `program_peak` folds the splice peak (`:110`) with each file's
  own peak (`:301-303`).
- **The gain** is `10f64.powf(gain_db / 20.0)`, applied as `(f64::from(x) * gain) as f32` and
  skipped at exactly 0 dB (`:342-352`). This is the same expression as `level::linear_gain`
  (`src/level/mod.rs:248-250`) and `limiter::apply_one` (`src/level/limiter.rs:26-28`).
- **The codec seam is bypassed:** `encode_once_with_loop` (`src/loop_export.rs:381`) and
  `verify::decoded_true_peak_db` (`:385`) are called directly. The private `Codec` trait is
  `src/encode/mod.rs:68-77`.
- **Opus loop points** are resampled per end with `opus::resampled_len` at 48 kHz
  (`src/loop_export.rs:361-380`).
- **Progress:** `Encoded { pass }` is emitted once per pass, after every file's encode and
  decode and before the correction check (`:419`). An encode or decode error on any file in
  pass `p` returns with only `Encoded{1..p-1}` emitted.
- **The correction** is `gain_db -= worst_peak - ceiling + CORRECTION_MARGIN_DB` (`:424`). The
  exhausted warning uses the worst peak (`:427`).
- **The loudness summary** has `limiter_ceiling_dbtp: None` and `limiter_samples: 0`. Window,
  frames and seconds are per file (`:391-412`).

### Warning texts that differ

| Case | Limited | Loop presets |
|---|---|---|
| Loudness off, hot | "… the file will clip" (`src/level/mod.rs:112-116`) | "… the file may clip" (`src/loop_export.rs:307-309`) |
| Unmeasurable | `SILENCE_WARNING` (`src/level/mod.rs:26-27`, `:124`) | "loudness: skipped normalization because integrated LUFS is unmeasurable; the true-peak ceiling still applies" (`src/loop_export.rs:319-321`) |
| Limiter | "limiter: true peak is …" (`src/level/mod.rs:221-226`) | none |
| MP3 | none | "mp3 encoder delay and padding …" (`src/loop_export.rs:332-334`) |

### Planning and rendering are one function

- **`render_audio_for` (`src/lib.rs:92-247`) calls `plan::plan` (`:104`).** Then come
  `Progress::Planned` (`:111`), `plan::take_jobs` (refusal) (`:114`), the opening render
  (`:150-171`) and the isolated body (`:172-206`).
- **`plan::plan` is the only V8 caller.** `Host::new` appears only at `src/plan.rs:30`.
- **The types round-trip.** `PlannedRender`, `PlannedExport`, `PlannedRepeat` and `ChunkWindow`
  are `Serialize + Deserialize + Clone` (`crates/strudeland-host/src/planned.rs:11-95`), so tests
  can hand-build them.
- **The chunk render closure** is injected into `render::render_chunks` already
  (`src/render.rs:63-76`). Only `render_window`'s caller hard-codes the engine (`src/lib.rs:166-169`,
  `:199`).
- **The stem budget** is checked before rendering (`src/lib.rs:142-148`). `render_stems` enters
  through `render_audio_for` (`src/stems.rs:319-320`).
- **Unsupported signatures** come from haps, not from bare values
  (`crates/strudeland-engine/src/controls.rs:1816`).

### Public surface that must hold

- **The entry points:** `render`, `render_audio`, `finish` (`src/lib.rs:57-72`, `:277`), and
  `render_export` with `ExportFile`, `ExportRole` and `LoopPoints` (`src/lib.rs:43`).
- **`export::RealPipeline`** calls `crate::render`, `crate::render_export` and
  `stems::render_stems` (`src/export/pipeline.rs:49-79`).
- **No caller outside the crate** uses `encode::encode`, `encode_once`, `encode_mp3` or
  `encode_mp3_with_loop`. The CLI and app import only top-level items (`rg` over
  `crates/strudeland-cli`, `src-tauri`). The in-crate test users are:
  - `tests/encode.rs`: the imports `:6` and `:11`; `encode_once` at `:228`, `:235`, `:247`,
    `:407`, `:415`, `:420`, `:431`, `:472`, `:567`, `:576`, `:581`, `:784` and `:834`;
    `encode_mp3_with_loop` at `:273-274`; and `:739-758` (the `encode_loop` helper over
    `encode::encode`, returning `encode::Encoded`);
  - `tests/pipeline.rs:9` and `:551`;
  - `bench.rs:349` and `:370` (`finish`).
- **`src-tauri/src/instruments.rs:126-146`** calls `render_audio`, then `encode_wav` itself. It
  never calls `finish`.

### Docs that state only the limiter rule

- **`README.md:124-133`** (`### The lossy correction loop`). The loop paragraph is at
  `:108-111`. "Pipeline and modules" steps 4 and 5 (`:24-32`) put the correction loop in
  `encode/`, and `## Determinism` is at `:144`.
- **`AGENTS.md:251-252`** (Layout) lists "the true-peak correction loop" under `encode/`.
- **`AGENTS.md:397-399`:** "corrected by lowering the limiter ceiling (at most twice, then a
  warning); never loosen that".
- **`src/lib.rs:273-276`** names `encode::encode`.

### No test pins finished bytes across a change

- **`tests/pipeline.rs:96-99`** compares two `finish` calls in one run.
- **`tests/joiner_vectors.rs`** pins joined PCM (FNV-1a), not finished files.
- **There is no golden** of encoded output in the crate (`rg -i golden|sha256`).

### Glossary

- **"part"** is to be avoided for roles and sections (`GLOSSARY.md`, Role and Section).
- **"arrangement" and "layout"** are to be avoided for the structure (`GLOSSARY.md:43`).

## Corrected from the research

- **The research said `Real::Legacy` and `Real::Standalone` differ.** In fact they differ only
  for MP3. The flush is the only byte-relevant switch.
- **Folded and tagged loops build the same cut** (opening plus folded repeat, range `end..end +
  body`). They differ only in role (`src/export/request.rs:12-18`) and in what the planner
  plans.
- **`summary.json` timings are wall-clock** (`src/summary.rs:26-35`). The golden must zero them.
- **Lossy encoder floats and the platform `pow`/`log10` can differ by OS and architecture.** The
  golden needs a reference platform, as `jsmath`'s `pow`/`tanh` have (`AGENTS.md` engine rule).
- **`sha2 = 0.10.9` is already in `Cargo.lock`** (`crates/strudeland-host/Cargo.toml:29`). Use it
  as an exact-pinned dev-dependency, which `tests/licence.rs` requires. The lock also holds
  `sha2 0.11.1`, so `strudeland-render` gains a `"sha2 0.10.9"` edge (no new package).
- **`profile.dev.package."*"` (opt-level 3) covers dependencies only** (`Cargo.toml:5-6`). The
  render crate itself is unoptimised in a debug build; only the encoders are optimised.
- **`bun run lint` is `prettier --check .`** (`package.json:33`), and `.prettierignore` already
  excludes the render crate's `tests/vectors/`, not a `tests/golden/`.
- **`testing::audio`'s `0..1` window** would make `splice` reject any loop start bar above 0
  ("loop start is outside the planned window", `src/loop_export.rs:75-77`), so loop fixtures
  override it.

## Conclusion

Worth doing as planned. The byte-identity hinges on four points:

- reusing the gain and correction expressions verbatim;
- passing the MP3 flush explicitly;
- the per-strategy warning bookkeeping;
- a golden judged on one reference platform.
