# 003: the limiter preference in prefs

## Goal

The limiter preference moves from `localStorage` into `prefs.json`. `src/lib/prefs/store.ts`
then holds the only frontend copy of every preference. The status bar toggles through prefs,
`app.ts` applies prefs to the runtime, and the runtime keeps `state.limiter` as the applied
value. A one-time migration in `src/lib/prefs/migrate.ts` moves the stored value. The key is
removed only after the save succeeds. `src/lib/audio/limiter-pref.ts` is deleted.

The visible changes are exactly the accepted ones (proposal, Goal):

- the limiter does not persist in a plain browser;
- it is known once `prefs_get` resolves;
- a failed save shows the "prefs: …" notice;
- the one-time migration.

## Prerequisites

- Stages 001 and 002 committed and green.
- The proposal section "The limiter (stage 003)" and its migration table.

## Files

- New: `src/lib/prefs/migrate.ts`.
- Edited:
  - `crates/strudeland-project/src/prefs.rs`;
  - `src/lib/prefs/{api.ts,store.ts}`;
  - `src/lib/app.ts`;
  - `src/components/StatusBar.svelte`;
  - `src/lib/strudel/runtime.ts`, `src/lib/audio/rust.ts` and `tests/helpers/fake-runtime.ts`
    (doc comments only);
  - `tests/{prefs,app,runtime-levels,ipc-adapters}.test.ts`;
  - `AGENTS.md`.
- Deleted: `src/lib/audio/limiter-pref.ts`.
- Untouched (besides the shared list):
  - `crates/strudeland-record/`;
  - `src-tauri/`;
  - `crates/strudeland-project/src/{app_data,recents,recording_input,instruments}.rs`;
  - `src/components/ui/MasterMeter.svelte`;
  - `tests/helpers/fake-prefs-api.ts` (it spreads `DEFAULT_PREFS`, so it gains the field by
    itself).

## Steps

1. **Rust `Prefs`**: add `/// The master safety limiter on the live output.` `pub limiter: bool`
   after `piano_roll_open`. Nothing else in Rust changes. `prefs_set` deserializes it, and
   `bindings.ts` types `prefs` as `unknown`.
2. **`api.ts`**: `limiter: boolean` in `Prefs` (last, matching Rust), and `limiter: false` in
   `DEFAULT_PREFS`.
3. **`store.ts`**:
   - `PrefsStore` gains:
     - `limiter(): boolean` ("The current value, synchronously");
     - `setLimiter(on: boolean): Promise<boolean>`;
     - `edited(key: keyof Prefs): boolean` ("Whether this session set `key`; a value the user
       chose before the load wins over a migration");
   - every setter's type becomes `Promise<boolean>`, and its doc changes from "Never rejects."
     to "Resolves whether the save landed; never rejects.". The callers that `void` the promise
     are unaffected, but `app.toggleVim` is not: it returns `prefs.setVim(...)` as its
     `Promise<void>` (`app.ts:71,256-259`, `StatusBar.svelte:41`), and a `Promise<boolean>` does
     not assign to it. Step 5 changes its body;
   - in `set`: the queued step returns `true` after `api.set` resolved and `false` in the
     `catch`. Without an api it resolves `false`. `chain` stays a `Promise<void>` derived from
     it, so the queue order is unchanged;
   - `edited` reads `key in edits`.

   The `load` path, the error handling and the queue order are otherwise unchanged.
4. **`migrate.ts`**: `migrateLimiter(prefs, storage)`, exactly the proposal's table:

   ```ts
   const KEY = 'strudeland.limiter';
   ```

   - read: `try { value = storage()?.getItem(KEY) ?? null } catch { return }`;
   - `null` returns at once;
   - a value other than `'1'`/`'0'` returns and leaves the key;
   - if `prefs.edited('limiter')` (the user toggled before the load), remove the key only
     `if (await prefs.setLimiter(prefs.limiter()))`, and return. This saves an identical object
     once more, so a toggle whose own save failed cannot lose the stored value: the key stays
     for the next launch;
   - otherwise `if (await prefs.setLimiter(value === '1'))`, remove the key;
   - every `removeItem` is wrapped in `try`, with `console.error('could not remove the old
     limiter preference', e)` and never a throw.

   The module doc says it is the only module that touches `localStorage`. Once no installed
   build has the key, the file can be deleted with its call. Say so in one line.
5. **`app.ts`**:
   - delete the `limiter-pref` import (`:30`), `limiter: loadLimiter()` (`:116`), `savedLimiter`
     and the two limiter lines in the runtime subscriber (`:119-125`). The subscriber keeps
     `scopeClock.setRunning(s.started)`;
   - the prefs subscriber (`:199-205`) takes `{ vim, auditionNotes, limiter }` and adds
     `if (limiter !== get(runtime.store).limiter) runtime.setLimiter(limiter);`. Import `get`
     from `svelte/store` if it is not already imported;
   - after `const ready = prefs.load();`:
     `if (prefsApi) void ready.then(() => migrateLimiter(prefs, () => globalThis.localStorage));`.
     `ready` is unchanged. Nothing waits on the migration;
   - `toggleVim` (`:256-259`) becomes
     `async toggleVim() { const saved = prefs.setVim(!prefs.vim()); view?.focus(); await saved; }`.
     The focus still happens before the save settles, and `toggleVim(): Promise<void>` in `App`
     (`:71`) and `StatusBar.svelte:41` stay as they are.
6. **`StatusBar.svelte`**: `onlimiter={prefs.setLimiter}`. `limiter={$rt.limiter}` and
   `available` stay.
7. **`runtime.ts`**: doc comments only.
   - `limiter` (`:141`): "The applied master limiter (the app sets it from prefs; applied to both
     backends)."
   - `setLimiter` (`:224`): "Applies the limiter to both backends. The app calls it from prefs."
   - The `limiter` option (`:241`): "The initial applied limiter (default off)."
   - Optional, the same wording elsewhere: `tests/helpers/fake-runtime.ts:27` ("The runtime's
     initial limiter preference." becomes "The runtime's initial applied limiter.") and
     `src/lib/audio/rust.ts:126` ("The limiter preference; …" becomes "The applied limiter; …").
8. **Delete** `src/lib/audio/limiter-pref.ts`.
9. **`AGENTS.md`**:
   - `:86-88`: drop "and `limiter-pref.ts` (the stored preference)", so the sentence ends at
     `master-tap.ts` (the superdough tap);
   - `:139`: the `src/lib/prefs/` bullet adds `migrate.ts` ("the one-time move of the old
     `localStorage` limiter value; the only `localStorage` caller"). The words "the one frontend
     copy of a pref" stay; they are now true for the limiter too.

## Tests

- **Rust `prefs.rs`:**
  - the stage 001 byte test gains `,\n  \"limiter\": true` / `false` as the last key of both
    literals. This is the intended byte change;
  - new `a_file_without_limiter_loads_it_off`: the stage 001 four-field all-true text loads with
    the four values `true` and `limiter == false`;
  - new: `{"schema":1,"limiter":true}` loads `limiter` alone;
  - `defaults_are_off` asserts `!limiter`.
- **`tests/prefs.test.ts`:**
  - every whole-`Prefs`/`PrefsState` `toEqual` literal gains `limiter: false`: `:26-31`,
    `:43-48`, `:81`, `:107`, `:120-125`, `:131`, `:133-138`, `:149`, `:161`, `:179`,
    `:199-204`, `:208-209`, `:228-233`, `:256-258`, `:260-265` and `:278`. The `fakePrefsApi`
    arguments (`:37`, `:184-189`) are partial and may stay;
  - new: `setLimiter(true)` updates `limiter()` and the store at once and saves
    `{ ...loaded, limiter: true }`;
  - new: `edited` is false at start, true for the key after its setter, and false for other keys;
  - new: a setter resolves `true` after a good save and `false` with `failSet`, and without an
    api (`createPrefs(null)`) it resolves `false`;
  - new `describe('limiter migration')`, here rather than in `app.test.ts` because it needs no
    DOM. It calls `migrateLimiter` directly with `createPrefs(fakePrefsApi())`, after
    `await prefs.load()`, and a fake storage `{ getItem, removeItem }` over a `Map`:
    - `'1'` → `limiter()` true, saved, key removed;
    - `'0'` over a loaded `limiter: true` → false, saved, key removed;
    - absent → no save, nothing removed;
    - `'yes'` → no save, key left;
    - `getItem` throws → no save, no throw;
    - `storage()` itself throws → the same;
    - `failSet` → `limiter()` true for the session, `store.error` is `'write failed'`, key kept;
    - toggle first (`setLimiter(false)` before `load` resolves) with `'1'` stored → the saved
      value is `false`, there are exactly two saves (the toggle's and the migration's identical
      one), and the key is removed;
    - toggle first with `'1'` stored and `failSet` → `limiter()` is the toggled `false`,
      `store.error` is `'write failed'`, no save landed, and the key is kept;
    - `removeItem` throws after a good save → saved, no throw, `console.error` called (spy).
- **`tests/ipc-adapters.test.ts`:** the `prefs` literal at `:48` gains `limiter: false`.
- **`tests/app.test.ts`:**
  - the literal at `:939` gains `limiter: false`;
  - new, in the prefs `describe`: with `withPrefs({ limiter: true })`, the runtime limiter is
    `false` before `gate.resolve()` and `true` after `await app.ready`;
  - new: `await app.prefs.setLimiter(true)` sets `get(app.runtime.store).limiter` before the
    await resolves (check synchronously after the call) and `prefs.saved` ends with
    `limiter: true`;
  - new: a `setVim` toggle leaves the runtime limiter value and sends no limiter change. Spy on
    `app.runtime.setLimiter` and assert no call;
  - two `createApp` wiring tests over happy-dom's `localStorage`. `GlobalRegistrator`
    (`app.test.ts:4`) installs a real one shared by every test in the file, so seed it with
    `localStorage.setItem('strudeland.limiter', '1')` and call `localStorage.clear()` in an
    `afterEach`. Do not move the `Object.defineProperty` stub from `runtime-levels.test.ts`
    unless happy-dom's descriptor is `configurable`:
    - `createApp(..., prefsApi)` with `'1'` stored: after `await app.ready` and `await flush()`,
      the runtime limiter is `true` and the key is gone;
    - `createApp(..., null)` (no prefs api) with `'1'` stored: after `await app.ready`, the key
      is still `'1'` and the runtime limiter is `false`.
- **`tests/runtime-levels.test.ts`:**
  - delete the `limiter-pref` test (`:128-148`), its import (`:5`), `realStorage`, the
    `afterEach` and `useStorage` (`:120-126`);
  - drop `afterEach` from the `bun:test` import if it becomes unused;
  - the runtime limiter tests (`:99-117`) stay.
- **Targeted while working:**
  - `bun test tests/prefs.test.ts tests/app.test.ts tests/runtime-levels.test.ts`;
  - `cargo test -p strudeland-project prefs::`.

## Validation

Run the gates in [000-prompt.md](000-prompt.md) with `N=003`. Also:

```sh
git grep -n 'localStorage' -- src > "$S/0128-003-storage.log"; echo "exit $?" >> "$S/0128-003-storage.log"
git grep -n 'limiter-pref\|loadLimiter\|saveLimiter' -- src tests AGENTS.md > "$S/0128-003-gone.log"; echo "exit $?" >> "$S/0128-003-gone.log"
git diff --stat HEAD -- crates/strudeland-record src-tauri src/components/ui \
  > "$S/0128-003-own-untouched.log"; echo "exit $?" >> "$S/0128-003-own-untouched.log"
```

- `storage.log` lists `src/lib/prefs/migrate.ts` and the comment at `src/lib/editor/setup.ts:14`
  ("we never read or write @strudel/codemirror's localStorage atom"), nothing else.
- `gone.log` lists nothing and ends `exit 1` (no match), which is the pass here.
- `own-untouched.log` lists no files. It diffs against the stage 002 commit.
- The ipc log is clean: `bindings.ts` is unchanged.

## STOP

- A `prefs.json` byte change beyond the trailing `"limiter"` key, or a reordered key.
- A four-field `prefs.json` would load any value other than its four values and `limiter: false`.
- The migration could remove the key before a confirmed save, could throw, or could override a
  toggle made before the load.
- A visible change beyond the accepted list. Examples: the meter's display source changes, a
  new notice text appears, or the control socket's `levels.limiter` changes meaning.
- `bindings.ts` would change.
- Anything else in the shared STOP list.

## Commit message

```
refactor(prefs): keep the limiter with the other preferences

The safety limiter is stored in prefs.json and toggled through the
prefs store; the app applies it to the runtime, which keeps the applied
value. The value saved in localStorage moves over once, and its key is
removed only after the save succeeds.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
