# 0115 re-verification (step 0)

Checked on `main` at afb75fc, after the song playback module landed. The review ran against
63b945d. Baseline: `tests/control-handler.test.ts` passes (154 tests).

## Confirmed

Every claim still holds, at new lines:

- **The rules in `app.ts`:** `takeActive`/`memoActive` (`:139-142`) and `evaluateDoc`
  (`:178-184`: `if (takeActive) return; stopAuditions(); playRequests++; memos?.stopPlayback();
  void session.saveAndEvaluate()`).
- **`Toolbar.svelte`:** `taking` (`:26`), `memoAudible` (`:28-30`), and `stop()` (`:56-63`, which
  ends the take, else stops the memo and the song).
- **`control/handler.ts`:** `ControlPlayback`/`NO_PLAYBACK` (`:254-268`), `evaluateGuarded`
  (`:493-506`), `stop` (`:514-524`).
- **`memoAudible` appears three times:** `app.ts:237`, `Toolbar.svelte:28-30` and
  `InstrumentBrowser.svelte:34-36`.
- **During a take, play disagrees.** The app refuses it; control evaluates with
  `autostart:false`, although its comment claims parity with the toolbar.
- **Save differs by design.** The user's play saves; control never saves
  (`SKILL.md:113-118`).
- **`Toolbar.stop()` is untested.**

## Missed by the review

- **Cmd/Ctrl-. bypasses every rule** (`editor/setup.ts:37-41`, which calls `runtime.stop()`).
  - During a take, it stops the take's backing song and the recording continues.
  - During a memo with song backing, it stops the song and leaves the memo playing out of sync.
- **Cmd-Enter flashes even when play is refused** (`setup.ts:26-29`).
- **The Timeline's `playFrom` jumps a take's or a memo's song backing while it plays.**
- **`RecordControls.svelte:72` stops the memo before a take starts.** That is another copy of a
  rule.
- **`takeRunning` is re-derived in five places.**
- **`strudeland stop` prints `not playing`** after it stopped a memo.

## Conclusion

Still worth doing, with a wider scope. The full behaviour table is in the proposal.
