# 001: derive the timeline view in one pure function

## Goal

`timelineView` in `src/lib/project/timeline.ts` derives everything `Timeline.svelte` shows that
depends on what plays: the playing flags, `playable`, the ending, its enabled state and help,
the stop-looping help, the ruler and block playheads, the armed name and the audition names.
The component reads it through one `$derived`. Plain-object tests pin the cases reverify §2
found untested. Nothing visible changes (see [000-prompt.md](000-prompt.md), Rules).

## Prerequisites

- The tree is code-identical to `b0229a8`: `git diff --name-only b0229a8 HEAD` lists only
  `plans/`.
- `bun install` has run (`node_modules/` present).

## Files

- Edited: `src/lib/project/timeline.ts`, `src/components/Timeline.svelte`,
  `tests/timeline.test.ts`, `AGENTS.md` (`:137-138`).
- New, not committed: `$S/0133-literals.txt`, `$S/0133-literals.sh`, `$S/0133-markup.ts`.
  The session that wrote this plan already wrote them. Their full text is in the appendix.
  Recreate them from it when `$S` differs.
- Everything else is frozen.

## Steps

1. **Baseline, before any edit.**
   - Write (or check) the three scripts from the appendix. Then `chmod +x "$S/0133-literals.sh"`.
   - Log the red runs on the unchanged tree. Both must exit 1:
     - `L base-literals "$S/0133-literals.sh"` fails exactly the 8 `M` rows. Each reads
       `1 0 1 0`: the strings have not moved yet.
     - `L base-markup bun "$S/0133-markup.ts"` reports its first difference at markup line 16
       (`help={endingHelp}` against `help={endingEnabled`). The log also shows
       `expected 338 lines, got 350`.

     These runs show that both scripts detect the change they guard.
   - Log `L base-check bun run check` and record its svelte-check error and warning counts.
     Gate step 3 compares against them.

2. **Tests first** (`tests/timeline.test.ts`, appended after the last `describe`; the import
   list gains `timelineView` and `type TimelineViewInput`). Build every input as a plain
   object. A helper `input(over)` spreads overrides onto a base:
   - session `{ active: 'songs/a.js', structure: song, metaError: null }`;
   - runtime `{ started: true, structure: song }`;
   - playback `{ playing: { path: 'songs/a.js', title: 'a' }, transport: { view: { kind: 'song' }, loop: null, armed: null } }`;
   - position `{ cycle: 3, located: { section: 1, localCycle: 1, songBar: 3 } }`.

   `song` is the existing fixture (`:43-50`: intro 2, body 4 with the loop, tail 2). Expected
   values are written out, never computed with the function under test. Each help expectation
   is the literal string, copied from `b0229a8`'s component. The cases:
   - **the active song plays**: the base input gives `playingActive` true,
     `playingOther` false, `head` `{ section: 1, fraction: 0.25, localCycle: 1 }`, `pct` null
     (the song has sections), `ending` true, `endingEnabled` true, and the endingHelp
     `Finish the loop pass, then play the ending once.`
   - **playing another song**: `playing.path` `'songs/b.js'`. Then `playingOther` is true,
     `playingActive` false, `head` null, `pct` null, `endingEnabled` false, and the help is
     `The ending belongs to the song currently playing.` With a sectionless header and
     evaluated structure (`buildStructure({ bpm: 100 })` for both), `pct` is null while
     another song plays, and `playheadPct`'s value (`(3 % 16) / 16 * 100 = 18.75`) for the
     active one.
   - **evaluated structure differs from the header**: runtime structure
     `buildStructure({ sections: [intro 2, verse 2, body 4 loop, tail 2] })` and located
     `{ section: 2, localCycle: 1, songBar: 5 }`. Then `head` is
     `{ section: 1, fraction: 0.25, localCycle: 1 }`: header index 1, by the header's 4 bars.
     `playable` is the runtime structure, by identity (`toBe`). Stopped (`started` false,
     `playing` null), `playable` is the header.
   - **armed while stopped**: started false, `playing` null, `transport.armed` `'body'`.
     Then `armedName` is `'body'` and the ending help is
     `Start song playback to reach its defined loop.` With `metaError: 'meta.bpm …'` (and
     the structure still set, as the session holds the defaults), `armedName` is null. With
     `structure: null` and no meta error, `armedName` is null too: the gate is on the
     structure.
   - **duplicate names, first match**: the header is `{ ...song, sections: [intro, body, body', tail] }`,
     written by hand with the `Section` fields (`name`, `start`, `bars`, `loop`). A valid
     header cannot repeat a name (`sectionsProblem`, `structure.ts:170-198`), so `buildStructure`
     cannot make it. The evaluated section is `body`, so `head.section` is 1, never 2.
   - **missing section**: runtime structure with sections `[intro 2, bridge 4]`, located
     `{ section: 1, … }` (`bridge`). Then `head` is null while `playingActive` is true.
   - **audition names**: `transport.loop` `{ section: 'body', names: ['body', 'tail'] }`
     gives `['body', 'tail']`. `null` gives `[]`.
   - **ending help table** (5 rows, a `for` over rows as in `sectionItemHelp`'s test):
     1. enabled (base input): `Finish the loop pass, then play the ending once.`
     2. another song plays: `The ending belongs to the song currently playing.`
     3. stopped, nothing playing: `Start song playback to reach its defined loop.`
     4. started and active, `view: { kind: 'release' }`: `The ending is already queued or playing.`
     5. started and active, located in `tail` (`{ section: 2, localCycle: 0, songBar: 6 }`,
        view `song`), so not on a loop pass: `Available while the song plays through its defined loop.`

     Plus one precedence row: another song plays while `started` is false. The help is still
     row 2's.
   - **stop-looping help table** (3 rows):
     1. view `song`: `There’s no temporary section loop to clear.`
     2. view `{ kind: 'loop', section: 1 }`, started: `Return to the song’s normal playback at the next available bar line.`
     3. the same view, stopped: `Clear the temporary section loop for the next play.`

   Run `bun test tests/timeline.test.ts`. It fails on the missing export, which is expected.
   Do not log it as a gate.

3. **`timeline.ts`.**
   - Add `TimelineViewInput`, `TimelineView` and `timelineView` after `blockPlayhead`
     (`:137`), as the proposal's Technical design gives them. The doc comments carry the
     component's two comments over: the blocks-or-ruler playhead (`Timeline.svelte:90-92`)
     and the meta-error gate on the armed name (`:113-114`).
   - Imports:
     - `import type { SessionState } from './session'`;
     - `import type { PlaybackState } from './song-playback'`;
     - `import type { RuntimeState } from '../strudel/runtime'`;
     - `canPlayEnding` added to the existing `../strudel/transport` import.

     The first three are type-only, so no module is loaded at runtime (`session.ts` and
     `runtime.ts` pull in CodeMirror and Strudel).
   - The body moves today's expressions with their operand order. Name the view kind once
     (`const kind = playback.transport.view.kind`) and write the two help ternaries over it
     with the proposal's strings, typed or pasted from `b0229a8`, never retyped from memory.
     Curly apostrophes are `’` (U+2019).
   - The module doc (`:1`) becomes
     `/** Pure helpers behind \`Timeline.svelte\`: layout, what plays and where, section-item help and input validation. */`.
   - Run `bun test tests/timeline.test.ts`. It must pass.

4. **`Timeline.svelte`, script only, plus the two allowed attributes.**
   - Imports from `../lib/project/timeline`: drop `blockPlayhead`, `hasEnding` and
     `playheadPct`, and add `timelineView`. From `../lib/strudel/transport`, drop
     `canPlayEnding` and keep `type Located`.
   - Delete `:68-72` (`playingActive` through `ending`), `:90-107` (the playhead comment,
     `pct`, `head`, `auditionNames`, `endingEnabled`), and `:113-115` (the `armedName` comment
     and line).
   - After the position block (`:74-88`, unchanged), add:

     ```ts
     // What plays and where: the playheads, the ending and the armed start (see `timelineView`).
     const {
       playingOther,
       ending,
       endingEnabled,
       endingHelp,
       stopLoopingHelp,
       pct,
       head,
       armedName,
       auditionNames,
     } = $derived(timelineView({ session: $ss, runtime: $rt, playback: $pb, position }));
     ```

     `playingActive` and `playable` are not destructured: the markup does not read them. If
     svelte-check refuses the destructured form, fall back to `const view = $derived(...)`
     plus one `const x = $derived(view.x)` per name above. Markup stays the same either way.
     If both are refused, STOP.
   - The two attributes, exactly: `:183-191` becomes `help={endingHelp}` and `:277-281`
     becomes `help={stopLoopingHelp}`, at their current indentation. Nothing else after
     `</script>` changes.
   - `structure`, `sections`, `savedLoop`, `itemHelp`, the menu state, `showRangeOptions` and
     the commit glue stay as they are.

5. **AGENTS.md** (`:137-138`): `` `timeline.ts` (pure helpers for `Timeline.svelte`) `` becomes
   `` `timeline.ts` (pure helpers for `Timeline.svelte`, and `timelineView`, what it shows of
   the playing song: playheads, the ending and its help, the armed start) ``. Rewrap the
   bullet to the file's 100 columns. Change no other line.

6. `bun run format`, `bun test tests/timeline.test.ts`, then commit (below), then the full gate
   from [000-prompt.md](000-prompt.md), steps 1 to 8, in order. If a gate fails, fix it, amend,
   and re-run the gate once.

## Tests

The new `describe('timelineView')` in `tests/timeline.test.ts` (step 2). It covers all three
requested cases, the duplicate first match, the missing section, the 5-row ending help table
with its precedence row, the 3-row stop-looping table, the meta-error and no-structure armed
gates, `pct` for a sectionless song, and the audition names. The existing tests in the file
(`blockPlayhead`, `hasEnding`, `playheadPct`, `editableStructure`, `sectionItemHelp`, …)
stay byte-identical.

## Validation

- Gate steps 1 to 8 pass (000-prompt.md).
- Step 5's table: every `M` row reads `1 0 0 1`, and every `=` row and the `’` total are
  unchanged.
- Step 6 prints `markup equal: b0229a8 with the 2 allowed replacements`.
- `git diff b0229a8 -- src/components/Timeline.svelte` touches only `<script>` and the two
  `help=` lines.

## Commit message

```
refactor(timeline): derive the timeline view in one pure function

timelineView computes the playing flags, the block and ruler playheads,
the ending's state and help, the stop-looping help and the armed and
audition names from plain store snapshots. Timeline.svelte reads it
through one $derived. Markup, copy and help texts are unchanged. Tests
cover another song playing, an evaluated structure that differs from
the header, arming while stopped, duplicate and missing section names,
and both help tables.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: the scratch scripts

Each block is the file of that name in `$S`. Tabs separate the columns in `0133-literals.txt`.

### `0133-literals.txt`

```text
M	Finish the loop pass, then play the ending once.
M	The ending belongs to the song currently playing.
M	Start song playback to reach its defined loop.
M	The ending is already queued or playing.
M	Available while the song plays through its defined loop.
M	There’s no temporary section loop to clear.
M	Return to the song’s normal playback at the next available bar line.
M	Clear the temporary section loop for the next play.
=	Return to the section’s playback options.
=	Re-evaluate the song to use this section range.
=	Repeat through the selected end, starting at the next available bar line.
=	Set this section range to repeat the next time the song plays.
=	Start playback and repeat through the selected end.
=	Re-evaluate the song to play this section.
=	Jump to this section at the next available bar line.
=	Set this section as the starting point for the next play.
=	Start song playback at the first bar of this section.
=	Re-evaluate the song to loop this section.
=	Repeat this section starting at the next available bar line.
=	Set this section to repeat the next time the song plays.
=	Start playback and repeat this section.
=	Choose the last section to include in the loop.
=	Set the tempo in quarter notes per minute. Applies on the next evaluation.
=	Set the time signature before the first meter change, for example 4/4. Applies on the next evaluation.
=	Set the song’s time signature, for example 4/4. Applies on the next evaluation.
=	Set how many times the defined loop plays in the song’s clip layout.
=	Set this section’s length in whole bars. Applies on the next evaluation.
=	Stop the take first.
=	Stop the memo first.
=	<span>playing: {$pb.evaluated?.title}</span>
=	<span>will start at {armedName}</span>
=	play ending
=	>back</MenuItem
=	through {end.name}
=	start here
=	loop this section
=	>loop through…</MenuItem
=	stop looping
=	label="quarter-note bpm"
=	? 'starting meter' : 'meter'}
=	label="loops"
=	label="{sec.name} bars"
=	applies on the next evaluate
=	fix the meta header to edit
=	'the header changed, try again'
=	'this header cannot be edited here; edit it in the code'
=	aria-label="time signature changes"
=	data-range-focus=""
=	aria-haspopup="menu"
+	’
=	help=
```

### `0133-literals.sh`

```sh
#!/bin/sh
# Literal counts in Timeline.svelte (C) and timeline.ts (T), at b0229a8 and in the working tree.
# "M" rows must go from C=1 T=0 to C=0 T=1; "=" rows must keep both counts; the C+T total of
# every row ("+" rows: only it) must be unchanged. Exit 1 on any other result.
S=$(cd "$(dirname "$0")" && pwd); C=src/components/Timeline.svelte; T=src/lib/project/timeline.ts
cd /Volumes/rudenet/code/strudeland-worktrees/poptart || exit 2
git show b0229a8:$C > "$S/0133-base-C" && git show b0229a8:$T > "$S/0133-base-T" || exit 2
n() { grep -oF -- "$1" "$2" | wc -l | tr -d ' '; }
fail=0
printf 'kind\tbaseC\tbaseT\tnowC\tnowT\tliteral\n'
while IFS="$(printf '\t')" read -r kind lit; do
  bc=$(n "$lit" "$S/0133-base-C"); bt=$(n "$lit" "$S/0133-base-T"); nc=$(n "$lit" $C); nt=$(n "$lit" $T)
  ok=1
  if [ "$kind" = M ]; then
    [ "$bc$bt$nc$nt" = 1001 ] || ok=0
  elif [ "$kind" = = ]; then
    [ "$bc" = "$nc" ] && [ "$bt" = "$nt" ] || ok=0
  fi
  [ $((bc + bt)) = $((nc + nt)) ] || ok=0
  [ "$bc$bt" != 00 ] || ok=0
  [ $ok = 1 ] || { fail=1; printf 'FAIL '; }
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$kind" "$bc" "$bt" "$nc" "$nt" "$lit"
done < "$S/0133-literals.txt"
exit $fail
```

### `0133-markup.ts`

```ts
// The markup of Timeline.svelte (everything from `</script>` on) must equal b0229a8's with
// exactly the two allowed help replacements. Exit 0 equal, 1 different, 2 setup error.
import { $ } from 'bun';

const REPO = '/Volumes/rudenet/code/strudeland-worktrees/poptart';
const FILE = 'src/components/Timeline.svelte';
const markup = (text: string) => {
  const at = text.indexOf('</script>');
  if (at < 0 || text.indexOf('</script>', at + 1) >= 0) throw new Error('want one </script>');
  return text.slice(at);
};

const ENDING = `        help={endingEnabled
          ? 'Finish the loop pass, then play the ending once.'
          : playingOther
            ? 'The ending belongs to the song currently playing.'
            : !$rt.started
              ? 'Start song playback to reach its defined loop.'
              : $pb.transport.view.kind === 'release'
                ? 'The ending is already queued or playing.'
                : 'Available while the song plays through its defined loop.'}
`;
const STOP = `                        help={$pb.transport.view.kind !== 'loop'
                          ? 'There’s no temporary section loop to clear.'
                          : $rt.started
                            ? 'Return to the song’s normal playback at the next available bar line.'
                            : 'Clear the temporary section loop for the next play.'}
`;
const REPLACEMENTS: [string, string][] = [
  [ENDING, '        help={endingHelp}\n'],
  [STOP, '                        help={stopLoopingHelp}\n'],
];

try {
  let expected = markup(await $`git -C ${REPO} show b0229a8:${FILE}`.text());
  for (const [from, to] of REPLACEMENTS) {
    const parts = expected.split(from);
    if (parts.length !== 2) throw new Error(`base block found ${parts.length - 1} times:\n${from}`);
    expected = parts.join(to);
  }
  const actual = markup(await Bun.file(`${REPO}/${FILE}`).text());
  if (actual === expected) {
    console.log('markup equal: b0229a8 with the 2 allowed replacements');
    process.exit(0);
  }
  const a = expected.split('\n');
  const b = actual.split('\n');
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] !== b[i]) {
      console.log(`first difference at markup line ${i + 1}:\n- ${a[i]}\n+ ${b[i]}`);
      break;
    }
  }
  console.log(`expected ${a.length} lines, got ${b.length}`);
  process.exit(1);
} catch (e) {
  console.error(String(e));
  process.exit(2);
}
```
