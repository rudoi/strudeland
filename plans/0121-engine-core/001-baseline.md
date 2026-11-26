# Stage 001: the bit-guard baseline and local reverbs in `live_equals_offline`

## Goal

- An absolute A/B guard, built from the tree **before any edit**, that every later stage runs
  and that must show zero differences.
- `live_equals_offline` covers local (FX) reverbs, on today's code.

This stage changes no engine source. Its commit adds three tests.

## Prerequisites

- The branch is `llm/andrew/strudeland/0121-engine-core`, at `3cd77ba` plus the plan files.
- `git status --short crates/` prints nothing. **If it prints anything, stop:** the baseline
  must come from unedited sources.
- `jq` and `shasum` are on the path.

Throughout:

- `S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`
- `B=$S/0121-baseline`
- `REPO=/Volumes/rudenet/code/strudeland-worktrees/poptart`

Nothing under `$S` is committed.

## Implementation

### 1. The baseline engine binary (before any edit)

```sh
cd "$REPO"
cargo build --release -p strudeland-engine --bin strudeland-engine
T=$(cargo metadata --no-deps --format-version 1 | jq -r .target_directory)
mkdir -p "$B/bin"
cp "$T/release/strudeland-engine" "$B/bin/strudeland-engine-baseline"
git rev-parse HEAD > "$B/BASE"
```

### 2. The digest crate (scratchpad only)

The offline binary renders mixes only. The digest crate reaches the other paths through the
public API: the live worker (`Convolver::new_live`) with results on time, late, out of order and
after a disconnect, the synchronous live settle (`sync_ir`, and no worker attached), `stop`,
`set_seed` and stems.

- `$B/digest/Cargo.toml`:

  ```toml
  [package]
  name = "engine-digest"
  version = "0.0.0"
  edition = "2024"
  publish = false

  [workspace]

  [dependencies]
  strudeland-engine = { path = "/Volumes/rudenet/code/strudeland-worktrees/poptart/crates/strudeland-engine" }

  [profile.release]
  codegen-units = 1
  lto = true
  opt-level = 3
  ```

  The release profile keeps the default `panic = "unwind"`, so a variant that panics is caught
  and recorded rather than ending the run.
- Copy `$REPO/Cargo.lock` to `$B/digest/Cargo.lock`, so the engine's dependencies resolve to the
  workspace's versions. Cargo prunes the unused entries on the first build, so the lock changes
  and `--locked` would refuse it. Build with `--offline` instead: it resolves from the copied lock
  and the local registry cache and never picks a newer version.
- `$B/digest/src/main.rs` takes the refs directory as its one argument. For each `*.job.json` in
  sorted order, it prints one line per variant: `variant<TAB>job<TAB>digest`. The digest is a
  16-hex-digit FNV-1a 64 over:
  - every output sample's `to_bits()` as little-endian bytes;
  - then each event's or error's `{:?}` text;
  - then the counters' `{:?}`.

  Each variant runs inside `std::panic::catch_unwind(AssertUnwindSafe(…))` (`LiveEngine` is not
  `UnwindSafe`). A variant that panics prints `panic` as its digest, and the next variant runs.
  The panic message goes to stderr, which `ab.sh` does not compare.

  **The live replay.** Every `live-*` variant builds its engine, adds every value (value `k` gets
  `job.orbits.get(k)`), then renders quantum by quantum through `round_up_128(job.frames)`. Before
  the quantum that starts at frame `f`, in this order:

  1. the variant's worker step, if it has one (the table says what it delivers at `f`);
  2. the variant's action, when `f == mid`, where
     `mid = round_up_128(job.frames) / 2 / QUANTUM as u64 * QUANTUM as u64`, the quantum boundary
     at or just below half the render. It does not depend on the pauses;
  3. the haps of every pause whose `pause_frame` is `<= f` and not yet scheduled, in pause order,
     with the `LiveHap` fields `live_equals_offline.rs`'s `live_render` uses.

  Copy `pause_frame`, `round_up_128` and the `LiveHap` fields from `live_equals_offline.rs`. With
  no action and no worker step, this is exactly `live_render`'s order. After the last quantum the
  digest takes `take_events()` and `counters()`.

  The variants:

  | Variant | Applies to | Engine and worker | Action at `mid` |
  |---|---|---|---|
  | `live-worker` | 2-channel jobs with `orbits.len() <= values.len()` | `LiveEngine::new(job.seed)` with `attach_ir_worker(req_tx, built_rx)`. Worker step: every request in `req_rx.try_iter()` is built with `build_ir` and sent on `built_tx`, so a request sent in one quantum lands at the next quantum's poll. | none |
  | `live-sync` | the same jobs | `LiveEngine::with_options(job.seed, LiveOptions { sync_ir: false })` with no worker attached: the synchronous settle that `render_quantum` picks when no link exists. | none |
  | `live-stop` | the same jobs | `LiveOptions { sync_ir: true }`, no worker. | `e.stop()` |
  | `live-seed` | the same jobs | `LiveOptions { sync_ir: true }`, no worker. | `e.set_seed(job.seed.wrapping_add(1))`, then `e.set_seed(job.seed.wrapping_add(1))` again (the same seed: a no-op) |
  | `live-worker-lag` | the same jobs | As `live-worker`, but the worker step at `f` drains `req_rx.try_iter()` into a batch tagged with `f`, and delivers each batch three quanta later (at `f + 3 * QUANTUM`), its results sent in reverse request order. Batches still held at the end are never delivered. | none |
  | `live-worker-stop` | the same jobs | As `live-worker-lag`. | `e.stop()`. With the lag, every request sent in the three quanta before `mid` is still undelivered. |
  | `live-dead` | the same jobs | As `live-worker`, but after the worker step at `f = QUANTUM` (the requests of the first quantum built and sent) it drops `req_rx`. `built_tx` stays alive. Every later send fails, so the engine builds that miss in place and drops its link. | none |
  | `stems-orbit`, `stems-voice`, `stems-instrument` | every job | `render_with_stems(&job, &StemLayout::for_jobs(std::slice::from_ref(&job), grouping))`. On `Err`, the digest of the error's `Display` text. | none |

  Use only the public API: `strudeland_engine::{EngineJob, QUANTUM, render_with_stems, StemGrouping, StemLayout}`
  and `strudeland_engine::live::{LiveEngine, LiveOptions, LiveHap, IrRequest, IrBuilt, build_ir}`,
  with `std::sync::mpsc` channels. Every variant above is reachable that way: `IrRequest` and
  `IrBuilt` are opaque, but the digest only moves them, reorders them and passes them to
  `build_ir`. For a job outside a variant's "applies to" column, print `skip` as the digest.

  What each worker variant covers after stage 003: `live-worker` the in-order worker path,
  `live-worker-lag` the newest-request-wins landing with results late and out of order,
  `live-worker-stop` results that arrive for voices `stop` dropped (`land_ir` skips them), and
  `live-dead` the in-place fallback and the switch to `SyncSettler` on the next quantum.
- Build it and keep the baseline binary:

  ```sh
  CARGO_TARGET_DIR="$B/digest/target" cargo build --release --offline --manifest-path "$B/digest/Cargo.toml"
  cp "$B/digest/target/release/engine-digest" "$B/bin/engine-digest-baseline"
  ```

### 3. The A/B script, `$B/ab.sh`

`ab.sh rebuild-baseline`, `ab.sh baseline` or `ab.sh check`. POSIX `sh`, with `set -eu`. It
sets `B=$(cd "$(dirname "$0")" && pwd)` and `REPO` as above, so a copy restored into another
session's scratchpad works unchanged.

- **`rebuild-baseline`:**
  - refuses (exit 2) unless `git -C "$REPO" diff --quiet 3cd77ba -- 'crates/*/src/*'` succeeds,
    that is, unless the tree's engine sources are those of `3cd77ba` (HEAD is the 001 commit or
    earlier, with no source edit in the working tree);
  - builds the engine binary and the digest crate as steps 1 and 2 do, and copies them to
    `$B/bin/strudeland-engine-baseline` and `$B/bin/engine-digest-baseline`;
  - writes `git -C "$REPO" rev-parse HEAD` to `$B/BASE`.
- **`baseline`:**
  - refuses (exit 2) unless `git -C "$REPO" diff --quiet HEAD -- crates` succeeds;
  - uses the `*-baseline` binaries and writes `$B/base/`.
- **`check`:**
  - rebuilds both from the working tree:
    - `cargo build --release -p strudeland-engine --bin strudeland-engine`, then copies the
      result to `$B/bin/strudeland-engine-current`;
    - the digest crate as above, copied to `$B/bin/engine-digest-current`;
  - writes `$B/current/`;
  - diffs it against `$B/base/`.
- **For `baseline` and `check`,** with `out` as its directory: `rm -rf "$out"`, then for every
  `"$REPO"/crates/strudeland-engine/tests/refs/*.job.json` in sorted order:
  - run `"$eng" render --job "$job" --out "$out/wav/$name.wav"`, with stdout and stderr captured
    to files and the exit code kept (`set +e` around the call);
  - append one line to `$out/manifest.tsv`, tab-separated:
    1. `name`;
    2. the exit code;
    3. the WAV's `shasum -a 256` (`none` when no file was written);
    4. the stderr's `shasum -a 256`;
    5. stdout through `jq -c 'del(.renderMs)'` (the raw text if jq fails);
  - save the stderr text as `$out/stderr/$name.txt`;
  - delete the WAV (181 renders take hundreds of MB).
- **Then** `"$dig" "$REPO/crates/strudeland-engine/tests/refs" > "$out/digest.txt"`.
- **In `check` mode:**
  - `diff "$B/base/manifest.tsv" "$B/current/manifest.tsv"` and
    `diff "$B/base/digest.txt" "$B/current/digest.txt"`;
  - print the number of differing lines;
  - exit 0 when both are equal, else 1;
  - print `0 differences` on success.

### 4. Record and self-check the baseline

```sh
sh "$B/ab.sh" baseline
sh "$B/ab.sh" check
```

**The immediate `check` must report 0 differences.** It proves that the renders and the digest
are reproducible run to run. If it reports a difference, stop and report: a guard that varies
by itself cannot be used. Then confirm:

- `wc -l "$B/base/manifest.tsv"` is 181;
- `digest.txt` has 10 lines per job, with `skip` only where the table says;
- `grep -c 'panic$' "$B/base/digest.txt"` is 0. A variant that panics on today's code is a
  finding: stop and report the job and the variant.

### 5. Local reverbs in `live_equals_offline`

In `crates/strudeland-engine/tests/live_equals_offline.rs`, after `fx_room`, add:

```rust
#[test]
fn fx_chain_room() {
    check("fx-chain-room.job.json");
}

#[test]
fn fx_chain_mix_0() {
    check("fixture-fx-chain--mix-0.job.json");
}

#[test]
fn fx_chain_mix_1() {
    check("fixture-fx-chain--mix-1.job.json");
}
```

Also update the module doc to say that these jobs include local (FX entry) reverbs. All three
are stereo, with `orbits.len() <= values.len()`:

- `fx-chain-room`: 1 orbit, 1 value, which has a local room;
- the fixtures: 6 orbits, 15 values, 2 of them with local rooms.

Run `cargo test -p strudeland-engine --test live_equals_offline`. **All of them must pass on
today's code.** If one fails, stop and report the file, the first differing frame and the
errors. That is a live/offline divergence that exists today, outside this refactor. Do not touch
engine source to fix it.

### 6. Keep a copy, and recover

The scratchpad is per session and can be cleared, so keep the guard's sources outside it. Once
step 4 has passed:

```sh
mkdir -p ~/.cache/strudeland-0121
cp "$B/ab.sh" ~/.cache/strudeland-0121/
rsync -a --exclude target "$B/digest/" ~/.cache/strudeland-0121/digest/
```

This copies `ab.sh` and the digest crate's `Cargo.toml`, `Cargo.lock` and `src/`, not
`target/`.

**If `$B/base/` is lost** later (a new session, a reboot), rebuild it from the 001 commit, which
changes no engine source:

1. Set `S` to the current session's scratchpad and `B=$S/0121-baseline`. Restore:
   `mkdir -p "$B" && cp ~/.cache/strudeland-0121/ab.sh "$B/" && rsync -a ~/.cache/strudeland-0121/digest/ "$B/digest/"`.
2. If the working tree is dirty, make a WIP commit.
3. `git switch --detach <001-sha>`. Never use a scratch worktree.
4. `sh "$B/ab.sh" rebuild-baseline`, then `sh "$B/ab.sh" baseline`.
5. `git switch -`, then undo the WIP commit, if there is one, with `git reset HEAD~1`.
6. `sh "$B/ab.sh" check`. On a committed stage it must report 0 differences, as that stage's
   own log did.

## Tests

- **Added:** `fx_chain_room`, `fx_chain_mix_0` and `fx_chain_mix_1`.
- **Nothing is deleted or moved.**

## Validation

- The gates in [000-prompt.md](000-prompt.md), with `N=001`. All of them exit 0.
- The A/B log reports 0 differences: the commit adds only tests.
- `git diff --stat HEAD -- crates/strudeland-engine/src crates/strudeland-live` is empty.

## Commit message

```
test(engine): replay local-reverb fixtures through the live engine

live_equals_offline covered no job with an FX-entry reverb. Add
fx-chain-room and both fx-chain fixture windows, so the voices' local
reverbs are checked bit for bit against the offline render as well.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
