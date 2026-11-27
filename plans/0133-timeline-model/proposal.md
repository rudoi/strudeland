# 0133: the timeline view as one pure function

## Goal

`Timeline.svelte` still derives what it shows from what plays in about 25 lines of `$derived`
and two help ternaries in its markup (`:69-72`, `:93-107`, `:115`, `:183-191`, `:277-281` at
`b0229a8`). None of it is tested. The risky part is the block playhead's name lookup across
the evaluated and header structures, and the ending's 5-way help.

After this change one pure function, `timelineView` in `src/lib/project/timeline.ts`, derives
all of it from plain inputs. It follows the shape of `sectionItemHelp`. The component reads
it through one `$derived` and keeps the markup, the rAF `position` polling, the menu and focus
state, and the commit glue (the outcome-to-message mapping included). Plain-object tests in
`tests/timeline.test.ts` then cover playing another song, an evaluated structure that differs
from the header, arming while stopped, the duplicate-name first match, the missing section,
and both help tables.

**No visible change.** The DOM, classes, `data-*` attributes, help hints and their texts,
copy, focus, pointer and keyboard handling, and the rAF loop stay as they are. The markup
outside `<script>` changes in exactly two attributes, which now read precomputed strings
(see "Allowed markup replacements").

## Background

[reverify.md](reverify.md) holds the evidence at `b0229a8`, and its **Decisions** section is
binding: a pure function, not a store model with actions; the help strings move verbatim;
`playable` stays one line in the function (song-playback's `inForce` is not exported);
first-match on duplicate names and no block playhead for a missing section, both pinned; the
stale `position` after a stop, the rAF and the commit mapping stay in the component; the
existing one-liners and their tests stay.

## Scope

In:

1. `src/lib/project/timeline.ts`: `TimelineViewInput`, `TimelineView`, `timelineView`, and the
   module doc line.
2. `src/components/Timeline.svelte`: the script's derivations become one destructured
   `$derived(timelineView(...))`, and the two help attributes read the strings it returns.
3. `tests/timeline.test.ts`: a new `describe('timelineView')`. Existing tests are untouched.
4. `AGENTS.md:137`, the `timeline.ts` entry names `timelineView`.

Out:

- Any action wrapper (`playFrom`, `stopLooping`, `playEnding`, `commit`): they are already
  single tested calls (reverify §1, Actions).
- The stop-looping `disabled` test (`:276`), the menu items' `disabled`, the status line and
  the meta-error hint: they stay in the markup as written.
- `commitProblem(outcome)` (reverify Decision 7, optional). Not done.
- song-playback, the arbiter, transport, structure, session, and every other file.

## Technical design

```ts
import type { SessionState } from './session';
import type { PlaybackState } from './song-playback';
import type { RuntimeState } from '../strudel/runtime';
import { canPlayEnding, type Located } from '../strudel/transport';

/** What `timelineView` reads: store snapshots and the polled position. */
export interface TimelineViewInput {
  /** The edited header (`editableStructure`: none while it has a meta error) and the active song. */
  session: Pick<SessionState, 'active' | 'structure' | 'metaError'>;
  /** `started`, and the last successful evaluate's structure. */
  runtime: Pick<RuntimeState, 'started' | 'structure'>;
  playback: Pick<PlaybackState, 'playing' | 'transport'>;
  /** `runtime.position()`, polled by the component; stale after a stop, and every reader is gated. */
  position: { cycle: number; located: Located | null };
}

export interface TimelineView {
  playingActive: boolean;
  playingOther: boolean;
  /** The evaluated structure while started, else the header. */
  playable: SongStructure | null;
  ending: boolean;
  endingEnabled: boolean;
  endingHelp: string;
  stopLoopingHelp: string;
  pct: number | null;
  head: { section: number; fraction: number; localCycle: number } | null;
  armedName: string | null;
  auditionNames: string[];
}

export function timelineView(input: TimelineViewInput): TimelineView;
```

The body is today's component expressions, moved and renamed only (`$rt` to `runtime`, `$pb`
to `playback`, `$ss` to `session`, `structure` to `editableStructure(session)`, `sections` to
`structure?.sections ?? []`). The operand order of every `&&`, `?:` and `??` stays.

- `playingActive = playing !== null && playing.path === session.active`;
  `playingOther = playing !== null && !playingActive`.
- `playable = runtime.started ? runtime.structure : structure`; `ending = hasEnding(playable)`.
- `pct = playingActive && sections.length === 0 ? playheadPct(runtime.structure, position) : null`.
- `head`: null unless `playingActive` and `position.located`. Else the name of
  `playable?.sections[located.section]`, its first index in `sections` (`findIndex`, so -1
  when missing), and `blockPlayhead(structure, { ...located, section })`, which gives null at
  -1.
- `endingEnabled = canPlayEnding(runtime.started, runtime.structure, transport.view, located) && playingActive`.
- `endingHelp` and `stopLoopingHelp`: today's ternaries with the same conditions, order and
  strings (table below).
- `armedName = structure ? transport.armed : null`. The gate is on `structure`, as today, not
  on `metaError`. They differ when the session has no structure and no error.
- `auditionNames = transport.loop?.names ?? []`.

Why `session` and not a precomputed `structure`: the function then takes the meta error as an
input, as asked. Tests can then state "meta error" directly. The component keeps its own
`editableStructure($ss)` for the markup. It is the same pure call on the same snapshot, so the
two cannot disagree.

### Help texts that move (byte-identical, curly apostrophes included)

| Help | Condition, in order | Text |
|---|---|---|
| ending | `endingEnabled` | `Finish the loop pass, then play the ending once.` |
| | `playingOther` | `The ending belongs to the song currently playing.` |
| | `!runtime.started` | `Start song playback to reach its defined loop.` |
| | `view.kind === 'release'` | `The ending is already queued or playing.` |
| | otherwise | `Available while the song plays through its defined loop.` |
| stop looping | `view.kind !== 'loop'` | `There’s no temporary section loop to clear.` |
| | `runtime.started` | `Return to the song’s normal playback at the next available bar line.` |
| | otherwise | `Clear the temporary section loop for the next play.` |

Every other help and copy string stays where it is.

### Allowed markup replacements

Everything after `</script>` is byte-identical to `b0229a8` except:

1. `Timeline.svelte:183-191` (`help={endingEnabled` … `'Available while the song plays through its defined loop.'}`)
   becomes the one line `        help={endingHelp}`.
2. `Timeline.svelte:277-281` (`help={$pb.transport.view.kind !== 'loop'` … `'Clear the temporary section loop for the next play.'}`)
   becomes the one line `                        help={stopLoopingHelp}`.

Every other markup reader keeps its name, because the destructured `$derived` binds
`playingOther`, `ending`, `endingEnabled`, `pct`, `head`, `armedName` and `auditionNames` under
today's names. Both elements stay over prettier's 100 columns after the replacement (about 122
and 124), so prettier keeps one attribute per line.

## Stages

One: [001-timeline-view.md](001-timeline-view.md). The entry point is
[000-prompt.md](000-prompt.md).

## Risks

- **A changed string or condition order.** A literal-count script (base against the tree,
  both files) and the tests' exact expected strings catch it.
- **A markup edit beyond the two attributes.** The markup script rebuilds the expected markup
  from `b0229a8` with exactly the two replacements and byte-compares.
- **Per-frame recomputation.** The view reruns on every rAF tick, as `pct`, `head` and
  `endingEnabled` already do. Destructured fields are separate deriveds, so markup that reads an
  unchanged primitive does not rerun. `head` is a new object each frame, as today.
  `auditionNames` is now a fresh `[]` each frame when there is no loop. Before, it was fresh on
  each `$pb` change. The only reader is `.includes`, so nothing visible changes.

## Open questions

1. Should the stop-looping `disabled` (`:276`, `view.kind !== 'loop'`) also come from the view,
   so the help and the disabled state share one condition? The plan leaves it in the markup
   (smallest markup diff). Moving it adds a third allowed replacement.
2. Nobody starts the app, by rule. The markup proof stands in for a visual check. The user may
   want to look at the timeline (playing, armed, the ending button's hints) by hand before the
   merge.
