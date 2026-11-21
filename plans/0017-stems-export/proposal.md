# Task 17 — Stems export

Implemented in the CLI and desktop Export Audio dialog.

## Goal

Export a stereo reference mix and a set of stereo stems from the Rust renderer. Every file in the set starts at the same song position and has the same sample rate and frame count. A DAW can import the set at one position, and the unity-gain sum of the stems reproduces the reference mix within floating-point rounding.

## Scope

- Offer three grouping modes. **Orbit** groups by the engine's numeric orbit identity, including the default orbit. **Voice** groups musical parts by explicit Strudel `.label(...)` values, with a documented unlabeled group. **Instrument** groups by the resolved Rust sound name, including engine defaults and bank prefixing where supported. Voice refers to a reusable musical part. Successive notes and chord tones with the same label belong together.
- Make stem sets available from `strudeland render` and the desktop Export Audio dialog. Use the existing standard render range and project destination conventions. Include a mix and a manifest that maps filenames to group identities and records the selected range, sample rate, and frame count.
- Start with stereo 32-bit float WAV at the engine's existing 44.1 kHz. Use pre-master audio for the whole set, with unity gain and no loudness adjustment, true-peak limiting, or per-file normalization. Label the reference accordingly in the desktop dialog and CLI.
- Keep tagged-loop, intro-loop, folded-loop, one-shot, compressed outputs, and live capture outside this proposal. Existing mix exports retain their current options, mastering, and output behavior.

## Architecture

Reuse the headless host's standard `planRender` path in [job-plan.ts](../../src/lib/render/job-plan.ts) and the Rust pipeline's `render_audio_for` in [lib.rs](../../crates/strudeland-render/src/lib.rs). Plan and validate the ordinary mix jobs, then establish a render-wide stem catalog from their scheduled haps. Keep the engine job schema and existing browser parity routing intact.

Collect the mix and stems as outputs of one execution of each chunk's audio graph. [Core::render_quantum](../../crates/strudeland-engine/src/render.rs) already has the voices' direct outputs and their orbit assignments. [OrbitBank::process](../../crates/strudeland-engine/src/orbit.rs) produces the finished orbit signals immediately before merging them into the regular mix. Capture those signals by identity. The harness's numbered channel pairs wrap into a limited destination and cannot serve as export identities.

## Technical design

### Group identity and signal placement

- Use the same orbit lookup, sound preparation, and silence detection the engine uses for rendering. Read voice labels from the existing `label` control, which is already accepted by the engine. A label groups parts across chunks and orbits. Treat missing or empty labels as unlabeled and keep that group distinct from a user label with the same display text. Use typed group identities so a label or instrument named “FX” cannot collide with a shared-effects group.
- Build the catalog once across the checked jobs, including haps scheduled for lookback. This lets carry-in audio retain its group and gives each chunk the same layout, with zeros for absent groups. A group present only in lookback can produce a silent file. Exclude silence markers from the catalog while still rendering their scheduling effects, including ducking.
- For orbit grouping, capture the finished stereo output of each orbit, with its delay, reverb, orbit filter, and duck gain intact. Maintain the engine's existing merge order and route the reference mix through the existing merger.
- For voice and instrument grouping, sum the connected direct voice outputs by group within each orbit, with the existing stereo conversion and local voice effects. Apply the orbit's actual per-frame output gain and its fixed stereo panner to those grouped signals. Derive the shared-effects stem for that orbit as its finished stereo output minus its summed group contributions. That difference carries the shared delay and reverb, the orbit filter's change to the sound, and residual rounding. With a shared filter active, soloing the effects stem can include a subtraction of the direct signal. Document this in the export help. A bus reader's audible output belongs to the reader's group, and bus sends feed the same graph they do in the mix.
- Use the same quantum and pause schedule, voice construction order, random seeds, and sidechain inputs as a standard export. Implement capture as an opt-in path, keeping the live renderer and ordinary offline mix on their existing path. Test that enabling capture leaves the reference mix bit-identical to an ordinary pre-master render of the same checked jobs.

### Chunking and files

- Reuse [render.rs](../../crates/strudeland-render/src/render.rs) for bounded chunk workers and [Joiner](../../crates/strudeland-render/src/join.rs) for sample-exact placement. Generalize the stereo-only worker assertion for an explicit output layout. Use the same lookback, half-open window, and keep ranges for the mix and all stems. Fail early on excessive projected buffer or file sizes using checked arithmetic and a documented budget. Include worker buffers and encoding in the peak memory estimate, and encode one stereo file at a time.
- Write the set to a new directory in the chosen destination, using a name derived from the song and selected range by default. Use deterministic numbered stem names and store the original identities in the manifest so labels never become path components. Reuse the project crate's path scoping and atomic/no-clobber file operations. Reserve the set directory exclusively, report already-published paths on a later I/O failure, and write the manifest last so it identifies a completed set. Fail on an existing destination without replacing it.
- Add a grouping option to the CLI's [render command](../../crates/strudeland-cli/src/render.rs), and update its embedded skill in the same change. Make the desktop dialog preview the destination directory, and list the actual files after the render. Extend [export_audio.rs](../../src-tauri/src/export_audio.rs) and the frontend [export API](../../src/lib/export/api.ts) through the existing export progress and completion paths, including the quit guard. The selected source is the active editor buffer on desktop and the song file at the CLI.

## Dependencies

Keitora task 14 delivered the Rust render pipeline and is complete. The current tree also contains desktop export from task 95 and the application quit guard from task 99. The proposal depends on these existing paths and requires no new host, vendored Strudel, or codec dependency.

## Risks and acceptance

- **Signal correctness.** Test two labeled parts with the same instrument, one label across orbits, same-instrument grouping across labels, default/unlabeled haps, a ducking marker, and sparse, zero, and high-numbered orbits. Use fixtures with buses, orbit filter, seeded reverb, and delay across chunk boundaries. Check that the captured reference matches the standard pre-master mix, and assert reconstruction per sample with a tolerance tied to floating-point accumulation and the summed signal magnitudes.
- **DAW alignment.** Read the exported WAVs back and verify frame counts and sample rate. Confirm frame-zero alignment for nonzero range starts, one-sample boundary differences, a group that enters late, and multi-chunk renders. Check deterministic manifests and outputs between single-worker and parallel runs.
- **Resource and output limits.** Cover empty/silent renders, number of groups, memory and WAV size refusal, existing folders, label/path edge cases, and partial writes. Exercise CLI and desktop success, errors, progress, and quit while a stem set is being written.
- **User expectations.** Float WAV preserves peaks above full scale when exported without mastering. The unity-gain reference is the pre-master mix from the set. A separately mastered render can differ in level and dynamics.

## Open questions for refinement

1. Is `.label(...)` the right voice identity, with unlabeled material bundled together, or should authors have a dedicated stem label?
2. Is the pre-master float WAV set the right first release, or should exporting stems against a leveled mix and using the loop modes be part of task 17?
3. In voice and instrument mode, should shared effects be delivered per orbit as proposed, or grouped into a single stereo effects stem?

Recommended complexity is large. The change spans the engine's signal graph, the offline pipeline, and the two export interfaces. Keep fracture, singletonize, and loss check before sequencing.

## Refinement

The authorized automated pass kept all three grouping modes, the existing `.label(...)` control, and per-orbit shared effects. The set's reference is the same unmastered stereo mix as the normal pipeline produces before leveling. Float WAV carries values outside the integer full-scale range. Keep any sum comparison in float before playback or conversion to integer PCM.

The CLI uses `--stems orbit|voice|instrument` and interprets `--out` as the new set directory. The desktop uses an export mode with a grouping selector and previews the derived directory. A set contains `mix.wav`, numbered stereo stem files, and `stems.json`. The manifest is published last and includes typed group identities, so both CLI and app can report paths without interpreting user labels.

All distinct scheduled groups count toward a 128-stem limit. The pipeline also checks a 512 MiB budget for PCM output, chunk buffers, and a single WAV encoding buffer before starting any render worker. The budget is for audio buffers. The host, synth state, and effects caches also use memory. Limit the worker output queue so the estimate holds even if progress reporting and joining run slowly.
