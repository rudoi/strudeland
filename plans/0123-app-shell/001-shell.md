# 001: the app shell

## Goal

`src/lib/shell.ts` owns the modal slot, the Quit decision, the window-close predicate, the gate
table, the Export Audio sync and the listener registration. `App.svelte` renders from it and
keeps only the DOM work. Nothing visible changes. The proposal's "Technical design" is the
spec; this file is the order of work.

## Prerequisites

- On `llm/andrew/strudeland/0123-app-shell`, based on `f7861f5`.
- Read: the proposal, `reverify.md`, `src/App.svelte`, `src/lib/play-arbiter.ts` (style),
  `src/lib/export/controller.ts:150-339` (`canClose`, `requestQuit`, `answerQuit`,
  `finishQuit`), `src/lib/project/session.ts:1123-1133` and `:1305-1325`,
  `src/lib/project/actions.ts`, `tests/export-dialog.test.ts:35-42` and `:299-325`,
  `tests/control-handler.test.ts:40-66` (a real session in a test).

## Files

| File | Change |
|---|---|
| `src/lib/shell.ts` | new: `createShell`, `Shell`, `ShellState`, `Modal`, `FocusTarget`, `ShellAction`, `Gate`, `allowed`, `QuitPort`, `MenuPort`, `ShellSession` |
| `src/lib/shell-ports.ts` | new: `quitPort`, `menuPort` over the four existing adapter functions |
| `tests/helpers/fake-shell-ports.ts` | new: `fakeShellPorts()` |
| `tests/shell.test.ts` | new: the twelve cases |
| `tests/ipc-adapters.test.ts` | one test: the Tauri ports are the existing functions |
| `src/App.svelte` | moved onto the shell |
| `AGENTS.md` | layout line for the shell and its ports; the "Application Quit" line names the shell |
| `GLOSSARY.md` | "App shell" under "App boundary" |

## Steps

1. **Ports and the fake first.** Write the `QuitPort` and `MenuPort` types in `shell.ts`,
   `shell-ports.ts`, and `fake-shell-ports.ts` with its log, emitters, holds and `replyFails`
   (proposal, "Tests"). Add the `ipc-adapters.test.ts` case.
2. **`allowed` and its table test (case 8, the pure half).** Write the gate exactly as the
   proposal's table, with the comment that the row differences are deliberate and filed for
   review (#152 may change them; the comment names no task id, only that they are today's
   behaviour on purpose). Test every action against a base gate with each condition flipped
   alone.
3. **The store, the modal verbs and `close`.** `openSettings`, `openInstruments`, `menu`'s
   export branch, `close`, the instruments drop in the `session.store` subscription. Cases
   10, 11, 12 and the verb half of 8.
4. **`canCloseWindow` and `onQuit`.** Copy `onAppQuit` (`App.svelte:113-133`) line for line
   onto the ports, with the store updated alongside the local `quitting`. Cases 1-5 and 7.
5. **`start`, the menu sync and `destroy`.** Copy the `onMount` chain (`App.svelte:136-167`)
   without help and without `projectActions.destroy`. The sync sends on attach, then on change.
   Cases 6 and 9.
6. **Move `App.svelte`.** As the proposal's "App.svelte after the move": build the shell,
   render from `$shell`, keep the two close handlers' tick and focus code, call
   `shell.shortcut` from `onKeydown` after the `preventDefault` and `stopPropagation`, route
   Welcome's `onnew`, `onopen` and `onrecent` and StatusBar's `onsettings` and SongList's
   `oninstruments` through the shell. Delete every variable, effect and function the proposal
   lists as removed. `rg 'actionBlocked|projectsReady|nativeMenuReady|onAppQuit' src` is empty
   afterwards.
7. **Docs.**
   - `AGENTS.md`, next to the play arbiter's layout line: `src/lib/shell.ts` is the app shell
     (the open dialog and where focus returns, the Quit decision, the window-close predicate it
     hands `session.init`, the gate table for menu items, shortcuts and the welcome screen, the
     Export Audio item's availability). `src/lib/shell-ports.ts` wraps `app-quit.ts`,
     `onProjectMenu` and `setExportMenuAvailable`, which stay the only Tauri callers for them;
     tests use `tests/helpers/fake-shell-ports.ts`.
   - `AGENTS.md`'s "Application Quit on macOS" line: the AppKit callback defers to the webview
     through `src/lib/app-quit.ts` and the shell's `onQuit`.
   - `GLOSSARY.md`, under "App boundary", in the glossary's voice (what it is, no
     implementation):

     ```md
     **App shell**

     What the window does around the editor: which one dialog is open (settings, the
     instrument browser, export), whether a Quit is being decided, and whether a menu item,
     shortcut or welcome button acts now. An item that cannot act is ignored, not disabled;
     only Export Audio is greyed out, until a song is open.
     _Avoid_: app (everything `createApp` builds), workspace (the editor and its panels), window
     (the native window)
     ```

8. **Gates**, once (000-prompt). Then commit.

## Tests

The twelve cases in the proposal's "Tests", in `tests/shell.test.ts`, plus the
`ipc-adapters.test.ts` port test. Every case states its expected log or state, not just "no
throw". No existing test changes.

## Validation

- `bun test`, `bun run check`, `bun run lint` logs end `exit 0` (after `bun run format`).
- `git diff --stat f7861f5 -- src-tauri crates src/lib/app.ts src/lib/play-arbiter.ts
  src/components src/lib/project src/lib/export src/lib/app-quit.ts` is empty.
- `rg -n "app-quit|onProjectMenu|setExportMenuAvailable" src` shows only the defining modules
  and `src/lib/shell-ports.ts`.
- `rg -n "@tauri-apps" src/lib/shell.ts` is empty, and `shell.ts`'s imports from
  `project/api.ts` are `import type`.

## Manual smoke (the user runs it, `bun run tauri dev`; list it in the PR's test plan)

1. **Quit during an export.** Start a long export, press Cmd-Q: the dialog asks wait or cancel.
   Cancel: the app stays and the export continues. Cmd-Q again, wait: the result shows; continue
   with an unsaved song: the unsaved prompt comes next; Cancel keeps the app open; Save quits.
2. **Cmd-N with export open.** Nothing happens (no new-song form); the export dialog keeps focus.
   Also try File > Open and File > Settings: both ignored.
3. **Cmd-, with instruments open.** Settings does not open over the instrument browser. Close
   the browser without applying: focus returns to its button. Apply an instrument: focus goes to
   the editor.
4. **Window close with unsaved changes.** Close the window: the unsaved prompt. Cancel keeps it.
   During a running export, closing the window does nothing and shows no prompt.
5. **Export Audio menu item.** Disabled on the welcome screen and in an empty project, enabled
   once a song is open.
6. **Settings focus.** Open settings from the status bar, close it: focus returns to the
   settings button.

## Commit message

```
refactor(app): move quit, dialogs and menu gating into a shell module

App.svelte held the Quit ordering, the window-close predicate, three
dialog variables, the Export Audio menu sync and a gate re-derived at
each caller, none of it tested. src/lib/shell.ts now owns them behind
a quit port and a menu port, with one gate table that keeps every
caller's current preconditions. App.svelte renders from the shell and
keeps only the DOM work. No visible change.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
