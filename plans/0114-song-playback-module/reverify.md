# 0114 re-verification (step 0)

I checked these on `main` at 274a215, after the song header module landed. The review that raised
them ran against 63b945d.

| Claim from the review | On current `main` | Verdict |
|---|---|---|
| "The playing song" is rebuilt from `runtime.started` + `session.evaluated` | `App.svelte:36` `playingPath`, `Timeline.svelte:51-52` `playingActive`/`playingOther`, `control/handler.ts:330-333` `state.playing`, `handler.ts:459-463` `shouldAutoEval` | Confirmed |
| New rebuild of the same thing | `PianoRoll.svelte:178-182` `matchesLive` (`$song.evaluated?.path` + `activeCode`), `:265` gates the playhead on `started` | Fifth copy |
| The session resets the transport in four places | Now five call sites in four functions: `install` `:822`, `evaluateSong` `:1077` (header error while stopped), `:1081` (instrument error while stopped, new), `:1089` (another song takes over), `activeTransport` `:1162` (via `switchSong` `:1153`, `deleteTask` `:1259`) | Confirmed, and grown |
| Reset outside the session | `recording/backing.ts:110` (a failed start disarms) | Sixth site |
| Tests assert reset order, not outcomes | `tests/helpers/stub-runtime.ts:47,112,143`; `session.test.ts` calls `resets()` 23 times (`:2184-2207`, `describe('transport reset')` `:2897-2960`) | Confirmed |
| `armedSection` is an index looked up against different structures | Timeline against the song header (`:101-103`); handler against the evaluated structure (`:363-372`, for the loop view as well); recording controller against the song header (`controller.ts:365-373`) | Confirmed, wider than the review said |
| `matchingSectionRange` only translates between the two | `project/timeline.ts:18`, one caller (`Timeline.svelte:105`) | Confirmed |
| "Arm, then evaluate if stopped" is duplicated in the Timeline and in backing | Duplicated twice inside the Timeline (`:113-119`, `:124-130`). `backing.ts:94-114` is a different flow: evaluate, resolve against the evaluated structure, arm, start, disarm on failure | Partly wrong |

## A live bug

1. Stop song A, switch to song B and arm one of B's sections.
2. B's evaluate fails with a runtime error.
3. The arm stays, and the runtime's structure is still A's.

The timeline then says "will start at <B's section>". `strudeland state` names the section at that
index in A, or null.

## Conclusion

Still worth doing. The re-scope:

- six reset sites, not four;
- the handler names the loop view as well as the armed section;
- PianoRoll is a fifth place that rebuilds "what is playing";
- the shared flow is "arm, start, disarm on failure", not "arm, then evaluate".
