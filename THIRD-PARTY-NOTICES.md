# Third-party notices

Strudeland uses the GNU Affero General Public License, version 3 or later. See the root
[LICENSE](LICENSE). The notices below identify code and assets from other projects and the
licenses that accompany them.

## Strudel and embedded code

The Strudel packages in [vendor/strudel](vendor/strudel) come from the TidalCycles/uzu community
and Strudel contributors. [UPSTREAM.json](vendor/strudel/UPSTREAM.json) identifies the selected
packages at [revision 0cb6341299229f739b5090be93528bb4f170d855](https://codeberg.org/uzu/strudel/src/commit/0cb6341299229f739b5090be93528bb4f170d855).
[PATCHES.md](vendor/strudel/PATCHES.md) records Strudeland's changes. These packages declare
AGPL-3.0-or-later and carry the [Strudel license](vendor/strudel/LICENSE). The frontend uses the
editor, language and audio packages. The [headless JS bundle inputs](crates/strudeland-host/js/host.inputs.json)
identify the subset and npm dependencies compiled into the headless host.

### reverbGen

Copyright 2014 Alan deLespinasse, Apache-2.0. The impulse-response generator, including
its envelopes and gradual low-pass filter, and the `generateGraph` helper match the upstream
implementation at `d1bd9a0`. Strudel adapted it to the context's sample rate, module system
and audio-node lifecycle. Strudeland seeded the noise generator and ported the impulse-response path to
Rust. The native convolution stages separately derive from Blink at Chromium 156.0.8078.4.
[Original generator](https://github.com/adelespinasse/reverbGen/blob/d1bd9a013cc74c9e2ec8be749ff00d844a05cc23/reverbgen.js),
[upstream license](https://github.com/adelespinasse/reverbGen/blob/d1bd9a013cc74c9e2ec8be749ff00d844a05cc23/LICENSE.txt),
[browser adaptation](vendor/strudel/packages/superdough/reverbGen.mjs),
[native port](crates/strudeland-engine/src/reverb.rs), [Apache-2.0 license](LICENSES/Apache-2.0.txt),
[Blink license](crates/strudeland-engine/LICENSE-chromium).

### WebDirt

WebDirt was created by Jamie Beverley and David Ogborn. [Strudel's worklet header](vendor/strudel/packages/superdough/worklets.mjs)
credits WebDirt for coarse, crush and shape. The three effects are also implemented in
[Rust shaper.rs](crates/strudeland-engine/src/shaper.rs). The cited revision's license is GPL
version 3. [Worklets at 5ce3d698362c54d6e1b68acc47eb2955ac62c793](https://github.com/dktr0/WebDirt/blob/5ce3d698362c54d6e1b68acc47eb2955ac62c793/dist/AudioWorklets.js),
[upstream license at that revision](https://github.com/dktr0/WebDirt/blob/5ce3d698362c54d6e1b68acc47eb2955ac62c793/LICENSE),
[GPLv3 text](LICENSES/GPL-3.0.txt).

WebDirt's five vowel tables (`a e i o u`) and parallel bank of band-pass filters with gain
and summed output appear in the pre-npm tree at `70e97c1`, with a GPLv3 license.
The same filter routine and table entries appear at Strudel's cited revision `41342e8`,
whose npm manifest declares ISC. The retained portion comes under the earlier GPLv3 grant.
Strudel adapted the filter in 2022 and added the ten vowel entries `ae aa oe ue y uh un en an on`
and their aliases in 2024. It uses the table Q directly and manages the audio nodes.
The Rust port follows the Strudel version.
[Pre-npm source](https://github.com/dktr0/WebDirt/blob/70e97c158a1ece0be4a0f4ad0da5a2f6ebe32530/Graph.js#L514-L577),
[pre-npm license](https://github.com/dktr0/WebDirt/blob/70e97c158a1ece0be4a0f4ad0da5a2f6ebe32530/LICENSE),
[later Graph.js](https://github.com/dktr0/WebDirt/blob/41342e81d6ad694a2310d491fef7b7e8b0929efe/js-src/Graph.js#L525-L603),
[later manifest](https://github.com/dktr0/WebDirt/blob/41342e81d6ad694a2310d491fef7b7e8b0929efe/package.json),
[Strudel import](https://github.com/tidalcycles/strudel/commit/4406dd88d1f3f4313149216f3e4cd3d55d2381cb),
[added vowels](https://github.com/tidalcycles/strudel/commit/e051d383a07f54a7064e8cb67ce84e05f1de7ca9),
[added aliases](https://github.com/tidalcycles/strudel/commit/efa0737660b9d13ba127ed6eaea6db8d35053afa),
[browser implementation](vendor/strudel/packages/superdough/vowel.mjs),
[native port](crates/strudeland-engine/src/vowel.rs).

### Additional embedded code

- **Euclidean rhythms.** Strudeland ported the Bjorklund group reduction and sequence
  construction from Haskell Music Theory 0.20, copyright 2006–2022 Rohan Drape, GPL-3.0-only.
  The Strudel pattern wrappers carry the 2023 notice for Rohan Drape and Strudel contributors,
  AGPL-3.0-or-later. Both the browser and the headless host bundle the module.
  [HMT source](https://hackage-content.haskell.org/package/hmt-0.20/src/Music/Theory/Bjorklund.hs),
  [release metadata](https://hackage-content.haskell.org/package/hmt-0.20/src/hmt.cabal),
  [JavaScript port and wrappers](vendor/strudel/packages/core/euclid.mjs),
  [GPLv3](LICENSES/GPL-3.0.txt), [Strudel license](vendor/strudel/LICENSE).
- **fft.js.** Copyright Fedor Indutny, 2017, MIT. [Strudel's copy](vendor/strudel/packages/superdough/fft.js)
  credits upstream without a revision and is imported by the browser worklets.
  [Rust fft.rs](crates/strudeland-engine/src/fft.rs) ports Blink's RustFFT integration.
  [Upstream](https://github.com/indutny/fft.js), [verbatim vendored notice](LICENSES/fft.js-MIT.txt).
- **Phaze.** Olivier (olvb) released Phaze under the Unlicense. Strudel adapts Phaze's
  [overlap-add base class](vendor/strudel/packages/superdough/ola-processor.js#L3) and
  [phase vocoder](vendor/strudel/packages/superdough/worklets.mjs) for the browser's
  stretch effect. The matching upstream files and license were verified on the branch cited
  by Strudel, which does not specify a revision.
  [Upstream overlap-add](https://github.com/olvb/phaze/blob/master/src/ola-processor.js),
  [upstream phase vocoder](https://github.com/olvb/phaze/blob/master/src/phase-vocoder.js),
  [Unlicense text](LICENSES/phaze-Unlicense.txt).
- **Bytebeat.** The browser worklet adapts Chasyxx's `bitC`, `br`, `sinf`, `cosf`,
  `tanf` and `regG` helpers from EnBeat_NEW. Its expression compiler binds Math names,
  `int`, `window` and the helpers. The packed-source optimization uses the expression
  rewrite in the same EnBeat_NEW file, derived from Dollchan's bytebeat compiler.
  The EnBeat_NEW tree carries separate MIT notices for Chasyxx, SthephanShi and SArpnt.
  [Worklet](vendor/strudel/packages/superdough/worklets.mjs),
  [Strudel import](https://github.com/tidalcycles/strudel/commit/608dfd515e4b305ae9152de7a114d313a7742563),
  [EnBeat_NEW compiler](https://github.com/Chasyxx/EnBeat_NEW/blob/ca5bc41d1bcd79421062fa9b10d1784ed2894971/src/audio-processor.mjs#L264-L305),
  [Dollchan optimization](https://github.com/SthephanShinkufag/bytebeat-composer/commit/c4efa88aab3b26e42d27218b7c832be9464602b9),
  [Chasyxx © 2023](LICENSES/EnBeat_NEW-MIT.txt),
  [SthephanShi © 2021](LICENSES/Dollchan-Bytebeat-MIT.txt), [SArpnt © 2021](LICENSES/SArpnt-MIT.txt).
- **ZzFX.** Copyright (c) 2019 Frank Force, MIT. [Strudel's fork](vendor/strudel/packages/superdough/zzfx_fork.mjs)
  adapts sample generation from ZzFX.js and adds context sample rate and an injectable random source.
  It cites a branch without a revision. [Upstream source](https://github.com/KilledByAPixel/ZzFX/blob/master/ZzFX.js),
  [MIT text](LICENSES/ZzFX-MIT.txt).
- **Moog ladder filter.** The browser and native implementations derive their four
  nonlinear stages, four-tap feedback and rational tanh approximation from Magnus
  Jonsson's Microtracker, via the MoogLadders Microtracker model and The BouteillacBear's
  Web Audio kernel (copyright 2023). Strudel adapted the kernel for stereo processing
  and added its drive, resonance scaling and makeup gain. Strudeland ported that worklet
  to Rust. [Microtracker filter](https://github.com/magnusjonsson/microtracker/blob/86dd19c55601c3054252ea70ea886fb9dbc6f353/plugins/src/shared/moogfilter.c),
  [Microtracker tanh](https://github.com/magnusjonsson/microtracker/blob/86dd19c55601c3054252ea70ea886fb9dbc6f353/plugins/src/shared/fasttanh.c),
  [Microtracker Unlicense](LICENSES/Microtracker-Unlicense.txt),
  [MoogLadders model](https://github.com/ddiakopoulos/MoogLadders/blob/a968ec6ddfe1cf9b228faea277cb0e66a060639a/src/MicrotrackerModel.h),
  [MoogLadders Unlicense](LICENSES/MoogLadders-Unlicense.txt),
  [Web Audio kernel](https://github.com/TheBouteillacBear/webaudioworklet-wasm/blob/0632e138a561bd1adf2a3d4eec0e95b2fe1c52ca/src/moog/filterKernel.c),
  [kernel MIT license](LICENSES/webaudioworklet-wasm-MIT.txt),
  [browser worklet](vendor/strudel/packages/superdough/worklets.mjs),
  [native port](crates/strudeland-engine/src/ladder.rs).
- **Pulse oscillator.** Strudeland's browser and Rust implementations use a four-point
  BLEP derived from an integrated cubic B-spline to smooth each edge of a variable-width
  pulse, with a two-sample delay. Both replacements are distributed
  under the project's AGPL-3.0-or-later license.
  [Browser worklet](vendor/strudel/packages/superdough/worklets.mjs),
  [native oscillator](crates/strudeland-engine/src/pulse.rs), [license](LICENSE).
- **Kabelsalat.** Felix Roos and contributors, AGPL-3.0-or-later. The production build includes
  the web package chunk. The installed web and lib packages are 0.4.1, with core at 0.4.0.
  Their license texts are in the installed packages, and the AGPLv3 text is in the
  [root license](LICENSE). [npm lockfile](bun.lock).

### Editor themes

Strudel migrated the following CodeMirror themes from uiw 4.23.7 to local definitions,
carrying over settings, highlight groups and source credits from uiw's release at
`5fb6ebc`. Strudel adapted uiw's theme helper and subsequently changed some editor
settings and highlight rules. [Strudel lockfile](https://github.com/tidalcycles/strudel/blob/5a188f7461af9bbaf7d97c9609e1a6742ad4c6ed/pnpm-lock.yaml#L2807-L2835),
[Atom One and Dracula migration](https://github.com/tidalcycles/strudel/commit/9112a0c4f57622a39bb5b1dd80657c93d57e6904),
[DuoTone migration](https://github.com/tidalcycles/strudel/commit/63f9f6cba0e2da056864cb8725c051ad95c80b90),
[uiw theme helper](https://github.com/uiwjs/react-codemirror/blob/5fb6ebc6ef9aa3226a6c598960e7264a32409890/themes/theme/src/index.tsx),
[Strudel theme helper](vendor/strudel/packages/codemirror/themes/theme-helper.mjs),
[uiw MIT license, copyright 2021 uiw](LICENSES/uiw-MIT.txt).

- **DuoTone Dark and Light.** Both definitions are included in the source distribution.
  Simurai created the DuoTone palettes. Bram de Haan adapted them to CodeMirror, crediting
  Jan T. Sott for the CSS template. The settings and token color groups subsequently appear
  in uiw's port. The two original palettes carry the same 2015 simurai MIT text.
  [Dark definition](vendor/strudel/packages/codemirror/themes/duotoneDark.mjs),
  [light definition](vendor/strudel/packages/codemirror/themes/duotoneLight.mjs),
  [uiw port](https://github.com/uiwjs/react-codemirror/blob/5fb6ebc6ef9aa3226a6c598960e7264a32409890/themes/duotone/src/index.ts),
  [original dark](https://github.com/simurai/duotone-dark-syntax/tree/d0ce468711a82a60b177bfcfc963426e78c9b388),
  [original light](https://github.com/simurai/duotone-light-syntax/tree/e657a53e9325b5e481702a9290ba75f05b22ed8b),
  [simurai MIT license](LICENSES/DuoTone-MIT.txt),
  [Base2Tone CodeMirror](https://github.com/atelierbram/Base2Tone-codemirror/tree/66b2e3d0a21c4f97b7b1f264f19a183fa8318d38),
  [Bram de Haan's 2016 MIT license](LICENSES/Base2Tone-CodeMirror-MIT.txt),
  [CodeMirror contribution](https://github.com/codemirror/codemirror5/commit/658f9a07279fc978c6b533f3325172a735121262),
  [CodeMirror 2016 MIT license](LICENSES/CodeMirror-2016-MIT.txt).
- **Atom One.** uiw credits Atom's One Dark Syntax, copyright 2016 GitHub Inc., MIT.
  The Strudel adaptations include Atom One and the red-text, green-text and sonic-pink
  variants, which reuse its syntax-rule groups with different palettes.
  [Atom One](vendor/strudel/packages/codemirror/themes/atomone.mjs),
  [red](vendor/strudel/packages/codemirror/themes/red-text.mjs),
  [green](vendor/strudel/packages/codemirror/themes/green-text.mjs),
  [pink](vendor/strudel/packages/codemirror/themes/sonic-pink.mjs),
  [uiw port](https://github.com/uiwjs/react-codemirror/blob/5fb6ebc6ef9aa3226a6c598960e7264a32409890/themes/atomone/src/index.ts),
  [Atom source](https://github.com/atom/one-dark-syntax/tree/9c96f4454362267ac45322063e193ccf9d2debb1),
  [Atom MIT license](LICENSES/Atom-One-Dark-MIT.txt).
- **Dracula.** Zeno Rocha published the original palette under MIT. Michael Kaminsky
  contributed the Dracula stylesheet to CodeMirror in 2015 under that tree's MIT license.
  Both names and the token colors appear in uiw's theme and Strudel's local adaptation.
  [Strudel definition](vendor/strudel/packages/codemirror/themes/dracula.mjs),
  [uiw port](https://github.com/uiwjs/react-codemirror/blob/5fb6ebc6ef9aa3226a6c598960e7264a32409890/themes/dracula/src/index.ts),
  [original palette and grant, © Zeno Rocha](https://github.com/dracula/dracula-theme/blob/bc06b890c953af0e99a7948dd0cb0098499c3c5a/README.md#license),
  [Kaminsky's contribution](https://github.com/codemirror/codemirror5/commit/f52e64d095db6a8bbbd2528bab341ccd44e4b2bf),
  [CodeMirror 2015 MIT license](LICENSES/CodeMirror-2015-MIT.txt),
  [CodeMirror 2017 MIT license](LICENSES/CodeMirror-5-MIT.txt),
  [Dracula Theme 2016 MIT license](LICENSES/Dracula-2016-MIT.txt),
  [Dracula Theme 2023 MIT license](LICENSES/Dracula-Theme-MIT.txt).

## Native ports and linked libraries

- The engine ports parts of Blink at Chromium 156.0.8078.4 (Copyright 2012 The Chromium
  Authors in the port headers, BSD-3-Clause), V8 math from revision b976f35 at that Chromium tag
  (V8 project authors and upstream holders listed in the license, BSD-3-Clause), and CORE-MATH
  routines at commit 018fa97cbdacf59924b899a9dfa422d532cb2bb7 (Paul Zimmermann and Alexei Sibidanov,
  MIT). These ports are in the native renderer and live engine. See the
  [engine's upstream paths](crates/strudeland-engine/README.md) and the
  [Chromium](crates/strudeland-engine/LICENSE-chromium), [V8](crates/strudeland-engine/LICENSE-v8)
  and [CORE-MATH](crates/strudeland-engine/LICENSE-core-math) notices.
- The headless host separately links rusty_v8 152.2.0 and its prebuilt V8 with bundled
  dependencies. The [host's V8 notices](crates/strudeland-host/LICENSE-v8) identify those
  components and their copyright holders and contain their license texts. The host is used
  by the offline CLI renderer.
- The render crate uses LAME and its Rust wrappers (LGPL), libopus and Rust bindings
  (BSD/MIT/Apache), aoTuV/Lancer libvorbis and libogg with BSD-licensed wrappers, the Ogg
  crate (BSD-3-Clause), and Symphonia verification decoders (MPL-2.0). See the
  [LAME](crates/strudeland-render/LICENSE-lame), [libopus](crates/strudeland-render/LICENSE-opus),
  [Opus bindings](crates/strudeland-render/LICENSE-opus-rs), [Vorbis](crates/strudeland-render/LICENSE-vorbis),
  [Ogg](crates/strudeland-render/LICENSE-ogg-rs) and [Symphonia](crates/strudeland-render/LICENSE-symphonia)
  notices for versions and copyright holders. The [render README](crates/strudeland-render/README.md)
  describes static LAME relinking.
- The render and live crates use Rubato 5.1.0. The
  [Rubato notice](crates/strudeland-render/LICENSE-rubato) covers Rubato, RealFFT and the audioadapter
  crates. RustFFT also has a direct engine dependency. Match the release's full resolved native
  dependency graph to [Cargo.lock](Cargo.lock).

## Fonts, icons and audio assets

- **Space Grotesk.** Copyright 2020 The Space Grotesk Project Authors. The bundled Latin variable
  WOFF2 is from `@fontsource-variable/space-grotesk@5.3.0`.
  [Upstream](https://github.com/floriankarsten/space-grotesk),
  [SIL Open Font License 1.1](LICENSES/Space-Grotesk-OFL-1.1.txt).
- **JetBrains Mono.** Copyright 2020 The JetBrains Mono Project Authors. The bundled Latin variable
  WOFF2 is from `@fontsource-variable/jetbrains-mono@5.3.0`.
  [Upstream](https://github.com/JetBrains/JetBrainsMono),
  [SIL Open Font License 1.1](LICENSES/JetBrains-Mono-OFL-1.1.txt).

Both font licenses were copied from the corresponding pinned Fontsource package. Both subsets
are loaded by [fonts.css](src/styles/fonts.css).

- **App icons.** The initial scaffold generated a solid-color PNG and produced the committed
  icons with `tauri icon`. The macOS bundle uses the generated `icon.icns`.
  [icon files](src-tauri/icons).
- **Test audio.** The three [corpus samples](tests/render/corpus/samples) are synthesized by
  [make-samples.ts](scripts/render/make-samples.ts). The [browser reference WAVs](crates/strudeland-engine/tests/refs)
  are generated by [engine:refs](scripts/engine/refs.ts) from [micro fixtures](tests/render/engine-micro)
  and their planner jobs. [engine-sample.wav](tests/helpers/engine-sample.wav) is a Rust-rendered
  test fixture used in [render-rust-engine.test.ts](tests/render-rust-engine.test.ts).
  These WAVs are test data outside the frontend output directory.
- **Sample libraries and music.** Dirt-Samples appears in sampler documentation examples.
  A song can load a sample pack the user specifies. The inspected frontend build and app
  bundle contained no sample pack.

## Render tooling and publication scope

Andrew Rudoi owns the earlier render tooling adapted for Strudeland and authorized its
release under AGPL-3.0-or-later for the portions included in the reviewed, single-root-commit
public source snapshot. These portions include lookback and chunk planning, the render
verification gates, and the related thresholds and helpers. Their consumers include the
headless host planner, offline render pipeline and validation harness.
[Lookback](src/lib/render/lookback.ts), [chunk planning](src/lib/render/chunks.ts),
[gates](src/lib/render/gates), [thresholds](src/lib/render/thresholds.ts), [license](LICENSE).

The source preview uses one root commit in a new repository. The original repository stays
private, including its earlier MusicDSP-derived pulse implementations and their historical
artifacts. The public snapshot review excludes superseded source, its transcriptions and
derived audio, and private project material.

## Binary distribution

An inspected production frontend build emitted browser JS, a clock worker, CSS and the two
WOFF2 files. Both playback backend choices use an app with the worklet code present.
The [worklet bundler](vendor/strudel/packages/vite-plugin-bundle-audioworklet/vite-plugin-bundle-audioworklet.js)
inlines a data URL containing the coarse, crush, shape, ladder, pulse, phase-vocoder and
bytebeat processors. The inspected macOS bundle embeds the frontend assets in its executable
and carries the icon in Resources. Both outputs need standalone notices for distribution.

Package the root AGPL text, this notice and its applicable license texts with the
signed release, before signing. Include the engine and live dependency notices for the app. Include
the host and codec notices if the release ships a CLI. The [app manifest](src-tauri/Cargo.toml)
links live and engine through Rust dependencies. The [CLI manifest](crates/strudeland-cli/Cargo.toml)
includes the host and offline renderer. When distributing the CLI, include the corresponding
source and the materials needed for LAME relinking described by the render crate.

The production frontend source map also identifies Svelte, CodeMirror, Lezer, Replit's CodeMirror
extensions, the Tauri JavaScript API and plugins, Tonal, Acorn and other npm dependencies. The
source map omits the contents of inlined worklets. Use the actual emitted code, worklet inputs,
[resolved npm versions](bun.lock) and applicable host bundle inputs to identify the shipped
dependency notices, including transitive dependencies. Confirm the texts and corresponding
source against the final target's native dependency graph. Inspect the signed artifact for
user-accessible notices and the release's corresponding-source information.
