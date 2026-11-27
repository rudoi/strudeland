# 002: give the superdough backend its own type

## Goal

`graph` and `masterLatencySec` leave `AudioBackend` and go to a new `SuperdoughBackend` in
`src/lib/audio/superdough.ts`, which `createSuperdoughBackend` returns. `app.ts` reads both
without `?.` and without its ad hoc structural type. `fakeRuntime` loses its `graph` option,
which only its own self-test used. Only types change: nothing that runs changes except three
`?.` call sites in `app.ts` that can no longer miss, and the removed test-helper option.
Sound does not change (see the proposal, 002).

## Prerequisites

- Stage 001 is committed and its gate passed.
- `git diff --name-only 9a5b6ff HEAD -- . ":(exclude)plans"` lists exactly 001's four files.

## Files

- Edited: `src/lib/audio/backend.ts`, `src/lib/audio/superdough.ts`, `src/lib/app.ts`,
  `tests/helpers/fake-runtime.ts`, `tests/superdough.test.ts`, `AGENTS.md`.
- Scratch, not committed: `$S/0136-js.ts` (appendix; already written by the session that wrote
  this plan).
- Frozen: everything else, and in particular `src/lib/strudel/runtime.ts` (`prepareAudition`
  keeps `opts.backends.superdough` hard-coded at `:1030`; no doc line is added there),
  `src/lib/audio/rust.ts`, every module that takes a `graph` port (`note-audition.ts`,
  `instrument-preview.ts`, `metronome.ts`, `memo-player.ts`, `output-clock.ts`), and every
  test except the one deletion in `tests/superdough.test.ts`.

## Steps

1. **Baseline.** `N=002`.
   - `L base-js bun "$S/0136-js.ts" 002` must exit 1: `backend.ts`, `superdough.ts` and
     `runtime.ts` print `equal`, while `fake-runtime.ts` (JS line 47, `const graph = opts.graph;`)
     and `app.ts` (JS line 61, `superdough.graph?.() ?? null`) print `DIFFERENT`. (Dry-run when
     the plan was written: this on the unchanged tree, and exit 0 on a draft with exactly this
     stage's edits.)
   - `L base-check bun run check`: `0 ERRORS 0 WARNINGS`.

2. **`backend.ts`.** Delete the `graph?()` member with its doc comment (`:50-55`, from
   `  /**` / `   * The live graph, or null unless audio runs` through
   `  graph?(): AudioGraph | null;`), and `masterLatencySec?()` with its doc line (`:66-67`).
   Keep `AudioGraph` (`:12-16`): every graph port imports it from here. Nothing is added
   (`git diff --numstat` gives `0	8`).

3. **`superdough.ts`.** Before `export function createSuperdoughBackend` (`:53`) add:

   ```ts
   /**
    * The superdough backend, with what only it has: the Web Audio graph and the master stage's
    * latency. Note and instrument previews, the metronome, the output clock and memo playback use
    * them; previews play here even while the Rust engine plays the song.
    */
   export interface SuperdoughBackend extends AudioBackend {
     /**
      * The live graph, or null unless audio runs (it never creates the context) or while the output
      * is being rebuilt. Never throws. Callers call it each time they need a node and never keep the
      * result: the destination and the context can be replaced.
      */
     graph(): AudioGraph | null;
     /** The delay the master stage adds to what is heard, in seconds (0 when none). */
     masterLatencySec(): number;
   }

   ```

   The two member doc comments are the ones deleted from `backend.ts`, unchanged. The return
   type on `:53` becomes `SuperdoughBackend`. The returned object (`:62-112`) does not change:
   it already has both members. The `AudioBackend, AudioGraph` type import (`:4`) stays as it
   is (`numstat` `17	1`).

4. **`app.ts`.**
   - Delete `:4` (`import type { AudioGraph } from './audio/backend';`): nothing else in the
     file uses it.
   - `:11` becomes `import { createSuperdoughBackend, type SuperdoughBackend } from './audio/superdough';`.
   - `:127` becomes `  const auditionGraph = () => superdough.graph();`.
   - `:304` becomes `  backend: SuperdoughBackend,`.
   - `:309` becomes `  const graph = () => backend.graph();`.
   - `:322` becomes `    outputDelaySec: () => backend.masterLatencySec(),`.

   Nothing else changes (`numstat` `5	6`). The doc comment on `createRecording` (`:298-301`)
   already says "all on the one superdough backend's graph".

5. **`tests/helpers/fake-runtime.ts`.** `:3` becomes
   `import type { AudioBackend } from '../../src/lib/audio/backend';`. Delete the `graph`
   option with its doc line (`:17-18`) and the block that installs it (`:78-83`, from
   `  const graph = opts.graph;` through its `  }`). The fake superdough backend stays an
   `AudioBackend`: it goes to `createRuntime`, whose `backends.superdough` stays `AudioBackend`
   (`runtime.ts:230`). `numstat`: `1	9`.

6. **`tests/superdough.test.ts`.** Delete `:6` (`import { fakeRuntime } from './helpers/fake-runtime';`)
   and `:362-376`, the blank line and the test
   `'fakeRuntime passes it through as the backend graph(), null until audio runs'`. The
   `describe('fakeAudioGraph')` it sat in keeps its first test and its closing `});`.
   `numstat`: `0	16`. Every other test, the `be.graph!()` and `be.masterLatencySec!()` calls
   included (`:85-160`, `:217-328`), stays byte-identical. A `!` on a member that is no longer
   optional still compiles.

7. **`AGENTS.md`** (`:84-85`). Replace
   ``does with `LINEAR_SEGMENTS`). `src/lib/audio/` the `AudioBackend` seam (and `scope-tap.ts`, the``
   with these three lines:

   ```markdown
     does with `LINEAR_SEGMENTS`). `src/lib/audio/` the `AudioBackend` seam (`SuperdoughBackend` in
     `superdough.ts` adds the Web Audio graph and master latency that previews, the metronome and
     memos use; and `scope-tap.ts`, the
   ```

   Change no other line.

8. Targeted runs: `bun test tests/superdough.test.ts tests/scope-tap.test.ts tests/app.test.ts tests/runtime-backends.test.ts`
   (these must pass), then `bun run format`, commit (below), then the full gate with `N=002`
   and the Validation steps.

## Tests

No new test. This stage only changes types, and the transpile proof shows that. The tests that
pin the sound decisions stay byte-identical and must pass:

- `tests/superdough.test.ts` `describe('superdough backend graph()')` (`:85-160`): null before
  audio runs, the destination gain and its context once running, null mid-reset and then the
  replaced node, null when suspended or when superdough throws;
- the master tap tests (`:162-330`): `masterLatencySec` forwards (`:217-229`) and follows the
  worklet (`:239-328`);
- `tests/runtime-backends.test.ts:366-378` (`'with Rust selected, audition resumes only the
  preview graph and reuses it'`): `prepareAudition` resumes superdough while the Rust engine is
  selected;
- the graph-port tests (`instrument-browser`, `memo-player`, `metronome`, `output-clock`,
  `memos`, `note-audition-audio`, `play-arbiter`, `recording-controller`), which pass `graph`
  functions directly;
- `tests/app.test.ts`, all of it, on the real `createSuperdoughBackend`.

The deleted self-test covered only the removed option.

## Validation

- The full gate steps 1 to 4 pass (000-prompt.md).
- Step 5, the proofs:
  - `L js bun "$S/0136-js.ts" 002`: exit 0, printing `equal:` for all five files:
    `backend.ts`, `superdough.ts` and `runtime.ts` with 0 planned replacements,
    `fake-runtime.ts` with 1, `app.ts` with 3;
  - `L numstat git diff --numstat HEAD~1 HEAD`: exactly `0 8` for `backend.ts`, `1 9` for
    `fake-runtime.ts`, `5 6` for `app.ts`, `0 16` for `superdough.test.ts`, and for
    `superdough.ts` 1 deleted line with the added lines of step 3 (`17	1` as written there;
    STOP if the deleted count is not 1). Plus the `AGENTS.md` lines;
  - `L grep-graph git grep -nE "graph\?\.|masterLatencySec\?\.|graph\?\(\)|masterLatencySec\?\(\)" -- src tests`:
    exit 1 (no match left);
  - `L literals "$S/0136-001-literals.sh"` and `L markup bun "$S/0136-markup.ts"`: still exit 0
    (`StatusBar.svelte` untouched).
- Step 6 lists exactly `AGENTS.md`, `src/components/StatusBar.svelte`, `src/lib/app.ts`,
  `src/lib/audio/backend.ts`, `src/lib/audio/superdough.ts`, `src/lib/status-notices.ts`,
  `tests/helpers/fake-runtime.ts`, `tests/status-notices.test.ts`, `tests/superdough.test.ts`.
- Step 7 shows two hunks: 001's bullet and this stage's `src/lib/audio/` sentence.
- No touched file is a host-bundle input or in `PLANNER_FILES`. `backend.ts` is not listed in
  `host.inputs.json` (its imports are type-only).

## Commit message

```
refactor(audio): give the superdough backend its own type

SuperdoughBackend carries graph() and masterLatencySec(), which only the
superdough backend has and only the app reads; both leave AudioBackend.
createSuperdoughBackend returns it, the app reads both without optional
calls, and the fake runtime drops its unused graph option. Types only:
routing, latency and preview resumption are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: `0136-js.ts` (also used by 003)

```ts
// The transpiled JavaScript (types and comments stripped by `bun build --no-bundle`) of each
// listed file must equal 9a5b6ff's with exactly the planned code replacements. Stage 002 changes
// types only, except the three `?.` call sites in app.ts and the fake's removed `graph` option;
// stage 003 adds the options object, the `backends` field and the two `prepare` arrows.
// Usage: bun 0136-js.ts 002|003 [--root DIR] (DIR: a tree to check in place of the repo, for
// testing the script). Exit 0 all equal, 1 a difference, 2 setup error.
import { $ } from 'bun';
import { mkdirSync, writeFileSync } from 'node:fs';

const REPO = '/Volumes/rudenet/code/strudeland-worktrees/poptart';
const S = import.meta.dir;
const stage = process.argv[2];
const rootAt = process.argv.indexOf('--root');
const ROOT = rootAt > 0 ? process.argv[rootAt + 1]! : REPO;
if (stage !== '002' && stage !== '003') {
  console.error('usage: bun 0136-js.ts 002|003 [--root DIR]');
  process.exit(2);
}

const APP_002: [string, string][] = [
  [
    '  const auditionGraph = () => superdough.graph?.() ?? null;\n',
    '  const auditionGraph = () => superdough.graph();\n',
  ],
  [
    '  const graph = () => backend.graph?.() ?? null;\n',
    '  const graph = () => backend.graph();\n',
  ],
  [
    '    outputDelaySec: () => backend.masterLatencySec?.() ?? 0\n',
    '    outputDelaySec: () => backend.masterLatencySec()\n',
  ],
];
const APP_003: [string, string][] = [
  [
    'export function createApp(api = createTauriProjectApi(), inTauri = isTauri, controlApi = inTauri() ? createTauriControlApi() : null, recordingApi = isTauri() ? createTauriRecordingApi() : null, prefsApi = inTauri() ? createTauriPrefsApi() : null) {\n',
    `export function createApp(options = {}) {
  const {
    api = createTauriProjectApi(),
    inTauri = isTauri,
    controlApi = inTauri() ? createTauriControlApi() : null,
    recordingApi = isTauri() ? createTauriRecordingApi() : null,
    prefsApi = inTauri() ? createTauriPrefsApi() : null
  } = options;
`,
  ],
  [
    `  const superdough = createSuperdoughBackend();
  const rust = inTauri() ? createRustBackend(tauriPlaybackApi) : undefined;
`,
    `  const { superdough, rust } = options.backends ?? {
    superdough: createSuperdoughBackend(),
    rust: inTauri() ? createRustBackend(tauriPlaybackApi) : undefined
  };
`,
  ],
];
const FAKE_002: [string, string][] = [
  [
    `  const graph = opts.graph;
  if (graph) {
    backend.graph = () => backend.isRunning() ? typeof graph === "function" ? graph() : graph : null;
  }
`,
    '',
  ],
];

/** File, its replacements, and how many times each must occur (the prepare arrows occur twice). */
const FILES: [string, [string, string][], Record<string, number>][] = [
  ['src/lib/audio/backend.ts', [], {}],
  ['src/lib/audio/superdough.ts', [], {}],
  ['src/lib/strudel/runtime.ts', [], {}],
  ['tests/helpers/fake-runtime.ts', FAKE_002, {}],
  [
    'src/lib/app.ts',
    stage === '002'
      ? APP_002
      : [
          ...APP_002,
          ...APP_003,
          [
            '    prepare: runtime.prepareAudition,\n',
            '    prepare: () => runtime.prepareAudition(),\n',
          ],
        ],
    { '    prepare: runtime.prepareAudition,\n': 2 },
  ],
];

const transpile = async (dir: string, file: string, text: string) => {
  const at = `${S}/0136-js/${dir}/${file}`;
  mkdirSync(at.slice(0, at.lastIndexOf('/')), { recursive: true });
  writeFileSync(at, text);
  return await $`bun build --no-bundle ${at}`.quiet().text();
};

let ok = true;
try {
  for (const [file, reps, times] of FILES) {
    const base = await transpile('base', file, await $`git -C ${REPO} show 9a5b6ff:${file}`.text());
    const now = await transpile('now', file, await Bun.file(`${ROOT}/${file}`).text());
    let expected = base;
    for (const [from, to] of reps) {
      const n = expected.split(from).length - 1;
      if (n !== (times[from] ?? 1)) throw new Error(`${file}: base JS has ${n} of:\n${from}`);
      expected = expected.split(from).join(to);
    }
    if (now === expected) {
      console.log(`equal: ${file} (${reps.length} planned replacements)`);
      continue;
    }
    ok = false;
    const a = expected.split('\n');
    const b = now.split('\n');
    const i = a.findIndex((line, k) => line !== b[k]);
    console.log(`DIFFERENT: ${file} at JS line ${i + 1}:\n- ${a[i]}\n+ ${b[i]}`);
  }
} catch (e) {
  console.error(String(e));
  process.exit(2);
}
process.exit(ok ? 0 : 1);
```
