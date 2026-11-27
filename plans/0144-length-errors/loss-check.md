# 0144 loss check

Sources compared: the [reverify.md](reverify.md) recommendations, §5 wordings and Decisions,
and the planning brief. Targets: [proposal.md](proposal.md), [000-prompt.md](000-prompt.md),
[001](001-typed-length-codes.md) and [002](002-loop-preset-gate.md).

## Preserved

| Item | Where |
|---|---|
| Twelve codes, one per site (A-L), C kept typed | proposal Codes; 001 steps 2-3 |
| Slot `length: { code, …params }` replaces `flag`; an unknown code is `Runtime("malformed result slot …")` | 001 steps 4, 7 |
| `LengthProblem` serde enum in `strudeland-host` (`tag = "code"`, camelCase), re-exported by `strudeland-render` | 001 steps 6, 9 |
| `HostError::MetaFlag { problem, message }`, flag derived (`flag()`, `lengthFlag`); Display and `exit_class` unchanged | 001 steps 2, 6, 9 |
| CLI carries the TS message; byte-identical, exit 4, `SKILL.md` unchanged | 000 contract; 001 steps 3, 10; length A/B |
| Render limit as a parameter (`limit`), not `MAX_BARS` | proposal; 001 steps 3, 9 |
| `export/wording.rs` exhaustive `dialog_text`, called from `export_message` (`export_audio.rs:144`) | 001 steps 9, 11 |
| The §5 wordings, verbatim, as user-visible copy | proposal Dialog wordings; pinned by `every_problem_has_its_dialog_text` |
| No IPC shape change; `ipc:build` empty diff | 000 contract, gate step 9 |
| Planner obligations: `host:build` (diff in `job-plan.ts`/`main.ts` regions only), `engine:refs --write` (provenance and the two floor cases), `host:parity` single, `--batch` and `--mixed-meter`, `cargo test -p strudeland-host` | gate steps 1-5, 13, 16 |
| Baseline before any edit of CLI stderr and exit for every reachable length error (synthetic songs; `strudeland render` and the harness), byte-identical after | 001 step 1 (35 cases), gate step 12 |
| `plan-render` plans byte-identical on the corpus; engine jobs, goldens and vectors unchanged | gate steps 6, 7, 11, 16 |
| Tests: each code's TS throw, the Rust round trip of every variant, `dialog_text` for every variant, the `export_message` mapping | 001 steps 5-11 |
| `loopUnavailable(song, options)` in the controller; export disabled; help `Mark a loop in the song to export a loop preset.`; radios and warning unchanged | 002 steps 1-2 |
| `markedLoop` and the gate cannot disagree (reverify §6) | 002 step 2 (the warning reads `unavailable`) |
| Accepted limit: the header as typed | proposal Risks; 002 step 1 doc comment; PR note |
| Out of scope: the control socket's `unknown section` text | proposal Scope |
| One PR, two commits, in the decided order | 000 Stages; the stage commit messages |
| STOP on any CLI text or exit change, job, golden or vector change, parity diff, or IPC binding change | 000 STOP list |
| AGENTS.md lines to update | 001 step 12 (`:298`, `:308-310`); 002 step 4 (`:160-165`); host README in 001 |

## Modified (justified)

- The TS class property is `problem`, matching Rust's `MetaFlag { problem }`. The slot key
  stays `length`, as decided. The brief names only the slot.
- "Plans (minus the new field)": this task adds no plan field, so the plan A/B compares whole
  plans with nothing removed.
- The "TS throw for each code" covers eleven codes. C cannot be reached by any input, so its
  shape is pinned by the TS type and the Rust literal round trip (proposal, Open questions 2).
- The test of the component's `canExport` wiring: the repo has no Svelte component harness, so
  the wiring is documented (002 step 3). It is also made testable through the controller's
  `start()`, which refuses while the gate holds (proposal, Open questions 1).
- Reverify §1 missed some pinning sites, and 001 now lists them:
  - `tests/render-job-plan.test.ts:1162-1170`, `:1442-1446` and `:1646-1661`;
  - `crates/strudeland-host/tests/never_panics.rs:204`, which needs no edit;
  - the doc at `crates/strudeland-host/src/host.rs:95`.
- The `host.js` allowed regions changed since #129: `job-plan.ts` now ends at 18946, and there
  are two `main.ts` regions. The script computes the regions from the file instead of
  hard-coding them.

## Intentionally dropped

- The `perf_budget` gate. No planning path changes, only the throw sites.
- `render:check`. Job identity is proven by the plan A/B and by `host:parity`.

## Unintentionally dropped

None.
