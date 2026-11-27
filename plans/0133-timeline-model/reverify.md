# 0133 timeline model: step 0 re-verification

At b0229a8 (origin/main). The claim was reviewed against 63b945d (v0.1.2). Since then
`Timeline.svelte` changed in three commits: 8fce514 (contextual help), afb75fc (#28, one owner
for the playing song and its transport, the "#114" the task cites) and 7216aad (#29, the play
arbiter). `timeline.ts` gained `sectionItemHelp` (#29). Most of the claimed component logic has
moved out of the component. What remains is about 25 lines of derivation and two inline help
ternaries.

## 1. Timeline.svelte's script today (516 lines; script `:1-167`)

### Derived values

| Value | Where | Goes through | Status vs the 63b945d claim |
|---|---|---|---|
| `structure` | `:56` | `editableStructure($ss)` (`timeline.ts:93`) | unchanged one-liner |
| `sections`, `bars`, `marks`, `ticks`, `mixedRuler` | `:57-64` | `rulerBars`, `rulerMarks`, `minorTicks`, `mixedMeterRuler` | layout, tested |
| `savedLoop`, `hasLoop` | `:65-66` | `authoredLoop` (structure.ts) | trivial |
| `playingActive` / `playingOther` | `:69-70` | `$pb.playing.path === $ss.active` (song-playback store) | **still in the component**, now reading `playback.store.playing` rather than `$rt.started && $ss.evaluated` |
| `playable` | `:71` | `$rt.started ? $rt.structure : structure` | **still in the component**. It repeats song-playback's private `inForce` (`song-playback.ts:315-317`) |
| `ending` | `:72` | `hasEnding(playable)` | the one-liner is tested, `playable` is not |
| `position` + rAF `$effect` | `:75-88` | `runtime.position()` each frame while `$rt.started` | stays in the component (as intended). `position` is not cleared on stop, but every reader gates on started/`playingActive` |
| `pct` | `:93-95` | `playingActive && no sections ? playheadPct($rt.structure, position)` | gate in the component |
| `head` | `:96-102` | evaluated section index → name in `playable` → `findIndex` by name in the static `sections` → `blockPlayhead` | **still in the component, untested**. `blockPlayhead` tests (`timeline.test.ts:100-135`) pass an index directly, so the name mapping across evaluated and header structures is never exercised. A missing name gives `-1`, and `blockPlayhead` then returns null. With duplicate names the first one wins |
| `auditionNames` | `:103` | `$pb.transport.loop?.names` | **moved** to song-playback `viewLoop`/`resolveIn` (`song-playback.ts:240-248`, `:212-216`). Tested (`song-playback.test.ts:314` renames on hot reload) |
| `endingEnabled` | `:104-107` | `canPlayEnding($rt.started, $rt.structure, $pb.transport.view, position.located) && playingActive` | gate in the component. `canPlayEnding` is in transport.ts |
| `armedName` | `:115` | `structure ? $pb.transport.armed : null` | **moved**: the name comes from song-playback (`:288`). Only the meta-error gate stays here. Tested in `song-playback.test.ts:413-440` |
| `itemHelp` | `:42-53` | `sectionItemHelp` (`timeline.ts:253`) with `$play` and `$rt.started` | **moved** (#29), tested (`timeline.test.ts:264-319`) |

### Actions

| Action | Where | Goes through | Status |
|---|---|---|---|
| `openMenu` | `:117-121` | local `menuFor`/`choosingEnd` | UI state |
| `playFrom(choice)` | `:123-126` | `arbiter.playFrom` → `session.playback.arm` and, if stopped, `play()` (`play-arbiter.ts:175-181`) | **moved**: the claimed `arm`/`armRange` + `app.evaluateDoc()` no longer exist. Tested (`play-arbiter.test.ts:197, 346, 735-755`) |
| `stopLooping` | `:127-130` | `playback.stopLooping()` (`song-playback.ts:438-442`) | **moved**, tested (`song-playback.test.ts:375-395`) |
| play ending | `:181` | `playback.playEnding()` | **moved**, tested (`:396`) |
| `showRangeOptions` | `:131-136` | `tick()` then DOM focus (`[data-range-focus]:not(:disabled)`, else the first menu button) | DOM-only |
| `onFocus` / `commit` | `:142-167` | `commitInput` (`timeline.ts:212`) | pure part tested (`session.test.ts:2974-3000`). The outcome→message mapping and the input reset via `fieldValue` stay in the component (2 copy strings) |

### Logic in the markup

- Ending help: a 5-way ternary at `:183-191` over `endingEnabled`, `playingOther`, `$rt.started`, `view.kind === 'release'`. Untested.
- Stop-looping help: a 3-way ternary at `:277-281`, and disabled `view.kind !== 'loop'` at `:276`. Untested.
- Menu item disabled: `!$canArm(choice) || !$play.canPlayFrom` (`:227, :244, :256`).
- Status line: `playingOther` → `playing: {title}`, else `armedName` → `will start at {name}` (`:172-176`).
- Track hidden while `playingOther` (`:198`). Inputs only with `structure`, else the meta-error hint (`:332-391`).

### timeline.ts exports and callers

Exports: `FALLBACK_BARS`, `MAX_RULER_LABELS`, `rulerStride`, `rulerMarks`, `minorTicks`,
`mixedMeterRuler`, `litBars`, `editableStructure`, `rulerBars`, `hasEnding`, `playheadPct`,
`blockPlayhead`, `InputField`, `ParsedInput`, `parseInput`, `fieldValue`, `MetaEditor`,
`CommitOutcome`, `commitInput`, `SectionItemTexts`, `SectionItemPlay`, `sectionItemHelp`.
Callers: `Timeline.svelte:5-20`, `tests/timeline.test.ts:18`, `tests/session.test.ts:20`
(`commitInput`). Nothing else imports it.

## 2. Tests

| File | Covers | Gaps |
|---|---|---|
| `tests/timeline.test.ts` (319 lines) | mixed-meter ruler, stride/marks, `rulerBars`, `hasEnding`, `blockPlayhead` (by index, clamping, a missing section), `litBars`, `minorTicks`, `catOf`, `playheadPct`, `editableStructure`, `fieldValue`, `parseInput`, `sectionItemHelp` table | no composition: playing-other gating, the name-based block lookup, `endingEnabled`, ending/stop-looping help selection |
| `tests/song-playback.test.ts` (893) | evaluate/playing, disarming, lifecycle, `arm` by name (stopped against the header, playing against the evaluated structure, ranges, meta error, `canArm` recompute), `stopLooping`, `playEnding`, armed name follows the header (`:413-570`), `start`, session integration (`:707+`) | none relevant to this task |
| `tests/play-arbiter.test.ts` (963) | `playFrom` gating by take/memo, plays only when stopped | none relevant |
| `tests/session.test.ts` | `commitInput` against the real session (applied, stale+retry, refused/invalid) | the component's outcome→message copy |

There is no Svelte component test harness in `tests/` (no `mount`/testing-library), so the
component's derivations can only be tested by moving them into a pure function.

The three requested cases:
- **Playing another song.** Untested at the Timeline level. song-playback covers `playing`, but `playingOther`, the hidden track, no head, the disabled ending and its "belongs to the song currently playing" help are untested.
- **Evaluated structure differs from the header.** Untested for the playhead (the `head` name lookup). Tested for arming and loop names in song-playback.
- **Armed while stopped.** The name is tested in song-playback (`:280-296`, `:413-440`). The Timeline's meta-error gate (`:115`) and the status line are untested.

## 3. Visible behaviour that must stay identical

- **DOM.** `section.timeline` > `.status` (hidden when `:empty`) with a status `span` and `Button.ending` (`size="sm"`). `.track` (bound for focus queries) > `.blocks` > `.cell` (`style:flex-grow={bars}`) > `SectionBlock` (`data-cat` from `catOf(i)`, classes `loop`/`current`, `aria-haspopup="menu"`, `aria-expanded`) + `Menu.section-menu` + `.playhead` (`style:left` fraction %). `.ruler` (`class:minor`, `background-size`) with `.meter-ticks` and `.mark.mono`. `ul.meter-changes.mono` (`aria-label="time signature changes"`, `title="bar N · m/n"`). The ruler `.playhead` (`pct`). `.inputs` with `Field`s, `.hint.applies`, `Notice tone="danger"`, `.hint` for a meta error.
- **Data attributes.** `data-range-focus=""` on the range items and "loop through…" (`:226, :269`), which `showRangeOptions` uses for focus. `data-cat` on the blocks.
- **Help hints.** `help` props on `Button`, `MenuItem` and `Field` (`:183-191, :220, :228-235, :245-250, :257-262, :270, :277-281, :336, :347-349, :361, :374`). `use:helpTarget` on the block (`SectionBlock.svelte:52`). Every string stays byte-identical, curly apostrophes included. Help sentences are capitalised: the "UI copy is lowercase" rule exempts contextual help sentences and error text. `docs/writing-contextual-help.md:14` cites the BPM hint.
- **Lowercase copy.** `playing: …`, `will start at …`, `play ending`, `back`, `through {name}`, `start here`, `loop this section`, `loop through…`, `stop looping`, the field labels (`quarter-note bpm`, `meter`/`starting meter`, `loops`, `{name} bars`), `applies on the next evaluate`, `fix the meta header to edit`, and the two problem messages (`the header changed, try again`, `this header cannot be edited here; edit it in the code`).
- **Pointer and keyboard.** A block click toggles its menu (`stopPropagation`). `Menu onclose` closes it. "loop through…"/"back" swap the item list and move focus after `tick()`. The inputs commit on `change` against the doc seen on `focus`.
- **Playhead.** rAF polling only while `$rt.started`, cancelled in the effect cleanup. In the blocks when sections exist (by name), otherwise on the 16-bar ruler.

## 4. Is the friction real, and how big

- The claimed breakable logic has mostly dissolved. `arm`/`armRange` + `evaluateDoc` went to `arbiter.playFrom` (#29). `auditionNames` across the two structures and `armedName` went to song-playback (#28). All of it is tested.
- What remains is about 25 lines and roughly 15 branches: `playingActive`/`playingOther`, `playable`, `ending`, `pct`, `head` (the name mapping, 3 branches), `endingEnabled`, the meta-error gate on `armedName`, and the 5-way and 3-way help ternaries in the markup. The `head` name mapping and the ending help are the real untested risk.
- **A store-based `createTimelineModel` with actions is no longer the right seam.** Each action would pass straight through to `arbiter.playFrom`, `playback.stopLooping`, `playback.playEnding` or `commitInput`, which are already single calls with tests. A store wrapper would also have to own the rAF `position`, or take it as an input.
- **A smaller seam fits better: one pure function in `timeline.ts`.** It would follow the shape of `sectionItemHelp`. Something like `timelineView({ structure, active, playing, evaluatedTitle, started, evaluatedStructure, transport, position })` returning `{ playingActive, playingOther, ending, endingEnabled, endingHelp, stopLoopingHelp, pct, head, armedName, auditionNames }`. The component computes it with one `$derived` and keeps the rAF, menu and focus state, and the commit glue. The three requested cases become plain-object tests without stores.

## 5. New callers or copies since 63b945d

- `PianoRoll.svelte:193-197, :280-291` has its own song playhead: `position.located?.songBar` gated on `$playback.started` and on `matchesLive` (evaluated path + code + pins). Its semantics differ and it maps no names, so it is not a copy.
- `control/handler.ts:298-341` names the playhead with `barPosition` and the transport from `pb.transport`. It is a separate summary, not a copy.
- `recording/controller.ts:349` reads `transport.armed`/`loop`. `App.svelte:54-55` reads `playing.path` for the song list.
- Nothing duplicates `head`'s name lookup, `endingEnabled` or the ending help. `inForce` in song-playback is the only other place with the `playable` rule.

## Recommendation: re-scope, then proceed

Scope: add a pure `timelineView` (name to settle) in `src/lib/project/timeline.ts` that derives
the values in section 4. Move the ending and stop-looping help selection into it, with the texts
verbatim. `Timeline.svelte` reads it through one `$derived` and keeps the markup, the rAF
`position`, the menu and focus state, and the commit glue. Tests in `tests/timeline.test.ts`
cover the three requested cases plus the ending-help table. There is no `createTimelineModel`
store, and the actions are not wrapped. Expect roughly -30 lines in the component and +60 in
`timeline.ts`, with tests on top.

Decisions (recommended answer first):
1. **Pure function or store model?** A pure function. The actions are already one-call verbs with tests.
2. **Move the ending and stop-looping help ternaries into `timeline.ts`?** Yes, with byte-identical strings, as `sectionItemHelp` did.
3. **Who owns `playable`?** It stays in the view function (one line). Do not export song-playback's `inForce`: `PlaybackState` does not carry structures, and adding them widens the store's equality for one reader.
4. **Duplicate section names in the `head` lookup.** Keep first-match, as now and as song-playback's `indexIn` does, and pin it with a test.
5. **The evaluated section missing from the header.** Keep "no block playhead" (`findIndex` −1 → null) and pin it.
6. **The stale `position` after a stop.** Keep it: every reader is gated on started/`playingActive`. The function takes `position` as an input, and the rAF stays in the component.
7. **The commit outcome→message mapping.** Leave it in the component. `commitInput` is tested, and the mapping is two strings plus a DOM reset. Optional: a `commitProblem(outcome)` helper if the review wants the copy under test.
8. **The existing one-liners (`editableStructure`, `rulerBars`, `hasEnding`, `playheadPct`).** Keep them as exports. The view function uses them, and their tests stay.
9. **DOM, classes, `data-*`, help, copy, focus and rAF.** Unchanged. A pure refactor, with no copy changes.

## Decisions (2026-11-27, recommended answers taken; none changes visible behaviour)

- **Re-scoped to a pure function.** A pure `timelineView` function goes in `timeline.ts`, not a store model with actions: every action already passes through to tested song-playback and arbiter calls.
- **Help texts:** the two help ternaries move into `timeline.ts` with byte-identical strings, and the help sentences stay capitalised.
- **`playable`:** stays one line in the view function. `inForce` is not exported.
- **Block playhead:** with two sections of the same name, the first match still wins, and a test pins it. A playing section that isn't in the header still shows no block playhead, also pinned.
- **Stays in the component:** the stale position after a stop and the rAF polling. The commit outcome-to-message mapping also stays.
- **Kept:** the existing one-liner helpers and their tests.
- **Unchanged:** DOM, classes, `data-*` attributes, help hints, copy, focus and rAF.
