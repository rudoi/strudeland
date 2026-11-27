# 001: the status bar notices as one pure function

## Goal

`statusNotices` in the new `src/lib/status-notices.ts` returns the status bar's notices in
order, each with a stable id, text, tone, optional title and optional action kind.
`StatusBar.svelte` renders that list in the keyed `{#each}` it already has. Table tests pin
every source, both either/or pairs, the order, the titles and the ids. Nothing visible changes
(see [000-prompt.md](000-prompt.md), Rules).

## Prerequisites

- `git diff --name-only 9a5b6ff HEAD` lists only `plans/`.
- `bun install` has run (`node_modules/` present).

## Files

- New: `src/lib/status-notices.ts`, `tests/status-notices.test.ts`.
- Edited: `src/components/StatusBar.svelte` (script edits plus allowed markup replacements 1 and
  2 from the proposal), `AGENTS.md` (one new bullet).
- Scratch, not committed: `$S/0136-001-literals.txt`, `$S/0136-001-literals.sh` and
  `$S/0136-markup.ts`. The session that wrote this plan already wrote them; their full text is
  in the appendix. Recreate them from it when `$S` differs.
- Everything else is frozen.

## Steps

1. **Baseline, before any edit.**
   - Write (or check) the three scripts from the appendix, then `chmod +x "$S/0136-001-literals.sh"`.
   - `N=001`. Log the red runs on the unchanged tree. Both must exit 1:
     - `L base-literals "$S/0136-001-literals.sh"`. It prints `missing src/lib/status-notices.ts`
       and fails exactly 49 of its 68 rows (`grep -c FAIL` on the log gives 49). The 19 rows
       whose expected counts are the same at base and now pass: the help hints, the kept
       strings and `tone={n.tone}`.
     - `L base-markup bun "$S/0136-markup.ts"`. The markup check reports its first
       difference at markup line 5 (`{#each notices as n (n.id)}` against `{#if $rt.evalError}`)
       and `expected 166 lines, got 194`. The whole-file check reports file line 3
       (`import type { ControlInfo }` against `import { engineNotices }`) and
       `expected 236 lines, got 255`.

     These runs show that both scripts detect the change they guard. (Both were dry-run when
     the plan was written: against the unchanged tree they fail as above; against a draft with
     exactly this stage's edits both pass.)
   - `L base-check bun run check`. Record svelte-check's summary line; at `9a5b6ff` it is
     `0 ERRORS 0 WARNINGS`.

2. **Tests first:** the new `tests/status-notices.test.ts` (shape:
   `tests/engine-notices.test.ts`; `bun:test`, no DOM). Build every input as a plain object.
   A helper `input(over)` spreads overrides onto a quiet base:

   ```ts
   const quiet: StatusNoticesInput = {
     runtime: { evalError: null, outputError: null, tempoOverride: false, engineWarnings: [] },
     session: { error: null, metaError: null, notice: null },
     prefs: { error: null },
     control: { status: 'off', path: null, detail: null },
     recording: null,
     player: null,
     platform: 'MacIntel',
   };
   ```

   with `runtime`, `session`, `prefs` and `control` merged one level deep. A `rec(p)` helper
   builds a `RecordingUiState` like `tests/memo-ui.test.ts:66-78` but with `devices: []` and
   `input: null` (so there is no bluetooth notice). Write every expected value out as a literal,
   never with the function under test or with `engineNotices`/`recordingNotices`. Copy each
   text from `9a5b6ff`'s component, never from memory. Cases:

   - **nothing to say**: `quiet` gives `[]`. So do control `listening` and control `off`.
   - **one row per source** (a `for` over rows, as in `tests/timeline.test.ts`'s
     `sectionItemHelp` table). Each row gives the input override and the whole expected array:
     1. `evalError: 'boom'`: `[{ id: 'eval-error', tone: 'danger', text: 'error: boom' }]`
     2. `outputError: 'device gone'`: `[{ id: 'output-error', tone: 'danger', text: 'audio error: device gone' }]`
     3. `session.error: 'the project changed'`: `[{ id: 'session-error', tone: 'danger', text: 'the project changed' }]`
     4. `metaError: 'bpm must be a number'`: `[{ id: 'meta-error', tone: 'danger', text: 'meta: bpm must be a number' }]`
     5. `prefs.error: 'disk full'`: `[{ id: 'prefs-error', tone: 'danger', text: 'prefs: disk full' }]`
     6. `tempoOverride: true`: `[{ id: 'tempo-override', tone: 'warning', text: 'setcps in the code overrides meta.bpm' }]`
     7. `engineWarnings: [{ signature: 's=gm_piano', keys: [], firstCycle: 3 }]`: `[{ id: 'engine:s=gm_piano', tone: 'warning', text: 'rust engine skipped s=gm_piano' }]`
     8. the same with `keys: ['vowel', 'phaser']`: the same plus `title: 'keys: vowel, phaser'`
     9. `notice: { kind: 'claude-edit', path: 'songs/a.js', title: 'Alpha' }`: `[{ id: 'claude-edit', tone: 'warning', text: 'Claude edited Alpha' }]`
     10. control `{ status: 'in-use', path: '/tmp/strudeland.sock', detail: null }`: `[{ id: 'control-in-use', tone: 'warning', text: 'control socket in use', title: '/tmp/strudeland.sock' }]`
     11. control in use with `path: null`: the same with `title: ''`
     12. control `{ status: 'disabled', path: null, detail: 'bind failed' }`: `[{ id: 'control-disabled', tone: 'warning', text: 'control disabled: bind failed' }]`
     13. control disabled with `detail: null`: `text: 'control disabled: '` (Svelte renders a null as nothing)
     14. `recording: rec({ error: { kind: 'limitReached', message: 'x' } })`: `[{ id: 'limit-reached', tone: 'warning', text: 'recording stopped at the length limit; the memo was saved', action: { kind: 'dismiss' } }]`
     15. `player: { status: 'error', stem: 'take-01', kind: 'audio', message: 'could not play' }`: `[{ id: 'play-audio', tone: 'danger', text: 'could not play', action: { kind: 'dismiss-playback' } }]`

     `toEqual` treats an `undefined` property like a missing one, so after the table add
     `toStrictEqual` checks for rows 7 and 13: `'title' in n` is false for both. Add one for
     row 11 too: `'title' in n` is true and the title is `''`. Only the in-use notice keeps an
     empty title (today's `title=""`).
   - **the platform passes through**: `rec({ error: { kind: 'permissionDenied', message: 'Microphone access was denied' } })`.
     With `platform: 'MacIntel'` the notice is
     `{ id: 'permission', tone: 'danger', text: 'Microphone access was denied', action: { kind: 'link', label: 'open settings', href: MIC_SETTINGS_URL } }`
     (import the constant from `memo-ui.ts`; it is data, not the function under test). With
     `platform: 'Linux x86_64'` the text is
     `'Microphone access was denied · check the input device in sound settings'` and there is
     no `action` key.
   - **either/or pairs** (a table):
     - `evalError: 'boom'` with `outputError: 'device gone'`: only `eval-error`;
     - `evalError: ''` with `outputError: 'device gone'`: only `output-error` (an empty string
       shows nothing, as in `{#if}`);
     - `session.error: ''` gives `[]`;
     - control over all four statuses with `path: '/p'` and `detail: 'd'`: `off` and
       `listening` give `[]`, `in-use` gives only `control-in-use`, `disabled` gives only
       `control-disabled`.
   - **the order, everything at once**: `evalError`, `outputError`, `session.error`,
     `metaError`, `prefs.error`, `tempoOverride`, two engine warnings (`a` with keys `['k']`,
     `b` without), a Claude notice, control in use, a recording with a `limitReached` error and
     `warning: 'input level is low'`, and a player error of kind `tooLong`. The ids are exactly
     `['eval-error', 'session-error', 'meta-error', 'prefs-error', 'tempo-override', 'engine:a', 'engine:b', 'claude-edit', 'control-in-use', 'limit-reached', 'recording-warning', 'play-too-long']`.
     The same with `evalError: null` and control disabled gives
     `['output-error', 'session-error', …, 'claude-edit', 'control-disabled', …]` (the
     same list with `output-error` first and `control-disabled` in place of
     `control-in-use`). In both, the ids are unique (`new Set(ids).size === ids.length`), only
     `engine:a` and the control notice have a `title` key, and only `limit-reached` and
     `play-too-long` have an `action` key (`recording-warning` has none), and no notice has
     both a `title` and an `action` key.
   - **ids are stable per source**: `evalError: 'a'` and `evalError: 'b'` both give id
     `eval-error`; likewise `metaError`, control disabled with two details, and an engine
     warning with and without keys.

   Run `bun test tests/status-notices.test.ts`. It fails on the missing module, which is
   expected. Do not log it as a gate.

3. **`src/lib/status-notices.ts`.** Write the module as the proposal gives it, in the form the
   plan's draft took (it type-checked and was prettier-stable when the plan was written):

   ```ts
   /**
    * The status bar's notices, in the order it shows them, from plain store snapshots. Each has a
    * stable id per source (the list's key), so a changed message updates in place. The
    * component renders them and owns the action controls and their help. Pure, so the order, the
    * copy and the either/or pairs are tested without the UI.
    */
   import { engineNotices } from './audio/engine-notices';
   import type { MemoPlayerState } from './audio/memo-player';
   import type { ControlInfo } from './control/protocol';
   import type { PrefsState } from './prefs/store';
   import type { SessionState } from './project/session';
   import { recordingNotices, type NoticeAction, type RecordingNotice } from './recording/memo-ui';
   import type { RecordingUiState } from './recording/types';
   import type { RuntimeState } from './strudel/runtime';
   ```

   then, after one blank line, `StatusNoticesInput` and `StatusNotice`, exactly:

   ```ts
   export interface StatusNoticesInput {
     runtime: Pick<RuntimeState, 'evalError' | 'outputError' | 'tempoOverride' | 'engineWarnings'>;
     session: Pick<SessionState, 'error' | 'metaError' | 'notice'>;
     prefs: Pick<PrefsState, 'error'>;
     control: ControlInfo;
     /** Null until the recording controller is wired: no take notices. */
     recording: RecordingUiState | null;
     /** Null without memos: no playback notices. */
     player: MemoPlayerState | null;
     /** Absent: `navigator.platform` (see `memo-ui.ts`). */
     platform?: Navigator['platform'];
   }

   export interface StatusNotice {
     /** Stable per source, so a changed message updates in place. */
     id: string;
     tone: RecordingNotice['tone']; // 'warning' | 'danger'
     text: string;
     /** The tooltip; never set with `action`. */
     title?: string;
     /** The control after the text, rendered with its help by the component. */
     action?: NoticeAction;
   }
   ```

   `platform` is `Navigator['platform']`, not `string` and not
   `Parameters<typeof recordingNotices>[2]` (that would add a `recordingNotices` to the module
   and break that literal row). Then, after one blank line, the function:

   ```ts
   export function statusNotices(input: StatusNoticesInput): StatusNotice[] {
     const { runtime, session, prefs, control } = input;
     const out: StatusNotice[] = [];
     // An evaluate error hides the output error.
     if (runtime.evalError) {
       out.push({ id: 'eval-error', tone: 'danger', text: `error: ${runtime.evalError}` });
     } else if (runtime.outputError) {
       out.push({ id: 'output-error', tone: 'danger', text: `audio error: ${runtime.outputError}` });
     }
     if (session.error) out.push({ id: 'session-error', tone: 'danger', text: session.error });
     if (session.metaError) {
       out.push({ id: 'meta-error', tone: 'danger', text: `meta: ${session.metaError}` });
     }
     if (prefs.error) out.push({ id: 'prefs-error', tone: 'danger', text: `prefs: ${prefs.error}` });
     if (runtime.tempoOverride) {
       out.push({
         id: 'tempo-override',
         tone: 'warning',
         text: 'setcps in the code overrides meta.bpm',
       });
     }
     for (const n of engineNotices(runtime.engineWarnings)) {
       const notice: StatusNotice = { id: `engine:${n.key}`, tone: 'warning', text: n.text };
       if (n.detail !== undefined) notice.title = n.detail;
       out.push(notice);
     }
     if (session.notice) {
       out.push({ id: 'claude-edit', tone: 'warning', text: `Claude edited ${session.notice.title}` });
     }
     // The socket path is the tooltip, empty when unknown (the attribute is still set).
     if (control.status === 'in-use') {
       out.push({
         id: 'control-in-use',
         tone: 'warning',
         text: 'control socket in use',
         title: control.path ?? '',
       });
     } else if (control.status === 'disabled') {
       // Svelte renders a null as nothing; a template literal would write `null`.
       out.push({
         id: 'control-disabled',
         tone: 'warning',
         text: `control disabled: ${control.detail ?? ''}`,
       });
     }
     out.push(...recordingNotices(input.recording, input.player, input.platform));
     return out;
   }
   ```

   The literal script counts the module's text exactly, so:
   - write the `text:`, `id:`, `tone:` and `title` expressions exactly as above;
   - `StatusNotice.tone` is `RecordingNotice['tone']`, never the literal union (`tone: 'warning'`
     would add one to a counted row);
   - comments in the module mention none of the counted literals (`error: `, `meta: `,
     `dismiss`, `{#each`, `recordingNotices`, `engineNotices`, `title=`, `tone: '…'`). The doc
     comment above says "the list's key" and the `platform` doc says "see `memo-ui.ts`" for
     this reason.

   Run `bun test tests/status-notices.test.ts`. It must pass.

4. **`StatusBar.svelte`: script edits and allowed replacements 1 and 2.** At `9a5b6ff` line
   numbers:
   - delete `:3` (`import { engineNotices } …`) and `:11` (`import { recordingNotices } …`);
   - after `:12` (`import type { RecordingUiState } …`) add
     `  import { statusNotices } from '../lib/status-notices';`;
   - replace `:61` (`const notices = $derived(recordingNotices($recStore, $playerStore));`)
     with:

     ```ts
       // The notices in order, from every source; the action controls and their help stay here.
       const notices = $derived(
         statusNotices({
           runtime: $rt,
           session: $ss,
           prefs: $ps,
           control: $cs,
           recording: $recStore,
           player: $playerStore,
         }),
       );
     ```

     (`platform` is left out, so `recordingNotices` reads `navigator.platform` as today);
   - delete `:66-93`, the ten notice blocks from `{#if $rt.evalError}` through the `{/if}`
     after `control disabled: {$cs.detail}`;
   - `:124` becomes `        <Notice tone={n.tone} title={n.title}>{n.text}</Notice>`.

   Nothing else changes: not the action branch (`:95-122`), not the "audio suspended" button
   (`:127-135`), not the status line, not `<style>`. The `.notices:not(:has(> *))` rule still
   hides the empty row. The markup script rebuilds the whole expected file and byte-compares
   it. Its expected file is prettier-stable (checked when the plan was written).

5. **`AGENTS.md`.** After the `src/lib/shell.ts` bullet (ends `:128`,
   `` callers for them; tests use `tests/helpers/fake-shell-ports.ts`. ``), add:

   ```markdown
   - `src/lib/status-notices.ts` (`statusNotices`, pure) the status bar's notices: their order,
     copy, tones, tooltips, action kinds and stable ids, composed from `engineNotices` and
     `recordingNotices`. `StatusBar.svelte` renders the list and keeps the action controls,
     their help and the separate audio-suspended button.
   ```

   Change no other line.

6. `bun run format`, `bun test tests/status-notices.test.ts`, then commit (below), then the full
   gate from [000-prompt.md](000-prompt.md) with `N=001`, and the Validation steps here.

## Tests

The new `tests/status-notices.test.ts` (step 2): the quiet input; the 15-row source table with
the `toStrictEqual` title checks; the platform pass-through; the either/or table (1/1b, an
empty string, control over all four statuses); the everything-at-once order in both variants
with unique ids, the title and action key sets; stable ids. `tests/engine-notices.test.ts` and
`tests/memo-ui.test.ts` stay byte-identical.

## Validation

- The full gate steps 1 to 4 pass (000-prompt.md).
- Step 5, the proofs:
  - `L literals "$S/0136-001-literals.sh"`: exit 0, no `FAIL` row;
  - `L markup bun "$S/0136-markup.ts"`: exit 0, printing
    `markup equal: 9a5b6ff with the 2 allowed replacements` and
    `file equal: 9a5b6ff with the planned script edits and the 2 allowed replacements`.
- Step 6 lists exactly `AGENTS.md`, `src/components/StatusBar.svelte`,
  `src/lib/status-notices.ts`, `tests/status-notices.test.ts`.
- Step 7 shows one hunk: the new bullet after the `shell.ts` bullet.
- Neither `src/lib/status-notices.ts` nor `StatusBar.svelte` is in
  `crates/strudeland-host/js/host.inputs.json` or `PLANNER_FILES`, so there is no `host:build`
  or `engine:refs`.

## Commit message

```
refactor(status): derive the status bar notices in one pure function

statusNotices returns the status bar's notices in order, each with a
stable id, text, tone, tooltip and action kind, composed from the engine
and recording notices. StatusBar.svelte renders the one keyed list and
keeps the action controls, their help and the audio-suspended button.
Order, copy, tones, either/or pairs and tooltips are unchanged. Table
tests cover every source, both precedence pairs and the full order.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: the scratch scripts

Each block is the file of that name in `$S`. Tabs separate the columns in
`0136-001-literals.txt` (baseC, baseT, nowC, nowT, literal; `*` matches any count). C is
`StatusBar.svelte`, T is `status-notices.ts`.

### `0136-001-literals.txt`

```text
2	0	0	2	error: 
1	0	0	1	audio error: 
1	0	0	1	meta: 
1	0	0	1	setcps in the code overrides meta.bpm
1	0	0	1	Claude edited 
1	0	0	1	control socket in use
1	0	0	1	control disabled: 
1	0	0	0	prefs: {$ps.error}
0	0	0	1	text: `error: ${runtime.evalError}`
0	0	0	1	text: `audio error: ${runtime.outputError}`
0	0	0	1	text: session.error
0	0	0	1	text: `meta: ${session.metaError}`
0	0	0	1	text: `prefs: ${prefs.error}`
0	0	0	1	text: 'setcps in the code overrides meta.bpm'
0	0	0	1	text: n.text
0	0	0	1	text: `Claude edited ${session.notice.title}`
0	0	0	1	text: 'control socket in use'
0	0	0	1	text: `control disabled: ${control.detail ?? ''}`
5	0	0	0	tone="danger"
5	0	0	0	tone="warning"
0	0	0	5	tone: 'danger'
0	0	0	5	tone: 'warning'
2	0	2	0	tone={n.tone}
1	0	0	0	title={n.detail}
1	0	0	0	title={$cs.path ?? ''}
0	0	1	0	title={n.title}
2	0	1	0	title=
0	0	0	1	notice.title = n.detail
0	0	0	1	title: control.path ?? ''
0	0	0	1	id: 'eval-error'
0	0	0	1	id: 'output-error'
0	0	0	1	id: 'session-error'
0	0	0	1	id: 'meta-error'
0	0	0	1	id: 'prefs-error'
0	0	0	1	id: 'tempo-override'
0	0	0	1	id: `engine:${n.key}`
0	0	0	1	id: 'claude-edit'
0	0	0	1	id: 'control-in-use'
0	0	0	1	id: 'control-disabled'
1	0	1	0	audio suspended: click to enable
1	0	1	0	Open the microphone permissions for this computer.
1	0	1	0	Dismiss this memo playback message.
1	0	1	0	Dismiss this recording message.
1	0	1	0	Resume the audio output so you can hear playback.
1	0	1	0	Keep your existing layout when saving songs.
1	0	1	0	Format chains, song sections and mini-notation when saving songs.
1	0	1	0	Turn off Vim keybindings in the editor.
1	0	1	0	Use Vim keybindings in the editor.
1	0	1	0	Change typed-note previews and view credits and licenses.
1	0	1	0	Format on Save
1	0	1	0	aria-label="Settings"
1	0	1	0	>dismiss</Button
3	0	3	0	dismiss
4	0	4	0	help=
3	0	3	0	use:helpTarget
1	0	1	0	{@const action = n.action}
1	0	1	0	{#each notices as n (n.id)}
12	0	2	0	<Notice
2	0	1	0	{#each
11	0	4	0	{#if
5	0	3	0	{:else
2	0	0	2	engineNotices
2	0	0	2	recordingNotices
0	0	2	*	statusNotices
9	0	3	0	$rt.
7	0	1	0	$ss.
4	0	0	0	$cs.
9	0	7	0	$ps.
```

### `0136-001-literals.sh`

```sh
#!/bin/sh
# Literal counts in StatusBar.svelte (C) and status-notices.ts (T), at 9a5b6ff and in the tree.
# Each row of 0136-001-literals.txt is: baseC baseT nowC nowT literal (tab-separated; `*` = any).
# Every count must equal its row. Exit 1 on any difference, 2 on a setup error.
# Optional arguments name other C and T files (to test the script itself).
S=$(cd "$(dirname "$0")" && pwd)
REPO=/Volumes/rudenet/code/strudeland-worktrees/poptart
cd "$REPO" || exit 2
C=${1:-src/components/StatusBar.svelte}; T=${2:-src/lib/status-notices.ts}
git show 9a5b6ff:src/components/StatusBar.svelte > "$S/0136-base-C" || exit 2
: > "$S/0136-base-T" # the module is new
[ -f "$T" ] || { echo "missing $T"; T=/dev/null; }
n() { grep -oF -- "$1" "$2" | wc -l | tr -d ' '; }
fail=0
printf 'baseC\tbaseT\tnowC\tnowT\tliteral\n'
while IFS="$(printf '\t')" read -r ebc ebt enc ent lit; do
  bc=$(n "$lit" "$S/0136-base-C"); bt=$(n "$lit" "$S/0136-base-T"); nc=$(n "$lit" "$C"); nt=$(n "$lit" "$T")
  ok=1
  for pair in "$ebc:$bc" "$ebt:$bt" "$enc:$nc" "$ent:$nt"; do
    want=${pair%%:*}; got=${pair#*:}
    [ "$want" = '*' ] || [ "$want" = "$got" ] || ok=0
  done
  [ $ok = 1 ] || { fail=1; printf 'FAIL '; }
  printf '%s\t%s\t%s\t%s\t%s\n' "$bc" "$bt" "$nc" "$nt" "$lit"
done < "$S/0136-001-literals.txt"
exit $fail
```

### `0136-markup.ts` (also used by 003 with `--drain`)

```ts
// StatusBar.svelte must equal 9a5b6ff's with exactly the planned edits: the script edits of 001
// step 4 and the allowed markup replacements (001: the deleted `{#if}` notices and the one
// `title={n.title}` line; with `--drain`, also 003's `drain` line). Two checks: the markup alone
// (everything from `</script>` on), then the whole file, which also catches leftover imports and
// any other script change. Exit 0 both equal, 1 different, 2 setup error.
// Usage: bun 0136-markup.ts [--drain] [file]
import { $ } from 'bun';

const REPO = '/Volumes/rudenet/code/strudeland-worktrees/poptart';
const FILE = 'src/components/StatusBar.svelte';
const args = process.argv.slice(2);
const drain = args[0] === '--drain';
const other = drain ? args[1] : args[0];
const markup = (text: string) => {
  const at = text.indexOf('</script>');
  if (at < 0 || text.indexOf('</script>', at + 1) >= 0) throw new Error('want one </script>');
  return text.slice(at);
};
/** Replace the one occurrence of `from` in `text` with `to`. */
const once = (text: string, from: string, to: string) => {
  const parts = text.split(from);
  if (parts.length !== 2) throw new Error(`base block found ${parts.length - 1} times:\n${from}`);
  return parts.join(to);
};

// The script edits, as base line ranges (one-based, inclusive), each checked by its first and
// last base line. An empty `put` deletes the range.
const DERIVED = `  // The notices in order, from every source; the action controls and their help stay here.
  const notices = $derived(
    statusNotices({
      runtime: $rt,
      session: $ss,
      prefs: $ps,
      control: $cs,
      recording: $recStore,
      player: $playerStore,
    }),
  );`;
const SCRIPT: [number, number, string, string, string[]][] = [
  [
    3,
    3,
    "  import { engineNotices } from '../lib/audio/engine-notices';",
    "  import { engineNotices } from '../lib/audio/engine-notices';",
    [],
  ],
  [
    11,
    11,
    "  import { recordingNotices } from '../lib/recording/memo-ui';",
    "  import { recordingNotices } from '../lib/recording/memo-ui';",
    [],
  ],
  [
    12,
    12,
    "  import type { RecordingUiState } from '../lib/recording/types';",
    "  import type { RecordingUiState } from '../lib/recording/types';",
    [
      "  import type { RecordingUiState } from '../lib/recording/types';",
      "  import { statusNotices } from '../lib/status-notices';",
    ],
  ],
  [
    61,
    61,
    '  const notices = $derived(recordingNotices($recStore, $playerStore));',
    '  const notices = $derived(recordingNotices($recStore, $playerStore));',
    DERIVED.split('\n'),
  ],
];

// The markup replacements: the ten `{#if}`/`{#each}` notices before the recording list go, and
// the plain recording notice gains the title.
const NOTICES = `    {#if $rt.evalError}
      <Notice tone="danger">error: {$rt.evalError}</Notice>
    {:else if $rt.outputError}
      <Notice tone="danger">audio error: {$rt.outputError}</Notice>
    {/if}
    {#if $ss.error}
      <Notice tone="danger">{$ss.error}</Notice>
    {/if}
    {#if $ss.metaError}
      <Notice tone="danger">meta: {$ss.metaError}</Notice>
    {/if}
    {#if $ps.error}
      <Notice tone="danger">prefs: {$ps.error}</Notice>
    {/if}
    {#if $rt.tempoOverride}
      <Notice tone="warning">setcps in the code overrides meta.bpm</Notice>
    {/if}
    {#each engineNotices($rt.engineWarnings) as n (n.key)}
      <Notice tone="warning" title={n.detail}>{n.text}</Notice>
    {/each}
    {#if $ss.notice}
      <Notice tone="warning">Claude edited {$ss.notice.title}</Notice>
    {/if}
    {#if $cs.status === 'in-use'}
      <Notice tone="warning" title={$cs.path ?? ''}>control socket in use</Notice>
    {:else if $cs.status === 'disabled'}
      <Notice tone="warning">control disabled: {$cs.detail}</Notice>
    {/if}
`;
const REPLACEMENTS: [string, string][] = [
  [NOTICES, ''],
  [
    '        <Notice tone={n.tone}>{n.text}</Notice>\n',
    '        <Notice tone={n.tone} title={n.title}>{n.text}</Notice>\n',
  ],
];
if (drain) {
  REPLACEMENTS.push([
    '        drain={runtime.drainLevels}\n',
    '        drain={() => runtime.drainLevels()}\n',
  ]);
}
const allowed = `${REPLACEMENTS.length} allowed replacement${REPLACEMENTS.length === 1 ? '' : 's'}`;

/** Print the first differing line of two texts and both line counts; false when they differ. */
const compare = (what: string, expected: string, actual: string) => {
  if (actual === expected) return true;
  const a = expected.split('\n');
  const b = actual.split('\n');
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] !== b[i]) {
      console.log(`first difference at ${what} line ${i + 1}:\n- ${a[i]}\n+ ${b[i]}`);
      break;
    }
  }
  console.log(`expected ${a.length} lines, got ${b.length}`);
  return false;
};

try {
  const base = await $`git -C ${REPO} show 9a5b6ff:${FILE}`.text();
  const lines = base.split('\n');
  // Bottom up, so the earlier ranges keep their base line numbers.
  for (const [from, to, first, last, put] of [...SCRIPT].reverse()) {
    if (lines[from - 1] !== first || lines[to - 1] !== last) {
      throw new Error(`base lines ${from}-${to} are not:\n${first}\n…\n${last}`);
    }
    lines.splice(from - 1, to - from + 1, ...put);
  }
  let expected = lines.join('\n');
  const head = expected.slice(0, expected.indexOf('</script>'));
  let tail = markup(expected);
  for (const [from, to] of REPLACEMENTS) tail = once(tail, from, to);
  expected = head + tail;

  // An optional argument names another file to check, for testing the script itself.
  const actual = await Bun.file(other ?? `${REPO}/${FILE}`).text();
  let ok = compare('markup', tail, markup(actual));
  if (ok) console.log(`markup equal: 9a5b6ff with the ${allowed}`);
  if (compare('file', expected, actual)) {
    console.log(`file equal: 9a5b6ff with the planned script edits and the ${allowed}`);
  } else ok = false;
  process.exit(ok ? 0 : 1);
} catch (e) {
  console.error(String(e));
  process.exit(2);
}
```
