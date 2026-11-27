# 0136: the app seams

## Goal

Three small frontend refactors, run in order as three stages. None of them changes sound or
anything visible.

1. **(b) Status notices.** `StatusBar.svelte` decides the order, copy and precedence of its
   notices in about 30 lines of `{#if}` blocks (`:66-93` at `9a5b6ff`). Rows 1-5, 7 and 8 and
   the order are untested (reverify §b). After this change one pure function,
   `statusNotices` in `src/lib/status-notices.ts`, returns the ordered notices with stable ids,
   text, tone, title and action kind. The component renders the one keyed `{#each}` it already
   has, and keeps the action controls, their help hints and the separate "audio suspended"
   button. Table tests cover every source and every precedence pair.
2. **(c) `SuperdoughBackend`.** `graph` and `masterLatencySec` are optional on `AudioBackend`,
   but only superdough has them and only `app.ts` reads them (reverify §c). They move to a new
   `SuperdoughBackend` type in `superdough.ts` (as `RustBackend` lives in `rust.ts`);
   `createSuperdoughBackend` returns it; `app.ts` drops its `?.` calls and its ad hoc structural
   type; `fakeRuntime` loses its `graph` option, which only its own self-test used. Types only:
   the transpiled JavaScript of `backend.ts`, `superdough.ts` and `runtime.ts` is unchanged, and
   `app.ts`'s changes in exactly the three dropped `?.` call sites.
3. **(a) `createApp` options.** The five positional optional parameters (`app.ts:90-96`) become
   one options object with the same defaults, plus an optional `backends` field. The 12
   positional calls in `tests/app.test.ts` are rewritten; `App.svelte`'s `createApp()` already
   matches the new signature. `prepareAudition` and `drainLevels` are read at call time. The
   runtime is still built inside `createApp`.

## Background

[reverify.md](reverify.md) holds the evidence at `9a5b6ff` (origin/main), and its **Decisions**
section is binding:

- (b): a pure `statusNotices(...)` with table tests; order, copy, tones and either/or pairs
  exact; "audio suspended" stays its own button outside the list; the `title` tooltips are
  unchanged; each notice gets a stable id; the Settings `prefs:` notice is out of scope.
- (c): `SuperdoughBackend` carries `graph` and `masterLatencySec`, both leave `AudioBackend`;
  `createSuperdoughBackend` returns it, `app.ts` drops the `?.`, `fakeRuntime` loses its
  `graph` option; no preview-graph port; `prepareAudition` keeps superdough hard-coded.
- (a): one options object with the same defaults and an optional `backends` field;
  `prepareAudition` is read when it is called; the runtime is still built inside `createApp`.

Line numbers below were re-checked at `9a5b6ff` (the branch adds only `plans/` on top).

## Scope

In:

| Stage | Files |
|---|---|
| [001](001-status-notices.md) (b) | new `src/lib/status-notices.ts`, new `tests/status-notices.test.ts`, `src/components/StatusBar.svelte` (script edits plus the allowed markup replacements), `AGENTS.md` (one new bullet) |
| [002](002-superdough-backend.md) (c) | `src/lib/audio/backend.ts`, `src/lib/audio/superdough.ts`, `src/lib/app.ts`, `tests/helpers/fake-runtime.ts`, `tests/superdough.test.ts` (the self-test of the removed option and its import go), `AGENTS.md` (the `src/lib/audio/` sentence) |
| [003](003-app-options.md) (a) | `src/lib/app.ts`, `tests/app.test.ts`, `src/components/StatusBar.svelte` (the one `drain` line) |

Out:

- `Settings.svelte`'s duplicate `prefs: {$ps.error}` notice (reverify Decision b6).
- Moving the `title` tooltips to `help`, or making the "audio suspended" button a notice kind.
- A preview-graph port; any edit to `src/lib/strudel/runtime.ts`. It is a host-bundle input
  (`crates/strudeland-host/js/host.inputs.json:108`), so an edit would need `host:build`. The
  doc line reverify (c) suggested for `prepareAudition` goes on `SuperdoughBackend` instead.
- Making `readScope`, `onLevels`, `setLimiter` or `limiterAvailable` required on
  `SuperdoughBackend`. The runtime reads them generically; it stays optional (open question 3).
- Injecting the runtime into `createApp`, or removing the `runtime.stop` monkeypatches
  (reverify (a): the canvas drawer is the reason, not a backend).
- Switching the 12 existing `createApp` test calls to fake backends (open question 1).

## Technical design

### 001: `statusNotices`

```ts
// src/lib/status-notices.ts
export interface StatusNoticesInput {
  runtime: Pick<RuntimeState, 'evalError' | 'outputError' | 'tempoOverride' | 'engineWarnings'>;
  session: Pick<SessionState, 'error' | 'metaError' | 'notice'>;
  prefs: Pick<PrefsState, 'error'>;
  control: ControlInfo;
  recording: RecordingUiState | null;
  player: MemoPlayerState | null;
  /** Absent: `navigator.platform`, as `recordingNotices` defaults it. */
  platform?: string;
}

export interface StatusNotice {
  id: string;
  tone: RecordingNotice['tone']; // 'warning' | 'danger'
  text: string;
  title?: string;
  action?: NoticeAction;
}

export function statusNotices(input: StatusNoticesInput): StatusNotice[];
```

It lives at `src/lib/` next to `play-arbiter.ts` and `shell.ts`, the other modules that
compose several domains. It follows the extracted helpers' pattern (`engineNotices` in
`src/lib/audio/engine-notices.ts`, `recordingNotices` in `src/lib/recording/memo-ui.ts:305`):
pure, plain snapshots in, plain objects out, no Tauri, DOM or audio imports. It composes both
helpers unchanged. Every import except those two is `import type`, so `session.ts` and
`runtime.ts` (CodeMirror, Strudel) are not loaded.

The rows, in order, with the conditions as the component tests them today (truthiness for
`{#if}`, so an empty string shows nothing):

| # | Condition | id | tone | text | title |
|---|---|---|---|---|---|
| 1 | `runtime.evalError` | `eval-error` | danger | `error: {evalError}` | |
| 1b | else `runtime.outputError` | `output-error` | danger | `audio error: {outputError}` | |
| 2 | `session.error` | `session-error` | danger | `{error}` | |
| 3 | `session.metaError` | `meta-error` | danger | `meta: {metaError}` | |
| 4 | `prefs.error` | `prefs-error` | danger | `prefs: {error}` | |
| 5 | `runtime.tempoOverride` | `tempo-override` | warning | `setcps in the code overrides meta.bpm` | |
| 6 | each of `engineNotices(runtime.engineWarnings)` | `engine:{signature}` | warning | `rust engine skipped {signature}` | `keys: a, b`, only with keys |
| 7 | `session.notice` | `claude-edit` | warning | `Claude edited {notice.title}` | |
| 8 | `control.status === 'in-use'` | `control-in-use` | warning | `control socket in use` | `path ?? ''`, always set |
| 8b | else `control.status === 'disabled'` | `control-disabled` | warning | `control disabled: {detail ?? ''}` | |
| 9 | `recordingNotices(recording, player, platform)` | its ids | its tones | its texts | |

Two details keep the DOM byte-identical:

- **The control-in-use title is always set.** Today `title={$cs.path ?? ''}` renders
  `title=""` when the path is null. The function returns `title: ''` then, and every other
  notice has no `title` key, so `title={n.title}` renders no attribute for them, as today.
- **A null detail renders as nothing.** Svelte renders `{$cs.detail}` as `''` for null; a
  template literal would write `null`. The function writes `${control.detail ?? ''}`.

`StatusBar.svelte` keeps its markup except two replacements: the ten `{#if}`/`{#each}` notice
blocks before the recording list go (`:66-93`), and the plain recording notice
(`:124`) gains `title={n.title}`. The action branch (`:95-122`), with the three help hints and
the `memos?.stopPlayback()` and `recording?.clearError()` handlers, stays byte-identical, and so
does the "audio suspended" button (`:127-135`). The script swaps the two helper imports for
`statusNotices` and turns `const notices = $derived(recordingNotices(...))` (`:61`) into one
`$derived(statusNotices({ ... }))` over the six store snapshots. The name `notices` is kept, so
the `{#each notices as n (n.id)}` line is unchanged.

Keys: the recording ids (`permission`, `no-device`, `device-lost`, `limit-reached`,
`recording-error`, `recording-warning`, `bluetooth`, `play-*`) cannot collide with the new ones.
Engine signatures are deduplicated per play by the runtime, and today's `{#each}` is already
keyed by signature, so a duplicate would already throw. A source whose message changes keeps its
id and updates in place, as the `{#if}` blocks do. Today's either/or pairs (1/1b and 8/8b)
switch branch, so they remount; with different ids they still do.

### 002: `SuperdoughBackend`

```ts
// src/lib/audio/superdough.ts, before createSuperdoughBackend
export interface SuperdoughBackend extends AudioBackend {
  graph(): AudioGraph | null; // the doc comment moves from AudioBackend
  masterLatencySec(): number; // likewise
}
export function createSuperdoughBackend(deps: Deps = defaultDeps): SuperdoughBackend;
```

The interface's doc comment says previews play on this backend even while the Rust engine plays
the song (what reverify (c) wanted on `prepareAudition`, which stays untouched). `AudioBackend`
loses the two members and their doc comments (`backend.ts:50-55`, `:66-67`); `AudioGraph`
stays there, since every consumer imports it from there. In `app.ts`:
`superdough.graph?.() ?? null` becomes `superdough.graph()` (`:127`), `createRecording`'s
`backend` parameter becomes `SuperdoughBackend` (`:304`), its `graph` becomes `backend.graph()`
(`:309`), and `outputDelaySec` becomes `() => backend.masterLatencySec()` (`:322`). The now
unused `AudioGraph` import (`:4`) goes. `fakeRuntime` loses `opts.graph` (`:17-18`), its block
(`:78-83`) and the `AudioGraph` import; `tests/superdough.test.ts` loses the self-test
(`:362-376`, with the blank line before it) and the `fakeRuntime` import (`:6`).

Sound stays the same. The following stay as they are, and the tests that pin them stay
unchanged:

- `graph().destination` is superdough's `destinationGain`, so previews, the metronome and memos
  still feed the master tap (`superdough.test.ts:85-160`);
- `graph()` is null before audio runs and during a `reset()` (same tests);
- `outputDelaySec` is `masterLatencySec()` (`:217-229`, `:239-328`);
- previews resume superdough while the Rust engine plays (`runtime.ts:1028-1047`, untouched;
  `runtime-backends.test.ts:366-378`).

Dropping `?.` and `?? null`/`?? 0` cannot change a value. The real backend always has both
members, `graph()` returns `AudioGraph | null` (never undefined), and `master.latencySec()`
returns a number (`master-tap.ts:81`).

### 003: `createApp(options)`

```ts
export interface AppOptions {
  api?: ProjectApi;
  inTauri?: () => boolean;
  controlApi?: ControlApi | null;
  recordingApi?: RecordingApi | null;
  prefsApi?: PrefsApi | null;
  backends?: { superdough: SuperdoughBackend; rust?: RustBackend };
}

export function createApp(options: AppOptions = {}): App {
  const {
    api = createTauriProjectApi(),
    inTauri = isTauri,
    controlApi = inTauri() ? createTauriControlApi() : null,
    recordingApi = isTauri() ? createTauriRecordingApi() : null,
    prefsApi = inTauri() ? createTauriPrefsApi() : null,
  } = options;
  // ... unchanged until today's :106-108:
  const { superdough, rust } = options.backends ?? {
    superdough: createSuperdoughBackend(),
    rust: inTauri() ? createRustBackend(tauriPlaybackApi) : undefined,
  };
```

Destructuring defaults behave like the parameter defaults. Each applies only for `undefined`,
so `null` still means "none". They are evaluated left to right, so `controlApi` and `prefsApi`
see the given `inTauri`, and `recordingApi` still reads the real `isTauri()`. They run before the
body, as before. The backends are built at the same point as today, and only when `backends` is
absent. Given `backends` without `rust`, no Rust backend is built, even in Tauri.

`prepare: runtime.prepareAudition` (`:137`, `:152`) becomes `prepare: () =>
runtime.prepareAudition()`, and `StatusBar.svelte`'s `drain={runtime.drainLevels}` becomes
`drain={() => runtime.drainLevels()}`. Neither method uses `this` (`runtime.ts:1028`, `:1056`),
both are called with no arguments (`note-audition-queue.ts:49`, `instrument-preview.ts:42`,
`MasterMeter.svelte:50`), and the meter calls `drain()` each frame, so only the time of the
property read changes.

## Proofs

Four proofs back the test suite. Each is a script in the scratchpad, written and dry-run
when this plan was written; the scripts are in the stage appendices.

| Proof | Stage | What it checks |
|---|---|---|
| `0136-001-literals.sh` | 001, 003 | Literal counts in `StatusBar.svelte` and `status-notices.ts` at `9a5b6ff` and in the tree, row by row: every copy string, tone, title, id and help hint, and the structure counts (`<Notice`, `{#if`, `$rt.` …) |
| `0136-markup.ts` | 001, 003 (`--drain`) | Rebuilds the whole expected `StatusBar.svelte` from `9a5b6ff` with exactly the planned script edits and allowed replacements, and byte-compares the markup, then the whole file |
| `0136-js.ts 002\|003` | 002, 003 | Transpiles (`bun build --no-bundle`: types and comments stripped) each file at `9a5b6ff` and in the tree. `backend.ts`, `superdough.ts` and `runtime.ts` must be equal; `app.ts` and `fake-runtime.ts` must differ by exactly the planned code replacements |
| `0136-003-tests.ts` | 003 | `tests/app.test.ts` equals `9a5b6ff` with the two type imports and the 12 calls rewritten (prettier-formatted), plus one appended `describe` |

### Allowed markup replacements (`StatusBar.svelte`)

Everything after `</script>` is byte-identical to `9a5b6ff` except:

1. (001) `:66-93`, from `    {#if $rt.evalError}` through the `{/if}` that closes the control
   notices, are deleted.
2. (001) `:124` `        <Notice tone={n.tone}>{n.text}</Notice>` becomes
   `        <Notice tone={n.tone} title={n.title}>{n.text}</Notice>`.
3. (003) `:167` `        drain={runtime.drainLevels}` becomes
   `        drain={() => runtime.drainLevels()}`.

All three are prettier-stable. The expected files were checked with the repo's `.prettierrc`
when the plan was written.

## Host bundle and planner files

No touched file is in `crates/strudeland-host/js/host.inputs.json` or in `PLANNER_FILES`
(`scripts/engine/provenance.ts:30-39`). `runtime.ts` is a host input, so it stays frozen in
every stage. `backend.ts` is not an input: its imports are type-only and erased. No stage runs
`host:build` or `engine:refs`, and no cargo gate (no Rust changes).

## Risks

- **A changed string, tone, title or order.** The literal script, the markup rebuild and the
  table tests (literal expected strings, written out, never computed with the function) catch
  it.
- **`title=""` lost or `null` written.** Both are pinned by tests (control in-use with a null
  path; control disabled with a null detail).
- **Per-update recomputation.** The notices now recompute on any change to the six stores,
  `$ps` included. Today the runtime-store blocks and `engineNotices(...)` already rerun on every
  `$rt` change. Keyed items with unchanged text keep their DOM, so nothing visible changes.
- **A runtime change hidden in a "type-only" stage.** The transpile proof fails on any code
  difference beyond the listed ones.
- **A default evaluated differently.** The transpile proof pins the destructuring (same
  expressions, same order). The existing `app.test.ts` tests run unchanged in substance on the
  new calls.

## Stages

Sequential: [000-prompt.md](000-prompt.md) is the entry point, then
[001-status-notices.md](001-status-notices.md), [002-superdough-backend.md](002-superdough-backend.md)
and [003-app-options.md](003-app-options.md), one commit each, the full gate once per stage. 002
and 003 both edit `app.ts:4-11`, `:90-128` and `:302-322`, so they never run in parallel.

## Open questions

1. Reverify (a) gave the reason for `backends` as: "tests then use fake backends and no longer
   build the real `tauriPlaybackApi`". This plan rewrites the 12 existing calls mechanically,
   so they keep the real backends, and it adds four new tests that inject fakes. Moving the
   existing tests to fakes would change what they exercise (the real superdough backend under
   happy-dom). Should a follow-up do it?
2. The brief counts "13 test calls". There are 12 in `tests/app.test.ts` (`:69`, `:260`, `:417`,
   `:440`, `:456`, `:474`, `:513`, `:615`, `:821`, `:1026`, `:1039`, `:1050`); the 13th call is
   `App.svelte:25`. `createApp()` still type-checks, so `App.svelte` stays untouched. Is that
   acceptable, or should it spell out `createApp({})`?
3. `SuperdoughBackend` adds only `graph` and `masterLatencySec`. Should it also make
   `readScope`, `onLevels`, `setLimiter` and `limiterAvailable` required, as `RustBackend` does
   for its hooks? Nothing needs it now, and the test fakes would grow.
4. Nobody starts the app (by rule). The markup proof replaces a visual check. The user may want
   to look at the status bar by hand before the merge: an eval error, a Claude edit, a
   recording notice with its action, and "audio suspended".
