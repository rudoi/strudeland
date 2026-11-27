# 0136 loss check

The three stages cover the brief and every Decision in [reverify.md](reverify.md). The
proposal's Out list stays out by design.

| Source | Item | Stage |
|---|---|---|
| Brief (b) | A pure `statusNotices(...)` returning the ordered notices with stable ids, text, tone, title and actions; placed with the cross-domain modules at `src/lib/status-notices.ts`, following the `engineNotices`/`recordingNotices` pattern and composing both | 001 |
| Brief (b), Decision b1 | Exact order, copy, tones and the either/or pairs 1/1b and 8/8b; truthiness as in `{#if}` | 001 (row table, either/or table, order test, literal counts) |
| Decision b2 | "audio suspended" stays a separate `Button` after the list, with its help | 001 (markup proof; literal rows for its copy and help) |
| Decision b3 | The `title` tooltips on the engine and control notices are unchanged, `title=""` for a null path included | 001 (rows 8, 10, 11 and the `toStrictEqual` checks; markup replacement 2) |
| Decision b4 | The three action hints and their mapping to action kinds stay in the component; the function returns only the kind | 001 (action branch byte-identical; literal rows) |
| Decision b5 | Stable ids per source, recording ids unchanged; a changed message updates in place | 001 (id rows, stable-id test, uniqueness) |
| Decision b6 | The Settings `prefs:` duplicate is out of scope | Out |
| Brief (b) proof | Literal counts of every copy, help and title string across the component and the module; a whole-file markup rebuild with the allowed replacements listed exactly | 001 (`0136-001-literals.sh`, `0136-markup.ts`; re-run in 002 and 003) |
| Brief (c), Decisions c1-c2 | `SuperdoughBackend` with `graph` and `masterLatencySec`; both leave `AudioBackend`; `createSuperdoughBackend` returns it; `app.ts` drops the `?.` and the structural type; `fakeRuntime` loses its `graph` option and its self-test | 002 |
| Decision c3 | No preview-graph port; `prepareAudition` keeps superdough hard-coded. Its doc line goes on `SuperdoughBackend`, because `runtime.ts` is a host-bundle input | 002 (runtime.ts frozen) |
| Brief (c) proof | Type-only: `bun run check`, and proof that no runtime path changed | 002 (`0136-js.ts`: transpiled JS equal except the three `?.` sites and the fake's option; numstat; `?.` grep) |
| Brief (c), sound decisions | `graph().destination` is `destinationGain`; null before audio runs and mid-reset; delay from master latency; previews resume superdough while Rust plays; the pinning tests unchanged and passing | 002 (Tests section) |
| Brief (a), Decisions a1-a2 | One options object with the same defaults and an optional `backends` field; the test calls rewritten; the runtime still built inside; the `runtime.stop` monkeypatches stay | 003 |
| Decision a2, brief (a) | `prepareAudition` read at call time; `drainLevels` too, since it is the same (no `this`, no arguments) | 003 (new call-time test; markup replacement 3) |
| Brief (a) | "App.svelte and the 13 test calls updated": 12 test calls change; `App.svelte`'s `createApp()` already matches | 003 (decided question 2) |
| Brief | `AGENTS.md` lines | 001 (new `status-notices.ts` bullet), 002 (`src/lib/audio/` sentence), 003 (none: no line describes `createApp`) |
| Brief | Host-bundle inputs and `PLANNER_FILES` | None touched; `runtime.ts` frozen (proposal, Host bundle) |
| Brief | Gates logged with exit codes under `$S`: format, lint, check, test, proofs | 000-prompt.md (the full gate) and each stage's Validation |
| Brief | STOP on any copy, order, tone, help or markup change beyond the allowed list, or any runtime or sound change | 000-prompt.md |

Changed from the brief, with reasons:

- `App.svelte` is not edited (above).
- `fakeRuntime`'s superdough fake stays an `AudioBackend`. It goes to `createRuntime`, whose
  `backends.superdough` stays `AudioBackend`.
- The existing app tests keep the real backends; four new tests use fakes (decided question 1).
- `SuperdoughBackend`'s `readScope`, `onLevels`, `setLimiter` and `limiterAvailable` stay
  optional (decided question 3).
- Nobody starts the app; the PR description suggests a manual look at the status bar (decided
  question 4).
