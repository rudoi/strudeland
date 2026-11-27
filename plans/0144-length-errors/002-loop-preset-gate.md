# 002: the loop preset gate

## Goal

In the export dialog, a loop preset (`intro-loop`, `tagged-loop`, `folded-loop`) cannot be
exported when the song's header marks no loop. The export button is disabled, and its
contextual help reads `Mark a loop in the song to export a loop preset.` These stay as they
are:

- the radios, which remain selectable, so the warning can explain why;
- the range label;
- the warning text;
- the preview request.

A pure `loopUnavailable(song, options)` in the controller is the one statement of the rule. The
component and the controller's `start()` both read it.

## Prerequisites

- [000-prompt.md](000-prompt.md), and stage 001 committed (the baselines and the gate's
  scripts exist).
- No code dependency on 001.

## Files

- Edited:
  - `src/lib/export/controller.ts`;
  - `src/components/ExportAudio.svelte`;
  - `tests/export-dialog.test.ts`;
  - `AGENTS.md:160-165`, the desktop export bullet.
- Frozen (gate step 7):
  - against `0113154`, 001's frozen list minus `src/lib/export` and `src/components`;
  - plus, against `HEAD` (the 001 commit, before this stage commits): `git diff --exit-code
    HEAD -- crates src-tauri src/lib/render src/host src/lib/strudel tests/render
    crates/strudeland-host/js` (log it as `frozen-001`).

## Steps

1. **`controller.ts`.** After `showMp3Warning` (`:116-117`), add:

   ```ts
   /** A loop preset with no loop to export: the song's header, as typed, marks neither a
    *  `meta.loop` range nor a section with `loop: true`. The planner reads the evaluated song,
    *  so a loop that only `song()` registers is not seen here (the dialog words its ranges
    *  from the header too). */
   export const loopUnavailable = (
     song: Pick<ExportSong, 'structure'>,
     { mode }: Pick<Options, 'mode'>,
   ): boolean => loopMode(mode) && (song.structure === null || authoredLoop(song.structure) === null);
   ```

   Import `authoredLoop` with the existing `SongStructure` type import from
   `../strudel/structure`. In `start()` (`:261-262`), add `|| loopUnavailable(song,
   state.options)` to the early-return condition, after `state.result`. Nothing else in the
   controller changes. `requestFor`, the preview and `refresh` stay as they are, so the dialog
   still shows the destination for a loop preset.
2. **`ExportAudio.svelte`.**
   - Import `loopUnavailable` with the other controller imports (`:3-10`).
   - Add `const unavailable = $derived(loopUnavailable(source, options));` after `options`
     (`:30`).
   - Change `canExport` (`:63`) to `$derived(!locked && !unavailable && $store.preview.kind ===
     'ready' && !$store.result)`.
   - In `exportHelp` (`:69-79`), add a branch right after `locked`: `: unavailable ? 'Mark a
     loop in the song to export a loop preset.'`. The missing loop is the real blocker
     whatever the preview's state (`docs/writing-contextual-help.md`: for an unavailable
     action, give the actual blocker, then the next step).
   - Change the warning's condition (`:266`) from `loopMode(options.mode) && !markedLoop` to
     `unavailable`. Its text (`:267-269`) stays. Section names are identifiers
     (`structure.ts:186`), so the two conditions agree on every song.
   - `markedLoop` (`:51-57`) stays as the range label's text. `authoredLoop` stays imported for
     it.

   Check the hint against the guide:
   - It names the blocker and the next step.
   - It is one sentence in sentence case, which help sentences allow.
   - Read with the button name it says "export audio: Mark a loop in the song to export a loop
     preset."
   - It appears only while a loop preset is chosen on a song without a loop. Choosing another
     mode, or marking a loop in the header (the session republishes the structure), lifts it.
     Note that the dialog captures `source` once at open (`:28`), so a loop marked while the
     dialog is open lifts the gate only on reopening. That matches the existing warning and the
     range label.
3. **Tests** in `tests/export-dialog.test.ts`. Import `loopUnavailable`.
   - **Table test `loopUnavailable`.** Each mode in `MODES`, crossed with these structures:
     - `structure` (the file's, `body` has `loop: true`);
     - `buildStructure({ sections: [{ name: 'a', bars: 2 }, { name: 'b', bars: 2 }] })`, no
       loop;
     - a `meta.loop` range, `buildStructure({ sections: [a 2, b 2, c 2], loop: { from: 'a',
       through: 'b' } })`;
     - `buildStructure({})`, no sections;
     - `null`.

     Expect true exactly for the three loop modes on the no-loop, no-sections and `null`
     structures, and false everywhere else (standard, stems, one-shot and every looped
     structure).
   - **`start()` refuses while the gate holds.** Use `setup({ ...editor, structure:
     buildStructure({}) })`. Select `tagged-loop` and await the preview, so it is `ready`.
     Then `await dialog.start()`, and expect `api.start` not called and `activity` still
     `idle`. Select `one-shot` with `range: 'length'` and `bars: '2'`, wait for the preview,
     start, and expect `api.start` called once.
   - **A loop preset on a looped song still starts.** With the file's `editor`, select
     `folded-loop`, start, and expect `api.start` called with `preset: 'folded-loop'`.
   - The existing `accepts manual clip or loop requests when the static structure is
     unavailable` (`:163`) stays unedited. It asserts the preview requests, which do not
     change.

   The component's wiring, `canExport`, `exportHelp` and the warning, has no automated test.
   The repo has no Svelte component harness: every `tests/*.test.ts` drives controllers and
   pure functions. The wiring reads only `loopUnavailable`, which is tested, and
   `bun run check` (svelte-check) proves that it typechecks. Record this in the PR's test plan.
   Do not add a source-text assertion on the `.svelte` file.

   Targeted: `bun test tests/export-dialog.test.ts tests/contextual-help.test.ts`.
4. **Docs.** At the end of `AGENTS.md`'s desktop export bullet (`:160-165`), add: "The dialog
   words the planner's length problems with `export::dialog_text`, and disables a loop preset
   export while `loopUnavailable` holds: the header marks no loop." The first clause names
   001's behaviour; 001's step 12 documents only the module.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=002`: steps 1 (with
`host-unchanged`), 3 (exit 0), 6, 7 (with `frozen-001`), 8, 9, 10, 11, 12, 13, 14 and 15. Steps
2, 4, 5 and 16 do not apply, because no planner or bundled file changes. The length and plan
A/B re-run as controls, and both must still be equal.

Also run:

```sh
git diff --name-only HEAD > "$S/0144-002-files.log"
```

It must list exactly the four files under Files.

Hand check. This is a reading of the code; do not start the app. Walk `exportHelp`'s branches
for a loop preset on a song without a loop, in each preview state (`loading`, `error`, `ready`):

- each one gives the gate's sentence;
- `locked` still wins;
- the warning and the help never contradict each other.

## STOP

- A needed change to `requestFor`, the preview flow, the radios' `disabled` state or the
  warning text. Those are outside the decided scope.
- Any file outside the four, or any frozen diff.
- Anything in the shared STOP list.

## Commit message

```
feat(export): disable loop preset export when the song marks no loop

A loop preset on a song whose header marks no meta.loop range and no
section with loop: true can no longer be started: the export button is
disabled and its help says to mark a loop. The radios and the existing
warning are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
