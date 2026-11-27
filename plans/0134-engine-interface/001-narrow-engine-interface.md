# 001: narrow the engine's public interface

## Goal

`capability` and `orbit` become crate-private. The root re-exports `UnsupportedSignature`,
`unsupported_signatures` and `CHAIN_KEYS`, and stops exporting `Engine` and `quantum_end_time`.
`Rendered` carries `unsupported: Vec<UnsupportedSignature>` in place of the formatted
unsupported errors. The engine binary and `render_one` build their texts from that field,
byte-identical to today. Nothing visible or audible changes (see [000-prompt.md](000-prompt.md),
Rules).

## Prerequisites

- The tree is code-identical to `7c6e632`: `git diff --name-only 7c6e632 HEAD` lists only
  `plans/`.
- `bun install` has run, the stable toolchain builds the workspace, and `rustup toolchain list`
  shows `nightly` (for the surface listing only) and the `x86_64-pc-windows-msvc` target.

## Files

- Edited: the 23 paths in 000-prompt.md's Rules, `tests/orbit_pairs.rs` deleted among them.
- New, not committed: in `$S`, `0134-ab-jobs.ts`, `0134-ab.sh`, `0134-surface.ts`,
  `0134-surface.sh`, `0134-surface-check.sh`, `0134-surface-added.txt`, `0134-literals.txt`
  and `0134-literals.sh`. The session that wrote this plan already wrote them and checked them
  on the unchanged tree. Their full text is in the appendix; recreate them from it when `$S`
  differs (tabs separate the columns of `0134-literals.txt`), then
  `chmod +x "$S"/0134-*.sh`.
- `0134-ab.sh` builds the engine binary into the repo's own `target/debug` (a plain
  `cargo build`), so its base and after runs share the workspace's build cache. The surface
  listing uses its own `CARGO_TARGET_DIR` under `$S`.

## Steps

1. **Baseline, before any edit.** With `S` and `L` from 000-prompt.md:
   - `L ab-base "$S/0134-ab.sh" base`. Exit 0. Its last line, as measured on the unchanged
     tree when the plan was written, reads
     `base: 260 jobs, 57 non-empty signatures, 55 unsupported summaries, 65 render exit 1, 4 exit 2`
     (181 fixture jobs plus 79 hand-made ones; the 57 include the 2 bad inputs, whose
     `signatures` output is empty). Different counts on the unchanged tree mean the scripts or
     the tree differ from the plan's: STOP.
   - `L surface-base "$S/0134-surface.sh" base`. Exit 0; it prints `364`.
   - `L literals-base "$S/0134-literals.sh"`. Exit 1, with `FAIL` on exactly the 13 rows whose
     two counts differ (their `now` still equals `base`). The other 7 rows pass. This red run
     shows the script detects the change it guards.

2. **Visibility** (`crates/strudeland-engine/src/lib.rs`):
   - `:8` `pub mod capability;` → `pub(crate) mod capability;`; `:24` `pub mod orbit;` →
     `pub(crate) mod orbit;`.
   - `:60` becomes
     `pub use capability::{ALLOWED_KEYS, CHAIN_KEYS, UnsupportedSignature, allowed_sounds, controls_json, unsupported_signatures};`
     (rustfmt wraps it).
   - `:67` `pub use render::Engine;` → `use render::Engine;`.
   - `:82` `pub fn quantum_end_time` → `pub(crate) fn quantum_end_time`. Its doc comment
     stays.
   - Run `cargo clippy -p strudeland-engine --lib 2>&1 | grep -A3 "never used\|never read"`
     (do not log it as a gate). Since `capability` and `orbit` are crate-private, it must name
     exactly the 35 items of step 3: the 32 rows of its table and its 3 deletions. The one-line
     `signature_of` is among them. If the set differs, STOP.

3. **Test-only accessors.** Put `#[cfg(test)]` directly above each `pub fn` line of the table,
   at its indentation, below its doc comment. The lines are those at `7c6e632`; edit from the
   bottom of each file up, or the later lines move:

   | file | line | item |
   |---|---|---|
   | `capability.rs` | 578 | `signature_of` |
   | `delay.rs` | 70, 74 | `DelayKernel::buffer`, `::write_index` |
   | `delay.rs` | 156 | `DelayLine::kernels` |
   | `delay.rs` | 304, 319 | `FeedbackDelay::line`, `::channels` |
   | `djf.rs` | 84 | `Djf::process` |
   | `orbit.rs` | 233 | `orbit_pair` (free function) |
   | `orbit.rs` | 388, 393, 471, 481, 572, 606 | `Orbit::duck_curve`, `::output_gain`, `::reverb`, `::returns`, `::orbit`, `::finish_with` |
   | `orbit.rs` | 686, 760 | `OrbitBank::orbits`, `::process` |
   | `param.rs` | 210, 214, 218, 223 | `Param::default_value`, `::min_value`, `::max_value`, `::events` |
   | `reverb.rs` | 308, 313 | `IrPlan::frames`, `::has_lowpass` |
   | `reverb.rs` | 879, 885 | `Convolver::latency_frames`, `::stages` |
   | `reverb.rs` | 961, 966 | `IrCache::generated`, `::len` |
   | `reverb.rs` | 1137, 1142, 1147, 1152, 1163 | `Reverb::settings`, `::latest`, `::applied`, `::pending`, `::convolver` |
   | `voice.rs` | 253 | `VoiceOutput::to_sends` |

   That makes 32 attributes. Delete the 3 items that nothing uses, tests included:
   - `orbit.rs`: the field `channels: Vec<i64>,` (`:287`), its initializer
     `channels: spec.channels.clone(),` in `Orbit::new` (`:324`), and the method
     `Orbit::channels` with its doc line (`:576-579`, plus the blank line after it).
     `routes: route(&spec.channels, count)` stays.
   - `reverb.rs`: `IrPlan::params` with its doc line `/// The request's settings.`
     (`:317-320`, plus the blank line after it). The field `params` stays: it is still read.

   Two doc-only edits keep rustdoc from linking to now-private items:
   - `live.rs:2`: ``[`crate::Engine`]`` becomes plain `` `Engine` `` (rewrap nothing). No
     other line of `live.rs` changes.
   - `capability.rs:452`: the `CHAIN_KEYS` doc's ``[`CHAIN_UNSUPPORTED_KEYS`]`` becomes plain
     `` `CHAIN_UNSUPPORTED_KEYS` ``.

   Then `cargo clippy -p strudeland-engine --lib` and the same with `--lib --tests` must
   report no dead code. The integration tests do not compile yet, which is expected.

4. **Move `tests/orbit_pairs.rs` into `orbit.rs`'s `mod tests`** (`:816`).
   - Append its `TABLE`, `entries()` and the 3 tests at the end of the module, unchanged except
     for these:
     - `TABLE` is `include_str!("../../../tests/helpers/orbit-pairs.json")` (the path
       relative to `src/orbit.rs`, the form `job.rs:488` uses);
     - `use serde_json::Value;` becomes `serde_json::Value` written in full, or a local `use`
       inside `entries()`. The module already imports `serde_json::json`, so no new top-level
       import is needed;
     - the file's `//!` doc becomes a `// ----` section comment: `// ---- orbit_pair and the
       stems routing against the table shared with the TS tests (tests/helpers/orbit-pairs.json,
       also read by tests/render-engine-job.test.ts)`.
   - The 3 test names stay: `the_table_covers_wrapping_and_non_positive_orbits`,
     `orbit_pair_matches_the_shared_table`, `stems_channels_route_to_the_pair`. Nothing in
     `orbit.rs` uses those names yet. `orbit_pair` and `route` reach them through
     `use super::*`.
   - `git rm crates/strudeland-engine/tests/orbit_pairs.rs`.

5. **`Rendered.unsupported`.**
   - `lib.rs:86-91`: the doc and the field, as in the proposal's Technical design. The field
     goes last.
   - Engine `render.rs:97-125` (`run_observing_overflow`): the `else` branch becomes
     `(Vec::new(), false)`. The result is
     `(Rendered { channels, errors, unsupported }, overflowed)`, where `unsupported` is the
     list from `unsupported_signatures_in`, moved unchanged. The comment
     `// Nothing has rendered on the unsupported path, so the overflow flag is still false.`
     stays. `RenderError` becomes unused in the non-test code, so `:11` becomes
     `use crate::job::{EngineJob, ErrorKind, JobError, bar_of};` (`ErrorKind::Hap` at `:159`
     still uses `ErrorKind`), and `use crate::job::RenderError;` moves into `mod tests`
     (`:194`), next to the `UnsupportedSignature` import, as a plain `use` (not behind any
     condition).
   - Engine `render.rs` unit tests:
     - `unsupported_jobs_render_silence_and_only_unsupported_errors` (`:376`) becomes
       `unsupported_jobs_render_silence_and_report_their_signatures`. The silence assertion
       stays. Then add `assert!(r.errors.is_empty(), "{:?}", r.errors)` and
       `assert_eq!(r.unsupported, vec![UnsupportedSignature { s: "bytebeat".into(), keys: vec!["ir".into()], first_bar: 2 }])`.
     - `a_bus_cycle_is_refused` (`:1639`): `r.errors` is empty, and `r.unsupported` is
       `vec![UnsupportedSignature { s: "bus".into(), keys: vec!["bus-cycle".into()], first_bar: 0 }]`.
       The silence assertion stays.
     - Import `crate::capability::UnsupportedSignature` in `mod tests`, next to
       `use crate::job::RenderError;`.
   - **Engine binary** (`src/bin/strudeland-engine.rs`):
     - `:20-21` become
       `use strudeland_engine::{EngineJob, ErrorKind, RenderError, SAMPLE_RATE, controls_json, render, unsupported_signatures, wav};`
       (rustfmt wraps it).
     - After the WAV write and before `let summary` (`:143`), add:

       ```rust
       // A refused job played nothing; each signature is reported as an `unsupported` error,
       // ahead of the render errors (empty when refused).
       let errors: Vec<RenderError> = rendered
           .unsupported
           .iter()
           .map(|u| {
               let message = format!("s={} keys={}", u.s, u.keys.join(","));
               RenderError::new(ErrorKind::Unsupported, message, Some(u.first_bar))
           })
           .chain(rendered.errors)
           .collect();
       ```

       Then `"ok": errors.is_empty()`, `"errors": errors` and `if errors.is_empty()` replace
       the three reads of `rendered.errors`. The `json!` keys and their order stay.
     - The module doc (`:9-10`, "1 rendered with render errors") stays: unsupported entries
       are render errors on that wire.

6. **Engine integration tests.**
   - `tests/fixtures.rs`:
     - `:19-21` become `use strudeland_engine::wav::{read_f32, write_f32};` and
       `use strudeland_engine::{EngineJob, render, unsupported_signatures};`. `ErrorKind` has
       no other use there.
     - `:252`: `if a.errors != b.errors || a.unsupported != b.unsupported`.
     - After `:277-279`, add
       `if !rendered.unsupported.is_empty() { return Err(format!("unsupported: {:?}", rendered.unsupported)); }`.
     - In `kept_unsupported_fixtures_are_refused_with_their_signatures`, the block from
       `assert!(!rendered.errors.is_empty(), …)` through
       `assert_eq!(messages, want_messages, …)` (`:467-480`) becomes three assertions:
       `rendered.errors` is empty; `rendered.unsupported` equals `unsupported_signatures(&job)`
       (the render reports exactly the detection's list, first bars included); and
       `rendered.unsupported`'s `(s, keys.join(","))` pairs, sorted and deduped, equal `want`.
       The silence assertion stays.
   - `tests/micro.rs:21`: `use strudeland_engine::unsupported_signatures;`. Nothing else
     changes: `micro_jobs_have_no_unsupported_signature` already guards the errors check.
   - `tests/bin.rs:432`: `json!(strudeland_engine::CHAIN_KEYS)`. `:221` stays and pins the
     summary entry.
   - `tests/never_panics.rs`:
     - `:89` `Some(r.errors.len())` → `Some(r.errors.len() + r.unsupported.len())`, so
       every `Some(0)` caller still means "played, no errors".
     - `:1150` becomes `assert!(r.errors.is_empty(), "{:?}", r.errors);` and
       `assert!(!r.unsupported.is_empty());`.
     - `:1262` becomes `let chain_keys = strudeland_engine::CHAIN_KEYS;`.
   - `tests/stems.rs:36-37`: add `assert!(result.unsupported.is_empty(), "{:?}", result.unsupported);`
     and `assert_eq!(result.unsupported, ordinary.unsupported);`.
   - `src/capability.rs` `sound_render` (`:1298-1303`, the helper of
     `every_allowed_sound_renders`): after `let out = crate::render(…).unwrap();` add
     `assert!(out.unsupported.is_empty(), "{:?}", out.unsupported);`, so a refused sound no
     longer passes as a clean render. Its return value stays.

7. **`strudeland-render`.**
   - `src/plan.rs:7`: `use strudeland_engine::{UnsupportedSignature, unsupported_signatures};`.
     Merge it with `:6`'s `EngineJob` import if rustfmt or clippy prefers; either form is fine.
   - `tests/decay_bound.rs:14`: `use strudeland_engine::unsupported_signatures;`, or merged
     into `:15`.
   - `tests/joiner_identity.rs:88-92`: add
     `assert!(rendered.unsupported.is_empty(), "chunk {k}: {:?}", rendered.unsupported);`.
   - `src/render.rs` `render_one` (`:217-223`) becomes:

     ```rust
     if let Some(u) = rendered.unsupported.first() {
         // Unreachable in practice (`plan::take_jobs` refuses first). The engine binary's
         // message for the signature, capped as the engine caps its messages.
         let message = format!("s={} keys={}", u.s, u.keys.join(","));
         let e = EngineRenderError::new(ErrorKind::Unsupported, message, None);
         return fail(format!("unsupported: {}", e.message));
     }
     ```

     The `use` line (`:8`) stays.
   - `src/render.rs` tests:
     - `Fake` (`:329`) gains `unsupported: Vec<UnsupportedSignature>`, `ok()` sets `vec![]`,
       and `fake_render` (`:367`) sets `unsupported: job.unsupported.clone()` in the
       `Rendered` literal (`:372`). Import `strudeland_engine::UnsupportedSignature` in
       `mod tests`.
     - `a_surviving_unsupported_error_fails` (`:498`) becomes
       `a_surviving_unsupported_signature_fails`. `jobs[1].result` is
       `Ok(vec![hap(1, "fine")])` and `jobs[1].unsupported` is
       `vec![UnsupportedSignature { s: "kick".into(), keys: vec!["ir".into()], first_bar: 1 }]`.
       The expected text is `"render failed: chunk 1: unsupported: s=kick keys=ir"`. This is
       the only test expectation that changes. The old fixture's `"s=kick"` was a message no
       engine could produce, because the engine always writes `keys=`.
     - New `a_surviving_unsupported_message_is_capped_as_the_engine_caps_it`: one signature
       `s: "sine"` with 40 keys `unknowncontrol00`…`unknowncontrol39`. The error text is
       `"render failed: chunk 1: "` + `"unsupported: "` + 300 characters, and those 300
       characters are the first 300 of `s=sine keys=unknowncontrol00,…`. Build the expected
       string in the test as `format!("s=sine keys={}", keys.join(","))` cut with
       `chars().take(300)`, not with `EngineRenderError::new`. The literal `s={} keys={}` must
       not appear in the test: the literal table counts it once in this file (`render_one`).
   - `src/pipeline_audio.rs`:
     - `:111-114`: `let Rendered { channels: joined, errors: mut engine_errors, .. } = …`.
     - `render_window` (`:211-214`):
       `Ok(Rendered { channels: joiner.finish()?, errors, unsupported: Vec::new() })`, with a
       comment: `// render_one failed on any chunk with unsupported signatures.`
     - The `Ramp` fake (`:338-341`): `unsupported: Vec::new()`.

8. **README** (`crates/strudeland-engine/README.md:635-636`): `The engine refuses such jobs
   with \`unsupported\` errors.` becomes `The engine refuses such jobs: \`render\` returns
   their signatures in \`Rendered.unsupported\`, and the binary reports them as
   \`unsupported\` errors.` Rewrap the paragraph to the file's width. Change no other line.

9. `cargo fmt --all`, then the targeted runs: `cargo test -p strudeland-engine --lib`,
   `cargo test -p strudeland-engine --test fixtures --test never_panics --test bin --test stems --test micro`,
   `cargo test -p strudeland-render --lib`. Commit (below), then run the full gate from
   [000-prompt.md](000-prompt.md), steps 1 to 9, in order. If a gate fails, fix it, amend, and
   re-run that gate once. If the fix touched code, run gate 6 too.

## Tests

- Changed: `capability.rs`'s `sound_render` helper (it asserts `unsupported` is empty), the
  engine's `render.rs` unit tests (renamed
  `unsupported_jobs_render_silence_and_report_their_signatures`, and `a_bus_cycle_is_refused`),
  `fixtures.rs` (`kept_unsupported_fixtures_are_refused_with_their_signatures`, the repeat and
  supported checks), `never_panics.rs` (`exercise`, the bus-cycle assertion), `stems.rs`
  `capture`, render `joiner_identity.rs`, and render `render.rs`
  `a_surviving_unsupported_signature_fails`.
- Moved: the 3 orbit table tests, now under `orbit::tests::`.
- New: `a_surviving_unsupported_message_is_capped_as_the_engine_caps_it`.
- Pinned and unchanged: `tests/bin.rs:215-222` (the summary entry
  `{kind:"unsupported",message:"s=bytebeat keys=ir",bar:0}`), the signature table, render
  `tests/pipeline.rs` and `tests/bin.rs` (the refusal text), the live `event.rs` wire test, and
  every TS test.

## Validation

- Gate steps 1 to 9 pass (000-prompt.md).
- `diff -r "$S/0134-ab/base" "$S/0134-ab/after"` is empty, and both summary lines read the same.
- `surface-check` prints `base 364 lines, after 256, removed 121, added 13`, and
  `0134-nightly-base.txt` equals `0134-nightly-after.txt`.
- The literal table has no `FAIL` row.
- `git diff 7c6e632 -- crates/strudeland-engine/src/{delay,djf,param,voice}.rs` shows only added
  `#[cfg(test)]` lines. Under `reverb.rs` the only other change is the `IrPlan::params` removal.
  `git diff 7c6e632 -- crates/strudeland-engine/src/live.rs` changes only line 2.

## Commit message

```
refactor(engine): narrow the public interface

capability and orbit are crate-private. The root re-exports
UnsupportedSignature, unsupported_signatures and CHAIN_KEYS, and no
longer exports Engine or quantum_end_time. Rendered reports refused
material as signatures in a new unsupported field instead of formatted
unsupported errors; the engine binary and the render pipeline's
re-check build their unchanged texts from it. Accessors only tests read
compile for tests only, two unused ones go, and the orbit pair table
test moves into the orbit unit tests.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: the scratch scripts

Each block is the file of that name in `$S`.

### `0134-ab-jobs.ts`

```ts
// Writes the hand-made A/B jobs into <out dir>: one job per entry of the engine's signature table
// (`tests/data/unsupported-signatures.json`, built as `table_job` in `tests/bin.rs` builds it), a
// mixed job (supported and unsupported values), one whose unsupported message is longer than
// `MAX_MESSAGE` (300 characters), one bus cycle, and two bad inputs (invalid JSON, schema 1).
// Usage: bun 0134-ab-jobs.ts <repo> <out dir>. Exit 0 written, 2 setup error.
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';

const [repo, out] = process.argv.slice(2);
if (!repo || !out) {
  console.error('usage: bun 0134-ab-jobs.ts <repo> <out dir>');
  process.exit(2);
}
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

function tableJob(values: unknown[]) {
  const n = values.length;
  const pauses = values.map((_, j) => {
    const c = j + 0.5;
    return {
      suspendSec: j === 0 ? 0.0 : j - 0.05,
      haps: [{ t: c, duration: 0.25, cycle: c, v: j, event: j }],
    };
  });
  return {
    schema: 2,
    sampleRate: 44100,
    frames: 44100 * Math.max(n, 1),
    seed: 0,
    cps: 1,
    channels: { count: 2, multiChannelOrbits: false },
    orbits: [{ orbit: 1, channels: [0, 1] }],
    pauses,
    values,
  };
}

const write = (name: string, body: string) => writeFileSync(`${out}/hand-${name}.job.json`, body);

const table = await Bun.file(`${repo}/crates/strudeland-engine/tests/data/unsupported-signatures.json`).json();
table.forEach((entry: { values: unknown[] }, i: number) => {
  write(`table-${String(i).padStart(2, '0')}`, JSON.stringify(tableJob(entry.values)));
});
write(
  'mixed',
  JSON.stringify(
    tableJob([{ s: 'sine', note: 60 }, { s: 'bytebeat', ir: 1 }, { s: 'zzfx' }, { s: 'sine', ir: 'x', irspeed: 2 }]),
  ),
);
const longKeys: Record<string, unknown> = { s: 'sine' };
for (let k = 0; k < 40; k++) longKeys[`unknowncontrol${String(k).padStart(2, '0')}`] = 1;
write('long-message', JSON.stringify(tableJob([longKeys])));
write('bus-cycle', JSON.stringify(tableJob([{ s: 'bus', n: 1, bus: 2 }, { s: 'bus', n: 2, bus: 1 }])));
write('bad-json', '{ "schema": 2, ');
write('bad-schema', JSON.stringify({ ...tableJob([{ s: 'sine' }]), schema: 1 }));
console.log(`wrote ${table.length + 5} jobs to ${out}`);
```

### `0134-ab.sh`

```sh
#!/bin/sh
# The engine binary's outputs over the fixture jobs and the hand-made jobs, into $S/0134-ab/<label>:
# per job the `signatures` stdout, stderr and exit, the `render` stdout (renderMs zeroed), stderr
# and exit, and the WAV's SHA-256; plus `controls --json` and two bad-path renders.
# Usage: 0134-ab.sh <label>. Exit 0 captured, 2 setup error. Compare with
# `diff -r "$S/0134-ab/base" "$S/0134-ab/after"`.
set -u
S=$(cd "$(dirname "$0")" && pwd)
R=/Volumes/rudenet/code/strudeland-worktrees/poptart
label=${1:?usage: 0134-ab.sh <label>}
OUT="$S/0134-ab/$label"
HAND="$S/0134-ab/jobs"
rm -rf "$OUT" && mkdir -p "$OUT/wav" || exit 2
bun "$S/0134-ab-jobs.ts" "$R" "$HAND" || exit 2
(cd "$R" && cargo build -q -p strudeland-engine --bin strudeland-engine) || exit 2
BIN="$R/target/debug/strudeland-engine"
n_jobs=0
for j in "$R"/crates/strudeland-engine/tests/refs/*.job.json "$HAND"/*.job.json; do
  n=$(basename "$j" .job.json)
  "$BIN" signatures --job "$j" > "$OUT/$n.sig.out" 2> "$OUT/$n.sig.err"; echo $? > "$OUT/$n.sig.exit"
  "$BIN" render --job "$j" --out "$OUT/wav/$n.wav" > "$OUT/$n.render.out" 2> "$OUT/$n.render.err"
  echo $? > "$OUT/$n.render.exit"
  sed -E -i '' 's/"renderMs":[0-9]+/"renderMs":0/' "$OUT/$n.render.out"
  if [ -f "$OUT/wav/$n.wav" ]; then
    shasum -a 256 "$OUT/wav/$n.wav" | cut -d' ' -f1 > "$OUT/$n.wav.sha"
    rm "$OUT/wav/$n.wav"
  fi
  n_jobs=$((n_jobs + 1))
done
"$BIN" controls --json > "$OUT/controls.out" 2> "$OUT/controls.err"; echo $? > "$OUT/controls.exit"
"$BIN" render --job /nonexistent-0134/job.json --out "$OUT/wav/x.wav" > "$OUT/missing-job.out" 2> "$OUT/missing-job.err"
echo $? > "$OUT/missing-job.exit"
"$BIN" render --job "$HAND/hand-table-00.job.json" --out /nonexistent-0134/x.wav > "$OUT/missing-dir.out" 2> "$OUT/missing-dir.err"
echo $? > "$OUT/missing-dir.exit"
rmdir "$OUT/wav" 2>/dev/null
unsupported=$(grep -l '"kind":"unsupported"' "$OUT"/*.render.out | wc -l | tr -d ' ')
exit1=$(grep -lx 1 "$OUT"/*.render.exit | wc -l | tr -d ' ')
exit2=$(grep -lx 2 "$OUT"/*.render.exit "$OUT"/missing-*.exit | wc -l | tr -d ' ')
sigs=$(grep -L '^\[\]$' "$OUT"/*.sig.out | wc -l | tr -d ' ')
printf '%s: %s jobs, %s non-empty signatures, %s unsupported summaries, %s render exit 1, %s exit 2\n' \
  "$label" "$n_jobs" "$sigs" "$unsupported" "$exit1" "$exit2"
```

### `0134-surface.ts`

```ts
// The public surface of strudeland_engine, from nightly rustdoc JSON: one line per item reachable
// from the crate root (modules, re-exports, types, their public fields, variants, inherent
// methods and non-synthetic trait impls), sorted. Usage: bun 0134-surface.ts <rustdoc.json>
// Exit 0 listed, 2 setup error.
const [file] = process.argv.slice(2);
if (!file) {
  console.error('usage: bun 0134-surface.ts <strudeland_engine.json>');
  process.exit(2);
}
type Item = { name: string | null; inner: Record<string, any> };
const doc = JSON.parse(await Bun.file(file).text());
const index: Record<string, Item> = doc.index;
const out = new Set<string>();
const seen = new Set<string>();

const typeName = (t: any): string => {
  if (!t) return '?';
  if (t.resolved_path) return t.resolved_path.path;
  if (t.primitive) return t.primitive;
  if (t.borrowed_ref) return `&${typeName(t.borrowed_ref.type)}`;
  if (t.generic) return t.generic;
  return Object.keys(t)[0];
};

function walk(id: string | number, path: string, alias?: string) {
  const item = index[String(id)];
  if (!item) return;
  const kind = Object.keys(item.inner)[0];
  const body = item.inner[kind];
  const name = alias ?? item.name ?? '?';
  const here = path ? `${path}::${name}` : name;
  const key = `${id}@${here}`;
  if (seen.has(key)) return;
  seen.add(key);
  switch (kind) {
    case 'module':
      if (path) out.add(`mod ${here}`);
      for (const child of body.items) walk(child, path ? here : 'strudeland_engine');
      return;
    case 'use':
      if (body.id != null && index[String(body.id)]) walk(body.id, path, body.name);
      else out.add(`use ${path}::${body.name} = ${body.source}`);
      return;
    case 'struct':
    case 'enum':
    case 'union':
    case 'trait': {
      out.add(`${kind} ${here}`);
      const fields = body.kind?.plain?.fields ?? body.kind?.tuple ?? [];
      for (const f of fields) if (f != null && index[String(f)]) out.add(`field ${here}.${index[String(f)].name}`);
      if (body.kind?.plain?.has_stripped_fields) out.add(`field ${here}.<private>`);
      for (const v of body.variants ?? []) out.add(`variant ${here}::${index[String(v)]?.name}`);
      for (const i of body.impls ?? []) {
        const imp = index[String(i)]?.inner?.impl;
        if (!imp || imp.is_synthetic || imp.blanket_impl) continue;
        if (imp.trait) {
          out.add(`impl ${imp.trait.path} for ${here}`);
          continue;
        }
        for (const m of imp.items) {
          const mi = index[String(m)];
          if (!mi) continue;
          const mk = Object.keys(mi.inner)[0];
          if (mk === 'function') {
            const decl = mi.inner.function.sig;
            const args = decl.inputs.map(([n, t]: [string, any]) => `${n}: ${typeName(t)}`).join(', ');
            out.add(`method ${here}::${mi.name}(${args}) -> ${typeName(decl.output)}`);
          } else out.add(`${mk} ${here}::${mi.name}`);
        }
      }
      return;
    }
    case 'function': {
      const sig = body.sig;
      const args = sig.inputs.map(([n, t]: [string, any]) => `${n}: ${typeName(t)}`).join(', ');
      out.add(`fn ${here}(${args}) -> ${typeName(sig.output)}`);
      return;
    }
    default:
      out.add(`${kind} ${here}`);
  }
}

walk(doc.root, '');
console.log([...out].sort().join('\n'));
```

### `0134-surface.sh`

Run its base and after captures in the same session, so the same nightly writes both listings;
each run records `cargo +nightly --version` in `$S/0134-nightly-<label>.txt`, and the two files
must match.

```sh
#!/bin/sh
# Lists strudeland_engine's public surface (nightly rustdoc JSON, hidden items included) into
# $S/0134-surface-<label>.txt. Usage: 0134-surface.sh <label>. Exit 0 listed, 2 setup error.
S=$(cd "$(dirname "$0")" && pwd)
label=${1:?usage: 0134-surface.sh <label>}
cd /Volumes/rudenet/code/strudeland-worktrees/poptart || exit 2
cargo +nightly --version > "$S/0134-nightly-$label.txt" || exit 2
cat "$S/0134-nightly-$label.txt" >&2
CARGO_TARGET_DIR="$S/0134-doc-target" cargo +nightly rustdoc -q -p strudeland-engine --lib -- \
  -Z unstable-options --output-format json --document-hidden-items 2> "$S/0134-rustdoc-$label.log" || exit 2
bun "$S/0134-surface.ts" "$S/0134-doc-target/doc/strudeland_engine.json" > "$S/0134-surface-$label.txt" || exit 2
wc -l < "$S/0134-surface-$label.txt" | tr -d ' '
```

### `0134-surface-check.sh`

```sh
#!/bin/sh
# Compares $S/0134-surface-base.txt with $S/0134-surface-after.txt: every removed line is in
# `capability` or `orbit` (the modules or their items), or one of the five root lines for
# `Engine` and `quantum_end_time`; the added lines are exactly $S/0134-surface-added.txt.
# Exit 0 as planned, 1 otherwise, 2 setup error.
S=$(cd "$(dirname "$0")" && pwd)
export LC_ALL=C
B="$S/0134-surface-base.txt"; A="$S/0134-surface-after.txt"
[ -s "$B" ] && [ -s "$A" ] || exit 2
comm -23 "$B" "$A" > "$S/0134-surface-removed.txt"
comm -13 "$B" "$A" > "$S/0134-surface-added-got.txt"
fail=0
bad=$(grep -Ev '^[a-z_]+ ([A-Za-z]+ for )?strudeland_engine::(capability|orbit)($|::|\.|\()' "$S/0134-surface-removed.txt" \
  | grep -vxF -e 'struct strudeland_engine::Engine' \
              -e 'field strudeland_engine::Engine.<private>' \
              -e 'method strudeland_engine::Engine::new(job: &EngineJob) -> Result' \
              -e 'method strudeland_engine::Engine::run(self: Self) -> Rendered' \
              -e 'fn strudeland_engine::quantum_end_time(quantum_start: u64) -> f64')
if [ -n "$bad" ]; then fail=1; printf 'unexpected removals:\n%s\n' "$bad"; fi
if ! diff "$S/0134-surface-added.txt" "$S/0134-surface-added-got.txt"; then fail=1; echo 'additions differ (< planned, > got)'; fi
printf 'base %s lines, after %s, removed %s, added %s\n' "$(wc -l < "$B" | tr -d ' ')" \
  "$(wc -l < "$A" | tr -d ' ')" "$(wc -l < "$S/0134-surface-removed.txt" | tr -d ' ')" \
  "$(wc -l < "$S/0134-surface-added-got.txt" | tr -d ' ')"
exit $fail
```

### `0134-surface-added.txt`

```text
constant strudeland_engine::CHAIN_KEYS
field strudeland_engine::Rendered.unsupported
field strudeland_engine::UnsupportedSignature.first_bar
field strudeland_engine::UnsupportedSignature.keys
field strudeland_engine::UnsupportedSignature.s
fn strudeland_engine::unsupported_signatures(job: &crate::job::EngineJob) -> Vec
impl Clone for strudeland_engine::UnsupportedSignature
impl Debug for strudeland_engine::UnsupportedSignature
impl Eq for strudeland_engine::UnsupportedSignature
impl PartialEq for strudeland_engine::UnsupportedSignature
impl Serialize for strudeland_engine::UnsupportedSignature
impl StructuralPartialEq for strudeland_engine::UnsupportedSignature
struct strudeland_engine::UnsupportedSignature
```

### `0134-literals.txt`

```text
crates/strudeland-engine/src/render.rs	1	0	s={} keys={}
crates/strudeland-engine/src/bin/strudeland-engine.rs	0	1	s={} keys={}
crates/strudeland-render/src/render.rs	0	1	s={} keys={}
crates/strudeland-engine/tests/fixtures.rs	1	0	s={s} keys={k}
crates/strudeland-engine/src/render.rs	3	0	ErrorKind::Unsupported
crates/strudeland-engine/src/bin/strudeland-engine.rs	0	1	ErrorKind::Unsupported
crates/strudeland-render/src/render.rs	2	1	ErrorKind::Unsupported
crates/strudeland-engine/tests/fixtures.rs	1	0	ErrorKind::Unsupported
crates/strudeland-engine/tests/never_panics.rs	1	0	ErrorKind::Unsupported
crates/strudeland-engine/src/render.rs	1	0	"s=bytebeat keys=ir"
crates/strudeland-engine/tests/bin.rs	1	1	"s=bytebeat keys=ir"
crates/strudeland-engine/src/render.rs	1	0	"s=bus keys=bus-cycle"
crates/strudeland-render/src/render.rs	1	1	unsupported: {}
crates/strudeland-render/src/render.rs	1	0	unsupported: s=kick"
crates/strudeland-render/src/render.rs	0	1	unsupported: s=kick keys=ir"
crates/strudeland-render/src/error.rs	3	3	the song uses sounds or controls the engine cannot play yet; nothing was written:
crates/strudeland-render/src/error.rs	1	1	" keys="
crates/strudeland-render/src/error.rs	1	1	 first at bar {}
src/lib/render/engine-job.ts	1	1	`s=${sig.s} keys=${sig.keys.join(',')} bar=${sig.firstBar}`
crates/strudeland-live/src/event.rs	1	1	firstCycle
```

### `0134-literals.sh`

```sh
#!/bin/sh
# Literal counts at the base 7c6e632 and in the working tree against 0134-literals.txt
# (file<TAB>base<TAB>after<TAB>literal; a file missing at base counts 0). Exit 1 on any count
# that differs from its column, 2 on a setup error.
S=$(cd "$(dirname "$0")" && pwd)
cd /Volumes/rudenet/code/strudeland-worktrees/poptart || exit 2
git rev-parse -q --verify 7c6e632 > /dev/null || exit 2
fail=0
printf 'base\tnow\twant\tfile\tliteral\n'
while IFS="$(printf '\t')" read -r file want_base want_now lit; do
  b=$(git show "7c6e632:$file" 2>/dev/null | grep -oF -- "$lit" | wc -l | tr -d ' ')
  n=$( (cat "$file" 2>/dev/null) | grep -oF -- "$lit" | wc -l | tr -d ' ')
  mark=''
  if [ "$b" != "$want_base" ] || [ "$n" != "$want_now" ]; then fail=1; mark='FAIL '; fi
  printf '%s%s\t%s\t%s>%s\t%s\t%s\n' "$mark" "$b" "$n" "$want_base" "$want_now" "$file" "$lit"
done < "$S/0134-literals.txt"
exit $fail
```
