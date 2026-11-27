# 0144 proposal: typed length errors and the loop preset gate

Evidence and decisions: [reverify.md](reverify.md) (at `0113154`; its Decisions section is
binding). File and line references below were re-checked at `0113154`.

## Goal

1. **Typed length errors.** The planner's twelve length-request errors carry a typed code with
   its parameters, from the TS planner through the headless host to `strudeland-render`. Today
   they carry a free-text `flag` (`src/lib/render/job-plan.ts:55-69`). The CLI keeps printing
   the planner's own message, byte for byte, with exit 4. The desktop export dialog words the
   error itself, without CLI flags: today it shows `this song has no sections; pass --length N
   to say how many bars to render`.
2. **The loop preset gate.** The dialog disables export for a loop preset when the song marks
   no loop. Today the radio is selectable, the warning shows, and export stays enabled. Pressing
   it fails with the CLI text `--preset needs meta.loop or a section with loop: true in
   meta.sections`.

## Scope

In:

- A TS union `LengthProblem` (12 codes) in `job-plan.ts`. `PlanJobError` carries it as `problem`
  instead of `flag`, and `lengthFlag(problem)` derives the flag.
- The host slot (`src/host/main.ts:77-85`) writes `length: { code, …params }` instead of
  `flag`.
- In `strudeland-host`, a serde `LengthProblem` with `flag()`. `HostError::MetaFlag` becomes
  `{ problem, message }`. `strudeland-render` re-exports `LengthProblem`.
- `crates/strudeland-render/src/export/wording.rs`: an exhaustive
  `dialog_text(&LengthProblem) -> String`. `src-tauri/src/export_audio.rs` `export_message`
  (`:144`) calls it for a `MetaFlag`.
- How the host reads the slot's `length`: a malformed `length` (an unknown code, a missing or
  ill-typed parameter) fails the whole slot as `HostError::Runtime("malformed result slot:
  …")`; a well-formed `length` on a kind other than `meta` is ignored, as `flag` is today.
- In `src/lib/export/controller.ts`, a pure `loopUnavailable(song, options)`.
  `ExportAudio.svelte` uses it for `canExport`, for the export button's help and for the
  existing warning's condition.
- The planner obligations: `host:build`, `engine:refs --write` and `host:parity`, in all three
  modes.
- Tests for each code's TS throw, the Rust round trip, every `dialog_text` wording, the
  `export_message` mapping and `loopUnavailable`.
- The AGENTS.md and host README lines the change makes stale.

Out:

- Any change to CLI text, exit codes or `SKILL.md`.
- The control socket's own `unknown section '…'` text (`src/lib/control/handler.ts:219`). It is
  not a planner error.
- An IPC shape change. `ExportError` keeps `{ kind, message, written }`, so `ipc:build` writes
  no diff.
- Reading the evaluated structure in the dialog. The gate reads the header as typed (see
  Risks).
- Any change to engine jobs, goldens, vectors or ref audio. The only exceptions are the
  provenance hash and the two floor cases' re-render that the provenance rule forces.

## Architecture

```
job-plan.ts  throw new PlanJobError('meta', <CLI text, unchanged>, { code, …params })
     │
src/host/main.ts failure()  →  slot { ok:false, kind:'meta', error, length:{ code, … } }
     │
strudeland-host read_slot  →  HostError::MetaFlag { problem: LengthProblem, message }
     │                          (Display "meta: {message}", kind "meta": unchanged)
strudeland-render RenderError::Plan(..)  ── plan_text: message ──►  CLI "error: {message}", exit 4
     │                                                               (byte-identical)
export_audio.rs export_message
   Render(Plan(MetaFlag { problem, .. }))  →  export::dialog_text(problem)  →  dialog Notice
   any other Render(e)                     →  e.to_string()  (unchanged)
```

The TS planner stays the single owner of the CLI wording. The Rust side never formats CLI text,
so no parity test between TS and Rust formats is needed. The dialog wording lives in Rust, in the
shared export module. Every new code then gets a dialog wording, or the crate does not compile.

## Technical design

### Codes and parameters

| # | Site (`job-plan.ts`) | Code | Params | Flag (`lengthFlag` / `flag()`) |
|---|---|---|---|---|
| A | 329-335 | `presetWithLength` | none | `--preset` |
| B | 391-397 | `loopsWithoutLoop` | none | `--loops` |
| C | 399-403 | `loopsInvalid` | `message` | `--loops` |
| D | 420-425 | `noSections` | none | `--length` |
| E | 430-435 | `loopsPastLimit` | `loops`, `clipBars`, `limit` | `--loops` |
| F | 436-442 | `clipPastLimit` | `clipBars`, `limit`, `hasLoop` | `--length` |
| G | 448-452 | `sectionWithoutSections` | none | `--section` |
| H | 456-460 | `unknownSection` | `name`, `sections` | `--section` |
| I | 463-468 | `sectionPastLimit` | `name`, `toBar`, `limit` | `--section` |
| J | 474-479 | `lengthPastEnd` | `bars`, `clipBars` | `--length` |
| K | 492-497 | `presetWithoutLoop` | none | `--preset` |
| L | 508-513 | `loopPastLimit` | `end`, `limit` | `--preset` |

Every flag equals the literal passed at that site today. `limit` is always `MAX_RENDER_BARS`.
It travels as a parameter, so no Rust code assumes `MAX_BARS == MAX_RENDER_BARS`. C cannot be
reached: `--loops` is validated before it, and the loop is checked first. It stays typed, so no
length error is left without a code.

The wire form is `#[serde(tag = "code", rename_all = "camelCase", rename_all_fields =
"camelCase")]`, with serde 1.0.229 (`Cargo.lock`). Bar counts are `u64` and `loops` is `u32`.
The largest reachable value, 64 sections × 10000 bars × 1000 loops = 6.4e8, fits either type.
A slot whose `length` does not parse fails the whole slot, as
`HostError::Runtime("malformed result slot: …")`. A `length` on a kind other than `meta` is
ignored, as `flag` is today. `"length": null` reads as no problem, so the error is plain `Meta`.

### Dialog wordings (user-visible copy)

These are the dialog's texts after stage 001, and the PR lists them. Like the rest of the
dialog, they are lowercase (AGENTS.md styling rule). `{list}` is the section names joined with
`, `.

| Code | Dialog text |
|---|---|
| `noSections` | `this song has no sections; choose a length in bars` |
| `sectionWithoutSections` | `this song has no sections; choose a length in bars` |
| `unknownSection` | `this song has no section '{name}' (sections: {list}); choose another section` |
| `lengthPastEnd` | `{bars} bars is past the end of the song ({clipBars} bars); choose {clipBars} bars or fewer` (each count's `bars` is `bar` when the count is 1) |
| `clipPastLimit` | `the song's clip is {clipBars} bars, past the render limit of {limit} bars; choose a length in bars` |
| `sectionPastLimit` | `section '{name}' ends at bar {toBar}, past the render limit of {limit} bars` |
| `loopPastLimit` | `the loop ends at bar {end}, past the render limit of {limit} bars` |
| `presetWithoutLoop` | `loop export needs a meta.loop range or a section with loop: true` |
| `presetWithLength` | `loop exports use the marked loop section` |
| `loopsWithoutLoop` | `the loop count needs a meta.loop range or a section with loop: true` |
| `loopsPastLimit` | `{loops} loop passes make the clip {clipBars} bars, past the render limit of {limit} bars` |
| `loopsInvalid` | `{message}` |

Notes:

- `presetWithLength` is the app's existing text (`export_audio.rs:207`).
- `presetWithoutLoop` is a backstop once stage 002 lands, and it matches the dialog warning.
- The app cannot reach `loopsWithoutLoop`, `loopsPastLimit` or `loopsInvalid`, because the
  dialog never sends loops.
- `lengthPastEnd` writes `1 bar`, not `1 bars`, for a count of 1 (for example `2 bars is past
  the end of the song (1 bar); choose 1 bar or fewer`). The test pins a singular and a plural
  row. The other texts' counts are never 1 (`limit` is 100000, and every count past it is
  larger).

The gate's contextual help (stage 002), on the disabled export button, is `This song marks no
loop; mark one to export a loop preset.` It follows `docs/writing-contextual-help.md` point 2:
it starts with the reason the action is unavailable, then gives the next step. It is one
sentence in sentence case, as help sentences are. This updates the Decisions' draft (`Mark a
loop in the song to export a loop preset.`), which led with the step instead of the reason.

### The loop preset gate

`loopUnavailable(song: Pick<ExportSong, 'structure'>, options: Pick<Options, 'mode'>): boolean`
is true iff `loopMode(options.mode)` holds and either `song.structure` is null or
`authoredLoop(song.structure)` is null. In `ExportAudio.svelte`, one `$derived` value feeds:

- `canExport` (`:63`), which adds `&& !unavailable`;
- `exportHelp` (`:69-79`), whose first branch after `locked` returns the gate's help;
- the warning's condition (`:266`), `loopMode(options.mode) && !markedLoop`, which becomes
  `unavailable`. Section names are identifiers (`structure.ts:186`), so `markedLoop` is falsy
  exactly when `authoredLoop` is null, and the warning shows in the same cases as today.

The radios, the range label (`markedLoop || 'marked loop'`) and the warning text are
unchanged.

The controller's `start()` (`controller.ts:261`) also returns early when the gate holds. This
is the testable form of the wiring, because the repo has no Svelte component tests. It also
covers a form submit that does not go through the button. The preview is still requested, so
the destination keeps showing.

## Dependencies

- Planner rules (AGENTS.md, Rust engine and headless host):
  - `job-plan.ts` is in `PLANNER_FILES` (`scripts/engine/provenance.ts:30-39`);
  - `job-plan.ts` and `src/host/main.ts` are in `host.inputs.json` (`:93`, `:80`).

  So stage 001 runs `host:build`, `engine:refs --write`, `host:parity` (single, `--batch` and
  `--mixed-meter`) and `cargo test -p strudeland-host`.
- Chromium for Playwright is installed (`~/Library/Caches/ms-playwright/chromium-1248`).
- `cargo test --workspace` needs cmake, a C compiler and the V8 download.

## Risks

- **A bundle rename outside the edited modules.** Bun renames a colliding top-level identifier
  with a numeric suffix, and that would put `host.js` hunks in other modules. At `0113154`, the
  names `lengthError`, `lengthFlag`, `LengthProblem` and `lengthProblem` occur 0 times in
  `host.js`. The host-diff gate catches any other collision.
- **A parameter that does not fit its Rust type**, for example a non-integer. That would turn
  exit 4 into exit 1 (`Runtime`). Every parameter is an integer from a validated structure or a
  validated request. The length A/B covers every reachable code through the real CLI. Both
  tests in `never_panics.rs` still pass.
- **A stale bundle.** A host whose bundle still writes `flag` would read every length error as
  plain `Meta` (exit 1). The bundle is compiled in (`bundle.rs`), and the `host.inputs.json`
  staleness test in `bun test` fails until `host:build` has run.
- **The gate reads the header as typed.** These go in the PR notes, together:
  - A song whose header lacks the loop but whose `song()` registers one would be blocked. The
    dialog already builds its range options from the header. This limit is accepted
    (Decisions).
  - A header that does not parse (`song.structure` is null) makes the gate block every loop
    preset, with the loop help. The planner would refuse such a song anyway, with a `meta`
    error.
  - A header that marks a loop while the song's `song()` structure has none passes the gate.
    The planner then refuses it as K, and the dialog shows K's text (`loop export needs a
    meta.loop range or a section with loop: true`).
- **The floor cases' re-render** (`mod-lfo-note`, `vowel-formants`). Their job files carry the
  provenance hash, so they **will** change: `settleFloorCase` sees `the job changed`, replaces
  each WAV with a fresh render and measures its `floorDb` in `index.json` again. The
  `fixtures.floorDb` in `tests/refs/thresholds.json` is a tolerance, and `--write` does not
  rewrite it. That is the provenance rule's known effect. Nothing else in the refs may move.

## Open questions

1. `start()` also refuses while the gate holds (above). The Decisions name only the component.
   **Recommended:** keep it. It adds no visible behaviour, and it is the only way to test the
   wiring.
2. `loopsInvalid` (C) has no TS throw test, because no input reaches it. Its shape is covered by
   the TS type and by the Rust literal round trip. **Recommended:** accept this, rather than
   delete the catch (the Decisions keep C) or mock `withLoops`.
3. `lengthFlag` (TS) and `LengthProblem::flag()` (Rust) have no production caller once `flag`
   leaves the slot. Only tests read them, to pin today's flag per code. **Recommended:** keep
   them, as the Decisions say. The flag is the code's documented CLI meaning.

## Stages

- [001-typed-length-codes.md](001-typed-length-codes.md)
- [002-loop-preset-gate.md](002-loop-preset-gate.md)

The entry point is [000-prompt.md](000-prompt.md), and the loss check is
[loss-check.md](loss-check.md). Complexity: medium, about 20 files across one subsystem chain.
The stages are sequential, and 002 does not depend on 001's code.
