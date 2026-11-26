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
public API: the live worker (`Convolver::new_live`), `stop`, `set_seed` and stems.

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
  panic = "abort"
  ```

- Copy `$REPO/Cargo.lock` to `$B/digest/Cargo.lock`, so the engine's dependencies resolve to the
  workspace's versions. Cargo prunes the unused entries.
- `$B/digest/src/main.rs` takes the refs directory as its one argument. For each `*.job.json` in
  sorted order, it prints one line per variant: `variant<TAB>job<TAB>digest`. The digest is a
  16-hex-digit FNV-1a 64 over:
  - every output sample's `to_bits()` as little-endian bytes;
  - then each event's or error's `{:?}` text;
  - then the counters' `{:?}`.

  The variants:

  | Variant | Applies to | What it renders |
  |---|---|---|
  | `live-worker` | 2-channel jobs with `orbits.len() <= values.len()` | `LiveEngine::new(job.seed)` with `attach_ir_worker(req_tx, built_rx)`. After **every** `render_quantum`, every request in `req_rx.try_iter()` is built with `build_ir` and sent on `built_tx`, so it lands at the next quantum's poll. The haps are replayed pause by pause as `tests/live_equals_offline.rs`'s `live_render` does (value `k` gets `job.orbits.get(k)`), through `round_up_128(job.frames)`. |
  | `live-stop` | the same jobs | The same replay with `LiveOptions { sync_ir: true }`, and `e.stop()` called just before the haps of pause `pauses.len() / 2` are scheduled. |
  | `live-seed` | the same jobs | The same replay with `sync_ir: true`. Just before the haps of pause `pauses.len() / 2`: `e.set_seed(job.seed.wrapping_add(1))`, then the same seed again. |
  | `stems-orbit`, `stems-voice`, `stems-instrument` | every job | `render_with_stems(&job, &StemLayout::for_jobs(std::slice::from_ref(&job), grouping))`. On `Err`, the digest of the error's `Display` text. |

  Use only the public API: `strudeland_engine::{EngineJob, QUANTUM, render_with_stems, StemGrouping, StemLayout}`
  and `strudeland_engine::live::{LiveEngine, LiveOptions, LiveHap, build_ir}`. Copy the pause
  replay from `live_equals_offline.rs` (`pause_frame`, `render_until`, the `LiveHap` fields).
  For a job outside a variant's "applies to" column, print `skip` as the digest.
- Build it and keep the baseline binary:

  ```sh
  CARGO_TARGET_DIR="$B/digest/target" cargo build --release --manifest-path "$B/digest/Cargo.toml"
  cp "$B/digest/target/release/engine-digest" "$B/bin/engine-digest-baseline"
  ```

### 3. The A/B script, `$B/ab.sh`

`ab.sh baseline` or `ab.sh check`. POSIX `sh`, with `set -eu`.

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
- **For each mode,** with `out` as its directory: `rm -rf "$out"`, then for every
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
- `digest.txt` has 6 lines per job, with `skip` only where the table says.

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
