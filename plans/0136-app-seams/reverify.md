# 0136 app seams: step 0 re-verification

Checked on `llm/andrew/strudeland/0136-app-seams` at `9a5b6ff` (origin/main), 31 commits after
63b945d. Read only. The app was not run.

The three parts are independent and share no files except `src/lib/app.ts`, which (a) and (c)
both touch. Each part is small.

## (a) `createApp` seams

**Still there, and the same size.** The signature is now `src/lib/app.ts:90-96`: five
positional optional parameters (`api`, `inTauri`, `controlApi`, `recordingApi`, `prefsApi`). The
function builds the superdough backend (`:106`), the Rust backend over the real
`tauriPlaybackApi` when `inTauri()` (`:108`) and the runtime (`:109-116`) itself. #128 added
`migrateLimiter` (`:203`) and the limiter subscription (`:195`), but the signature did not change.

Other things `inTauri()` turns on with no seam: `inspectPianoRoll` (`:98`), the Rust backend
(`:108`) and `syncWindowTitle` (`:208`). `recordingApi` defaults on the real `isTauri()`, not on
`inTauri()` (`:94`). The doc comment at `:86-87` says this is on purpose.

**Callers**

| Caller | Call |
|---|---|
| `src/App.svelte:25` | `createApp()` (all defaults) |
| `tests/app.test.ts` (58 tests) | 13 calls: `:69` (`setup`), `:260`, `:417`, `:440`, `:456`, `:474`, `:513`, `:615`, `:821`, `:1026`, `:1039`, `:1050`. Most pass `null, null, null`. |

**Monkeypatches on the returned object** (`tests/app.test.ts`)

- `runtime.stop`: `:71`, `:261`, `:418`, `:441`, `:616`, `:734`, `:770`, `:822`. The comment says
  the real stop "draws on a canvas happy-dom lacks". That drawing comes from the visuals hooks,
  not from a backend.
- `runtime.evaluate`: `:72`, `:617`. `runtime.resumeAudio`: `:712`, `:729`, `:769`, `:787`.
  `runtime.readScope`: `:547`.
- The same style is used on other objects: `session.saveAndEvaluate` (`:322`, `:680`, `:711`),
  `session.playback.start` (`:730`), `recording.stop` (`:645`, `:745`), `instrumentPreview.stop`
  (`:659`), `scopeClock.dispose` (`:592`).
- **A latent trap.** `prepare: runtime.prepareAudition` (`app.ts:137`, `:152`) and
  `drain={runtime.drainLevels}` (`StatusBar.svelte:167`) capture the method itself. A patch on
  those methods would not reach them. No test patches them today.

**Why injecting `runtime` does not fit.** The runtime needs hooks that only the app has:
`visuals`, `onPlaybackRequest: stopAuditions` (`:115`), and a prebake that calls
`captureAuditionSynths` (`:111-114`). A runtime built outside `createApp` would have to rebuild
that wiring in each test. `tests/helpers/stub-runtime.ts` (196 lines, used by 9 test files) is a
hand-written `Runtime`. The app tests rely on the real scheduler and store behaviour
(`schedulerStarted`, `store.started`), so the stub would not cover them.

**Recommendation: re-scope.** Keep these two changes:

1. Replace the five positionals with one options object:
   `createApp({ api, inTauri, controlApi, recordingApi, prefsApi })`, with the same defaults.
2. Add an optional `backends` field (`{ superdough, rust? }`). Tests then use fake backends and
   no longer build the real `tauriPlaybackApi`.

Leave the runtime built inside `createApp`. The `runtime.stop` patch for the canvas stays; it is
a drawer problem, not a seam problem. Fix the `runtime.prepareAudition` capture at the same time
by using `() => runtime.prepareAudition()`, so it reads the property at call time.

**Visible behaviour.** None. `App.svelte` keeps calling `createApp()` with the same defaults,
including `recordingApi` on the real `isTauri()`.

## (b) Notice precedence in `StatusBar.svelte`

**Still there.** The block has moved to `src/components/StatusBar.svelte:65-135`. Its order
and copy are unchanged since 63b945d. 8fce514 (contextual help) added `help` hints to the action
controls and wrapped the block in `div.notices`.

**Current order, exactly**

| # | Source store | Condition | Tone | Text | Extra |
|---|---|---|---|---|---|
| 1 | runtime `evalError` | if | danger | `error: {evalError}` | |
| 1b | runtime `outputError` | else-if (hidden while 1 shows) | danger | `audio error: {outputError}` | |
| 2 | session `error` | if | danger | `{error}` | |
| 3 | session `metaError` | if | danger | `meta: {metaError}` | |
| 4 | prefs `error` | if | danger | `prefs: {error}` | |
| 5 | runtime `tempoOverride` | if | warning | `setcps in the code overrides meta.bpm` | |
| 6 | runtime `engineWarnings` via `engineNotices` (`src/lib/audio/engine-notices.ts`) | each, key `signature` | warning | `rust engine skipped {signature}` | `title` = `keys: a, b` when there are keys |
| 7 | session `notice` | if | warning | `Claude edited {title}` | |
| 8 | control `status === 'in-use'` | if | warning | `control socket in use` | `title` = `path ?? ''` |
| 8b | control `status === 'disabled'` | else-if | warning | `control disabled: {detail}` | |
| 9 | recording and memo player via `recordingNotices` (`memo-ui.ts:305`) | each, key `id` | per notice | per notice | action `link`: `Link` with help "Open the microphone permissions for this computer."; action `dismiss-playback`: `Button` "dismiss" with help "Dismiss this memo playback message." calling `memos.stopPlayback()`; any other action: `Button` "dismiss" with help "Dismiss this recording message." calling `recording.clearError()` |
| 10 | runtime `audio === 'suspended'` | if | (a `Button`, not a `Notice`) | `audio suspended: click to enable` | help "Resume the audio output so you can hear playback."; calls `runtime.resumeAudio()` |

`Notice` sets `data-tone={tone}` and `role` from `noticeRole` (`ui/notice.ts`): danger is
`alert`, anything else is `status`.

**Tests today.** `tests/engine-notices.test.ts` (3 tests) and `tests/memo-ui.test.ts` (60) test
the extracted parts. Nothing tests the order or rows 1-5, 7 and 8. There is no Svelte component
test setup (no `mount(` in `tests/`).

**New copy since 63b945d.** `Settings.svelte:54-56` repeats `prefs: {$ps.error}` (added in
8fce514).

**Recommendation: proceed as written.** Add one pure `statusNotices(runtime, session, prefs,
control, recording, player, platform?)` that returns `{ id, tone, text, title?, action? }[]`,
with table tests. It can live in `src/lib/status-notices.ts` and compose `engineNotices` and
`recordingNotices`. `StatusBar` then renders one keyed `{#each}`.

Decisions, each with a recommended answer:

1. Order, copy, tones and exclusivity: keep exactly as in the table, including 1/1b and 8/8b.
2. The `audio suspended` button: keep it as a separate `Button` after the list, not a notice
   kind. It is a control, and it has its own help hint.
3. The `title` tooltips on rows 6 and 8: keep them as `title`. Do not move them to `help` in this
   task.
4. The action hints: keep the three texts and their mapping to action kinds in the component.
   The pure function only returns the action kind.
5. Keys: use stable ids per source (`eval-error`, `output-error`, `session-error`, `meta-error`,
   `prefs-error`, `tempo-override`, `engine:{signature}`, `claude-edit`, `control-in-use`,
   `control-disabled`, plus the existing recording ids unchanged). A message that changes then
   updates in place, as the `{#if}` blocks do today.
6. The duplicate in `Settings.svelte`: leave it out of scope.

## (c) `AudioBackend` optional members

**Still there, and slightly bigger: 10 optional members now, not 8.** `src/lib/audio/backend.ts`
(73 lines): `resumeTimeoutMs`, `readScope`, `graph`, `liveStats`, `onWarning`, `onLevels`,
`setLimiter`, `limiterAvailable`, `masterLatencySec` and `stop`.

**Who implements what**

| Implementer | Optional members it has |
|---|---|
| superdough `src/lib/audio/superdough.ts:53` (returns plain `AudioBackend`) | `readScope`, `onLevels`, `setLimiter`, `limiterAvailable`, `masterLatencySec`, `graph` (`:91-118`) |
| rust `src/lib/audio/rust.ts:271` (`RustBackend` = `AudioBackend & Required<Pick<…>>`, `:49-56`) | `resumeTimeoutMs`, `readScope`, `liveStats`, `onLevels`, `setLimiter`, `limiterAvailable`, `onWarning`, `stop`. No `graph` and no `masterLatencySec`. |
| offline `src/lib/audio/offline.ts:51`, host `src/host/backend.ts:9` | none |
| `tests/helpers/fake-runtime.ts:51` (superdough fake) | `onLevels`, `setLimiter`, `limiterAvailable`, and `graph` when the `graph` option is set (`:77-82`) |
| `fake-runtime.ts:103` (rust fake) | the same set as rust |
| `tests/scope-tap.test.ts:235` (inline) | `readScope` |

**Who reads `graph` and `masterLatencySec`.** Only `app.ts` does. The runtime never calls
either.

- `app.ts:127` `auditionGraph = () => superdough.graph?.() ?? null` feeds note audition (`:128`)
  and instrument preview (`:153`). Instrument preview is a new consumer since 63b945d (07b072c).
- `app.ts:302-309` `createRecording(backend: { isRunning; graph?; masterLatencySec? })` is the ad
  hoc structural type. Its `graph` feeds the metronome (`:310`), the output clock (`:319`) and the
  memo player (`:331`). `masterLatencySec` feeds `outputDelaySec` (`:322`). `isRunning` feeds the
  memo player (`:332`).
- Every downstream module already takes a function port, `graph(): AudioGraph | null`:
  `note-audition.ts:45`, `instrument-preview.ts:13`, `metronome.ts`, `memo-player.ts`,
  `output-clock.ts`. Their tests pass `graph: () => …` directly (`instrument-browser`,
  `memo-player`, `metronome`, `output-clock`, `memos`, `note-audition-audio`, `play-arbiter`
  and `recording-controller` tests). None of these tests needs to change.
- The `graph` option of `fakeRuntime` is used only by its own self-test (`superdough.test.ts:363-377`).
- `superdough.test.ts:85-160` and `:217-328` call `be.graph!()` and `be.masterLatencySec!()`.

**`prepareAudition`** is now at `runtime.ts:1028-1047`, not `:1145`. It hard-codes
`opts.backends.superdough` at `:1030`. That is the right behaviour: previews play on
superdough's Web Audio graph even while the Rust engine plays the song, and the Rust engine has
no graph. The method also uses runtime internals (`active()`, `resumeAudio`, `resumeBackend`,
`ensurePrebaked`, the `auditionResume` dedupe), so it belongs in the runtime.

**Recommendation: re-scope to a type change.** A separate preview-graph port module is not
needed, because the consumers already take a `graph` function.

1. Add `SuperdoughBackend = AudioBackend & { graph(): AudioGraph | null; masterLatencySec():
   number; … }`, modelled on `RustBackend`, and return it from `createSuperdoughBackend`.
2. Remove `graph` and `masterLatencySec` from `AudioBackend`. Nothing generic reads them.
3. `createRecording` and `auditionGraph` take the `SuperdoughBackend` and lose the `?.`.
4. Remove the dead `graph` option of `fakeRuntime`, or move its self-test.
5. Leave `prepareAudition` where it is. Only add a doc line saying previews always play on
   superdough.

The other 8 optional members are read generically by the runtime (`:604`, `:684-698`,
`:785-786`, `:1055`, `:1064`), so they stay optional.

**Sound decisions, each with a recommended answer.** Routing must not change.

1. `graph().destination` stays superdough's `destinationGain`, so previews, the metronome and
   memos keep feeding the master tap.
2. `graph()` keeps returning null before audio runs and during a `reset()`.
3. `outputDelaySec` stays `masterLatencySec()`.
4. Previews keep resuming superdough while the Rust engine is active.

## Splitting

Do the three parts as three small PRs, or as separate tasks, in this order: (b) first (only
`StatusBar.svelte` and a new pure module), then (c), then (a). (c) and (a) both edit
`app.ts:106-128` and `:302-309`, so do them one after the other, never in parallel. None of the
three changes sound or visible UI.

## Decisions (2026-11-27, recommended answers taken; no visible or sound change)

- **Structure:** one task in three stages run in order: (b) status notices, (c) the backend type, (a) the app options. (c) and (a) both edit the same part of `app.ts`, so they run one after the other.
- **(b) Status notices:**
  - A pure `statusNotices(...)` with table tests replaces the inline order.
  - The order, copy, tones and either/or pairs stay exact.
  - "Audio suspended" stays its own button outside the list.
  - The `title` tooltips are unchanged.
  - Each notice gets a stable id.
  - The Settings `prefs:` notice is out of scope.
- **(c) Backend type:**
  - A new `SuperdoughBackend` type carries `graph` and `masterLatencySec`; both leave `AudioBackend`.
  - `createSuperdoughBackend` returns the new type, `app.ts` drops the `?.`, and `fakeRuntime` loses its `graph` option.
  - No new preview-graph port, and `prepareAudition` keeps superdough hard-coded.
- **(a) App options:**
  - `createApp` takes one options object with the same defaults and an optional `backends` field.
  - `prepareAudition` is read when it is called.
  - The runtime is still built inside `createApp`.
