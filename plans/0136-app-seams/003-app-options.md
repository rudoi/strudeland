# 003: take createApp's seams as one options object

## Goal

`createApp` takes one optional `AppOptions` object in place of its five positional optional
parameters, with the same defaults, plus an optional `backends` field. The 12 positional calls
in `tests/app.test.ts` become options objects; `App.svelte`'s `createApp()` already matches.
`prepareAudition` (twice in `app.ts`) and `drainLevels` (`StatusBar.svelte`) are read when
called, not captured. The runtime is still built inside `createApp`, and the existing
`runtime.stop` monkeypatches stay. Nothing visible or audible changes.

## Prerequisites

- Stages 001 and 002 are committed and their gates passed.
- `git diff --name-only 9a5b6ff HEAD -- . ":(exclude)plans"` lists exactly 002's nine files.

## Files

- Edited: `src/lib/app.ts`, `tests/app.test.ts`, `src/components/StatusBar.svelte` (allowed
  markup replacement 3 only).
- Not edited: `src/App.svelte`. Its `createApp()` (`:25`) type-checks against
  `createApp(options: AppOptions = {})` and keeps every default (proposal, open question 2).
  `AGENTS.md`: nothing in it describes `createApp`'s parameters (`grep -n createApp AGENTS.md`
  finds nothing), so it stays.
- Scratch, not committed: `$S/0136-003-tests.ts` (appendix), plus `0136-js.ts` (002's appendix)
  and `0136-markup.ts` and `0136-001-literals.sh` (001's appendix).
- Frozen: everything else, `runtime.ts` and the `tests/helpers/*` fakes included.

## Steps

1. **Baseline.** `N=003`. Each must exit 1 on the unchanged tree (each was dry-run when the
   plan was written: exit 1 like this, and exit 0 on a draft with exactly this stage's edits):
   - `L base-js bun "$S/0136-js.ts" 003`: `app.ts` is `DIFFERENT` at JS line 32
     (`export function createApp(options = {}) {` expected); the other four files are `equal`.
   - `L base-tests bun "$S/0136-003-tests.ts"`: first difference at line 22
     (`import type { RustBackend } …` expected).
   - `L base-markup bun "$S/0136-markup.ts" --drain`: first difference at markup line 78 and
     file line 148 (`drain={() => runtime.drainLevels()}` expected).

   Then `L base-check bun run check`: `0 ERRORS 0 WARNINGS`.

2. **Tests first** (`tests/app.test.ts`).
   - After `import type { EditorView } from '@codemirror/view';` (`:21`) add:

     ```ts
     import type { RustBackend } from '../src/lib/audio/rust';
     import type { SuperdoughBackend } from '../src/lib/audio/superdough';
     ```

     Type-only imports are erased, so they do not load the modules before
     `GlobalRegistrator.register()`.
   - Rewrite the 12 calls, each argument under its parameter's name, and nothing else on the
     line:

     | Line | Today | Becomes |
     |---|---|---|
     | `:69` | `createApp(api, () => opts.tauri ?? true, null, null, null)` | `createApp({ api, inTauri: () => opts.tauri ?? true, controlApi: null, recordingApi: null, prefsApi: null })` |
     | `:260`, `:417` | `createApp(api, () => true, null, null, null)` | `createApp({ api, inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: null })` |
     | `:440`, `:474`, `:513` | `createApp(api, () => true, control.api, null, null)` | `createApp({ api, inTauri: () => true, controlApi: control.api, recordingApi: null, prefsApi: null })` |
     | `:456` | `createApp(api, () => false)` | `createApp({ api, inTauri: () => false })` |
     | `:615` | `createApp(api, () => true, null, rec.api, null)` | `createApp({ api, inTauri: () => true, controlApi: null, recordingApi: rec.api, prefsApi: null })` |
     | `:821` | `createApp(api, () => true, null, null, prefs)` | `createApp({ api, inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: prefs })` |
     | `:1026`, `:1039` | `createApp(apiFor(), () => true, null, null, prefs)` | `createApp({ api: apiFor(), inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: prefs })` |
     | `:1050` | `createApp(apiFor(), () => true, null, null, null)` | `createApp({ api: apiFor(), inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: null })` |

     `bun run format` then breaks every call but `:456` over seven lines; that is expected,
     and the test proof formats its expected file the same way. `:456` passes only two fields,
     as today it passes two arguments: `controlApi` and `prefsApi` default to null through
     `inTauri() === false`, and `recordingApi` through the real `isTauri()`, which is false
     under `bun test`.
   - Append after the last `});` one new `describe`, separated by one blank line:

     ```ts
     describe('createApp options', () => {
       /** Backends that log what the runtime and the previews ask of them. */
       function fakeBackends() {
         const log: string[] = [];
         const superdough: SuperdoughBackend = {
           id: 'superdough',
           init: async () => {},
           resume: async () => true,
           isRunning: () => false,
           getTime: () => 0,
           setSeed: () => {},
           output: () => {},
           setLimiter: (on) => void log.push(`superdough limiter ${on}`),
           graph: () => (log.push('superdough graph'), null),
           masterLatencySec: () => 0,
         };
         const rust: RustBackend = {
           id: 'rust',
           init: async () => {},
           resume: async () => true,
           isRunning: () => false,
           getTime: () => 0,
           setSeed: () => {},
           output: () => {},
           liveStats: () => null,
           onWarning: () => () => {},
           onLevels: () => () => {},
           setLimiter: (on) => void log.push(`rust limiter ${on}`),
           limiterAvailable: () => true,
           stop: async () => {},
         };
         return { log, superdough, rust };
       }

       function appWith(backends: { superdough: SuperdoughBackend; rust?: RustBackend }) {
         const { api } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: '$: s("a")\n' } } });
         // Renders succeed, so a preview reaches the graph.
         api.instrumentPreview = async () => new Uint8Array();
         const app = createApp({
           api,
           inTauri: () => true,
           controlApi: null,
           recordingApi: null,
           prefsApi: null,
           backends,
         });
         app.runtime.stop = () => {}; // the real stop draws on a canvas happy-dom lacks
         return app;
       }

       const version = { id: 'pad', fingerprint: 'pad', root: null };

       test('the runtime plays on the given backends', () => {
         const { log, superdough, rust } = fakeBackends();
         const app = appWith({ superdough, rust });
         log.length = 0;
         app.runtime.setLimiter(true);
         expect(log).toEqual(['superdough limiter true', 'rust limiter true']);
         app.destroy();
       });

       test('given backends without rust, none is built, even in Tauri', () => {
         const { log, superdough } = fakeBackends();
         const app = appWith({ superdough });
         log.length = 0;
         app.runtime.setLimiter(true);
         expect(log).toEqual(['superdough limiter true']);
         app.destroy();
       });

       test("previews read the given superdough backend's graph", async () => {
         const { log, superdough } = fakeBackends();
         const app = appWith({ superdough });
         expect(app.arbiter.auditionsAvailable()).toBe(true);
         app.runtime.prepareAudition = async () => true;
         await app.instrumentPreview.play(version);
         expect(log).toContain('superdough graph');
         expect(get(app.instrumentPreview.store)).toEqual({
           status: 'error',
           message: 'audio output is unavailable',
         });
         app.destroy();
       });

       test('previews call prepareAudition as it is when they start', async () => {
         const { superdough } = fakeBackends();
         const app = appWith({ superdough });
         expect(app.arbiter.auditionsAvailable()).toBe(true);
         let prepares = 0;
         app.runtime.prepareAudition = async () => (prepares++, false);
         await app.instrumentPreview.play(version);
         expect(prepares).toBe(1);
         expect(get(app.instrumentPreview.store)).toEqual({
           status: 'error',
           message: 'could not start preview audio',
         });
         app.destroy();
       });
     });
     ```

     Why each expectation holds: `runtime.setLimiter` calls `setLimiter` on both backends in
     `ENGINES` order (`runtime.ts:1062-1065`); the prefs subscription does not call it at
     creation (the default limiter pref and the runtime's initial limiter are both off), and
     `log.length = 0` makes the tests independent of that. The instrument preview
     (`instrument-preview.ts:34-69`) calls `prepare`, then `render`, then `graph()`, and stores
     the two error messages above. The last test is the call-time read: before this stage
     `prepare` holds the original method, so `prepares` stays 0.
   - Run `bun test tests/app.test.ts`. The new tests fail (positional `createApp` still takes
     the object as `api`), which is expected. Do not log it as a gate.

3. **`app.ts`** (line numbers after 002, which removed `:4`; every other line keeps its
   `9a5b6ff` number minus one).
   - `import { createRustBackend } from './audio/rust';` becomes
     `import { createRustBackend, type RustBackend } from './audio/rust';`.
   - The doc comment and signature (`9a5b6ff` `:83-96`) become `AppOptions` and the new
     signature. The doc comments move to the fields, with the same facts:

     ```ts
     /** `createApp`'s test seams. Every field is optional; `createApp()` builds the real app. */
     export interface AppOptions {
       /** Default: Tauri's. */
       api?: ProjectApi;
       /**
        * Whether the page runs in Tauri; default `isTauri`. It picks the defaults below and turns on
        * the Rust backend, the piano-roll inspection and the window title.
        */
       inTauri?: () => boolean;
       /** Default: Tauri's in Tauri. With `null` the control socket is not attached. */
       controlApi?: ControlApi | null;
       /**
        * Default: Tauri's when the page really runs in Tauri (`isTauri`, not `inTauri`). With `null`
        * there is no recording controller and no memo list.
        */
       recordingApi?: RecordingApi | null;
       /** Default: Tauri's in Tauri. With `null` the prefs live in memory only. */
       prefsApi?: PrefsApi | null;
       /**
        * The live backends. Default: a superdough backend, plus in Tauri a Rust backend over the
        * playback API. Given without `rust`, every song plays on superdough.
        */
       backends?: { superdough: SuperdoughBackend; rust?: RustBackend };
     }

     /** Composition root: the live backends, one runtime, one session per window. */
     export function createApp(options: AppOptions = {}): App {
       const {
         api = createTauriProjectApi(),
         inTauri = isTauri,
         controlApi = inTauri() ? createTauriControlApi() : null,
         recordingApi = isTauri() ? createTauriRecordingApi() : null,
         prefsApi = inTauri() ? createTauriPrefsApi() : null,
       } = options;
     ```

     Keep the five defaults' expressions and order exactly; the transpile proof compares them.
   - The backends (`9a5b6ff` `:106-108`) become:

     ```ts
       const { superdough, rust } = options.backends ?? {
         superdough: createSuperdoughBackend(),
         // The Rust engine plays through the app; in a plain browser every song plays on superdough.
         rust: inTauri() ? createRustBackend(tauriPlaybackApi) : undefined,
       };
     ```

     at the same place, so the backends are built after the prefs, the inspection queue, the
     vim mode and the visuals, as today.
   - Both `prepare: runtime.prepareAudition,` (`9a5b6ff` `:137` in the audition queue, `:152`
     in the instrument preview) become `prepare: () => runtime.prepareAudition(),`. Above the
     first, add `// Read at call time, so a replaced \`prepareAudition\` is the one called.`
   - Nothing else changes. `createRecording(recordingApi, superdough, …)` gets the
     destructured `superdough`, which is a `SuperdoughBackend` either way.

   Run `bun test tests/app.test.ts`. It must pass, all 58 old tests and the 4 new ones.

4. **`StatusBar.svelte`, allowed markup replacement 3.** `        drain={runtime.drainLevels}`
   (`:148` after 001; `:167` at `9a5b6ff`) becomes `        drain={() => runtime.drainLevels()}`.
   Nothing else in the file changes. `MasterMeter` calls `drain()` with no arguments, once
   per frame (`MasterMeter.svelte:50`), and `drainLevels` uses no `this`
   (`runtime.ts:1056`).

5. `bun run format`, `bun test tests/app.test.ts`, then commit (below), then the full gate with
   `N=003` and the Validation steps.

## Tests

Four new tests in `describe('createApp options')`: the given backends receive the runtime's
calls; given backends without `rust` build no Rust backend; previews read the given
superdough's `graph()`; previews call a `prepareAudition` replaced after `createApp` (the
call-time read). The 58 existing tests keep every assertion; only their `createApp` calls
change form. No test patches `drainLevels`, and there is no Svelte component harness (reverify
(b)), so the `drain` arrow is proved by the markup script and by the reasoning in step 4.

## Validation

- The full gate steps 1 to 4 pass (000-prompt.md). `bun test`'s log shows
  `tests/app.test.ts` with 62 passing tests.
- Step 5, the proofs:
  - `L js bun "$S/0136-js.ts" 003`: exit 0, `equal:` for all five files, `app.ts` with 6
    planned replacements;
  - `L tests bun "$S/0136-003-tests.ts"`: exit 0,
    `equal: 9a5b6ff with the 2 imports, the 12 calls and the appended describe`;
  - `L markup bun "$S/0136-markup.ts" --drain`: exit 0, `markup equal: 9a5b6ff with the 3
    allowed replacements` and `file equal: …`;
  - `L literals "$S/0136-001-literals.sh"`: still exit 0;
  - `L callers git grep -n "createApp(" -- src tests`: `src/App.svelte:25` (`createApp()`),
    two lines in `src/lib/app.ts` (the `AppOptions` doc comment's `createApp()` and the
    definition), and 13 lines in `tests/app.test.ts`, each `createApp({` (the 12 rewritten
    calls and `appWith`'s). Nothing else.
- Step 6 lists exactly 002's nine files plus `tests/app.test.ts`, in git's path order:
  `AGENTS.md`, `src/components/StatusBar.svelte`, `src/lib/app.ts`, `src/lib/audio/backend.ts`,
  `src/lib/audio/superdough.ts`, `src/lib/status-notices.ts`, `tests/app.test.ts`,
  `tests/helpers/fake-runtime.ts`, `tests/status-notices.test.ts`, `tests/superdough.test.ts`.
- Step 7 is unchanged from 002's.

## Commit message

```
refactor(app): take createApp's seams as one options object

createApp takes an optional AppOptions object in place of five
positional parameters, with the same defaults, and an optional backends
field for tests. prepareAudition and drainLevels are read when called,
so a replaced method is the one used. The runtime is still built inside
createApp; the app's own createApp() call is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: `0136-003-tests.ts`

```ts
// tests/app.test.ts must equal 9a5b6ff's with exactly 003's edits: the two type imports, the 12
// positional `createApp` calls rewritten as options objects (then prettier-formatted, as
// `bun run format` leaves them), and one new `describe('createApp options')` appended at the end.
// Usage: bun 0136-003-tests.ts [file]. Exit 0 equal, 1 different, 2 setup error.
import { $ } from 'bun';

const REPO = '/Volumes/rudenet/code/strudeland-worktrees/poptart';
const FILE = 'tests/app.test.ts';
const IMPORT = "import type { EditorView } from '@codemirror/view';\n";
const IMPORTS =
  IMPORT +
  "import type { RustBackend } from '../src/lib/audio/rust';\n" +
  "import type { SuperdoughBackend } from '../src/lib/audio/superdough';\n";
/** Positional call, its options form, and how many times it occurs at 9a5b6ff. */
const CALLS: [string, string, number][] = [
  [
    'createApp(api, () => opts.tauri ?? true, null, null, null)',
    'createApp({ api, inTauri: () => opts.tauri ?? true, controlApi: null, recordingApi: null, prefsApi: null })',
    1,
  ],
  [
    'createApp(api, () => true, null, null, null)',
    'createApp({ api, inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: null })',
    2,
  ],
  [
    'createApp(api, () => true, control.api, null, null)',
    'createApp({ api, inTauri: () => true, controlApi: control.api, recordingApi: null, prefsApi: null })',
    3,
  ],
  ['createApp(api, () => false)', 'createApp({ api, inTauri: () => false })', 1],
  [
    'createApp(api, () => true, null, rec.api, null)',
    'createApp({ api, inTauri: () => true, controlApi: null, recordingApi: rec.api, prefsApi: null })',
    1,
  ],
  [
    'createApp(api, () => true, null, null, prefs)',
    'createApp({ api, inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: prefs })',
    1,
  ],
  [
    'createApp(apiFor(), () => true, null, null, prefs)',
    'createApp({ api: apiFor(), inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: prefs })',
    2,
  ],
  [
    'createApp(apiFor(), () => true, null, null, null)',
    'createApp({ api: apiFor(), inTauri: () => true, controlApi: null, recordingApi: null, prefsApi: null })',
    1,
  ],
];
const APPENDED = "\ndescribe('createApp options', () => {\n";

try {
  let text = await $`git -C ${REPO} show 9a5b6ff:${FILE}`.text();
  const count = (s: string) => text.split(s).length - 1;
  if (count(IMPORT) !== 1) throw new Error('want one EditorView import');
  text = text.replace(IMPORT, IMPORTS);
  for (const [from, to, n] of CALLS) {
    if (count(from) !== n) throw new Error(`base has ${count(from)} of ${from}, want ${n}`);
    text = text.split(from).join(to);
  }
  if (count('createApp(') !== 12) throw new Error(`base calls: ${count('createApp(')}, want 12`);
  const expected =
    await $`cd ${REPO} && bunx prettier --stdin-filepath ${FILE} < ${new Response(text)}`.text();
  const actual = await Bun.file(process.argv[2] ?? `${REPO}/${FILE}`).text();
  let ok = true;
  if (!actual.startsWith(expected)) {
    ok = false;
    const a = expected.split('\n');
    const b = actual.split('\n');
    const i = a.findIndex((line, k) => line !== b[k]);
    console.log(`first difference at line ${i + 1}:\n- ${a[i]}\n+ ${b[i]}`);
  } else {
    const rest = actual.slice(expected.length);
    const calls = rest.split('createApp(').length - 1;
    const describes = rest.split('\ndescribe(').length - 1;
    if (!rest.startsWith(APPENDED) || describes !== 1 || calls !== 1) {
      ok = false;
      console.log(`appended part: want one describe('createApp options') with one createApp call;`);
      console.log(
        `got ${describes} describe(s), ${calls} call(s), starting ${JSON.stringify(rest.slice(0, 60))}`,
      );
    }
  }
  if (ok) console.log('equal: 9a5b6ff with the 2 imports, the 12 calls and the appended describe');
  process.exit(ok ? 0 : 1);
} catch (e) {
  console.error(String(e));
  process.exit(2);
}
```
