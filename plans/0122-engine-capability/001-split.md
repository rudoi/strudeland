# 001: the equality guard, and controls split into capability, params and coerce

## Goal

1. **The guard.** Record the absolute equality guard from an unedited tree, before any edit.
2. **The split.** Move `crates/strudeland-engine/src/controls.rs` into three modules, following
   the proposal's "The three modules":
   - `capability.rs` (`pub mod`);
   - `params.rs` (`pub(crate) mod`);
   - `coerce.rs` (`pub(crate) mod`).

   Every import changes to the real path, and `controls.rs` is deleted with no facade.
3. **The stale docs.** Fix every doc path that stops existing, and AGENTS.md's nonexistent
   `signatures` module.

**Not one line of logic changes.** Code moves between files. The only other edits are these:

- `use` lines and module docs;
- two visibility changes: `is_silence`, and the six filter key consts;
- the `bus_controls` test split in two;
- test helpers copied where both sides call them.

## Prerequisites

- On `llm/andrew/strudeland/0122-engine-capability`.
- `git diff --quiet 385724a -- crates` holds: no engine edit yet.
- `jq`, `shasum` and a working `cargo build --release -p strudeland-engine`.

## Implementation

### Step 1: the equality guard (before any edit)

`S` is the scratchpad (see [000-prompt.md](000-prompt.md)). The guard is in `$S/0122-baseline/`.
A copy of it is in `~/.cache/strudeland-0122/`.

1. **Install the script.** Create `$S/0122-baseline/eq.sh` with the text below and
   `chmod +x` it. A draft of the same script was dry-run while planning, at
   `$S/0122-draft/eq.sh`. It ran against an existing binary and showed:
   - 217 jobs;
   - 20 sounds and 91 targets in the dump;
   - every extra job accepted by `signatures`;
   - `{"s":["-"]}` accepted, with the render error `sound - not found`.
2. **Record the baseline.** Run `"$S/0122-baseline/eq.sh" baseline`. It refuses unless the
   tree's `crates/` equals `385724a`. It builds the release binary, writes the 36 extra jobs and
   records `base/`. It checks that every extra job is accepted and that the dump holds 20 sounds
   and 91 targets, then copies itself to `~/.cache/strudeland-0122/eq.sh`. Expect
   `baseline: 217 jobs, 20 sounds, 91 targets`.
3. **Check the guard.** Run `"$S/0122-baseline/eq.sh" check` once on the unedited tree. It must
   print `equal` and exit 0. A difference here means the guard itself is not deterministic: stop
   and report it.
4. **Recovery** (a new session, or a cleared scratchpad, after edits have started):
   1. copy `~/.cache/strudeland-0122/eq.sh` into the new `$S/0122-baseline/`;
   2. make a WIP commit if the tree is dirty;
   3. run `git switch --detach 385724a`, then `eq.sh baseline`;
   4. run `git switch -` and undo the WIP commit (`git reset --soft HEAD~1`).

   Never use a scratch worktree. Never record a baseline from an edited tree.

```sh
#!/bin/sh
# 0122 equality guard: eq.sh baseline | check
#   baseline  build the engine from an unedited tree (385724a) and record its outputs in base/
#   check     build the engine from the working tree, record into cur/, diff -r against base/
set -eu

B=$(cd "$(dirname "$0")" && pwd)
REPO=/Volumes/rudenet/code/strudeland-worktrees/poptart
REFS="$REPO/crates/strudeland-engine/tests/refs"
BASE_SRC=385724a

target_dir() {
	cargo metadata --no-deps --format-version 1 --manifest-path "$REPO/Cargo.toml" | jq -r .target_directory
}

# build SUFFIX: the release engine binary from the working tree, copied to bin/.
build() {
	mkdir -p "$B/bin"
	(cd "$REPO" && cargo build --release -p strudeland-engine --bin strudeland-engine)
	cp "$(target_dir)/release/strudeland-engine" "$B/bin/strudeland-engine-$1"
}

# extra_jobs: one job per line of values below (every value's hap at time 0, in one pause).
extra_jobs() {
	rm -rf "$B/extra"
	mkdir -p "$B/extra"
	i=0
	while IFS= read -r values; do
		[ -n "$values" ] || continue
		i=$((i + 1))
		jq -n --argjson values "$values" '{
			schema: 2, sampleRate: 44100, frames: 22050, seed: 0, cps: 1,
			channels: { count: 2, multiChannelOrbits: false },
			orbits: [{ orbit: 1, channels: [0, 1] }],
			pauses: [{ suspendSec: 0, haps: [range(0; $values | length) as $j
				| { t: 0, duration: 0.25, cycle: 0, v: $j, event: $j }] }],
			values: $values
		}' >"$B/extra/$(printf 'x%02d' "$i").job.json"
	done <<'EOF'
[{"s":"-"}]
[{"s":"_"}]
[{"s":"~"}]
[{"s":"brown"}]
[{"s":"sine","bus":1,"dry":0},{"s":"bus","n":1}]
[{"s":"bus","n":1}]
[{"s":"crackle"}]
[{"s":"one"}]
[{"s":"pink"}]
[{"s":"pulse"}]
[{"s":"saw"}]
[{"s":"sawtooth"}]
[{"s":"sin"}]
[{"s":"sine"}]
[{"s":"sqr"}]
[{"s":"square"}]
[{"s":"supersaw"}]
[{"s":"tri"}]
[{"s":"triangle"}]
[{"s":"user"}]
[{"s":"white"}]
[{}]
[{"s":["-"]}]
[{"s":["~"]}]
[{"s":["_"]}]
[{"s":["sine"]}]
[{"s":1}]
[{"s":"sine","bank":"x"}]
[{"s":"-","bank":"x"}]
[{"s":"bytebeat"}]
[{"s":"sbd"}]
[{"s":"wt_a"}]
[{"s":"z_sine"}]
[{"s":"zzfx"}]
[{"s":"piano"}]
[{"s":"sine","FX":[{"cutoff":500,"i":1,"ir":"x"}]}]
EOF
}

# record ENGINE OUT: the dump, and every job's signatures and render, into OUT.
record() {
	eng=$1
	out=$2
	rm -rf "$out"
	mkdir -p "$out/sig" "$out/tmp"
	set +e
	"$eng" controls --json >"$out/controls.json" 2>"$out/controls.stderr"
	echo "exit $?" >>"$out/controls.stderr"
	set -e
	: >"$out/render.tsv"
	for job in "$REFS"/*.job.json "$B"/extra/*.job.json; do
		name=$(basename "$job" .job.json)
		wav="$out/tmp/$name.wav"
		set +e
		"$eng" signatures --job "$job" >"$out/sig/$name.txt" 2>&1
		scode=$?
		"$eng" render --job "$job" --out "$wav" >"$out/tmp/stdout" 2>"$out/tmp/stderr"
		rcode=$?
		set -e
		echo "exit $scode" >>"$out/sig/$name.txt"
		if [ -f "$wav" ]; then
			wsum=$(shasum -a 256 "$wav" | cut -d' ' -f1)
		else
			wsum=none
		fi
		esum=$(shasum -a 256 <"$out/tmp/stderr" | cut -d' ' -f1)
		if ! stdout=$(jq -c 'del(.renderMs)' "$out/tmp/stdout" 2>/dev/null); then
			stdout=$(cat "$out/tmp/stdout")
		fi
		printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$rcode" "$wsum" "$esum" "$stdout" >>"$out/render.tsv"
		rm -f "$wav"
	done
	rm -rf "$out/tmp"
}

case "${1:-}" in
baseline)
	if ! git -C "$REPO" diff --quiet "$BASE_SRC" -- crates; then
		echo "eq.sh: crates/ differs from $BASE_SRC; refusing to record a baseline" >&2
		exit 2
	fi
	build baseline
	extra_jobs
	record "$B/bin/strudeland-engine-baseline" "$B/base"
	bad=$(grep -L '^exit 0$' "$B"/base/sig/x*.txt || true)
	if [ -n "$bad" ]; then
		echo "eq.sh: extra jobs the engine rejects: $bad" >&2
		exit 2
	fi
	sounds=$(jq '.sounds | length' "$B/base/controls.json")
	targets=$(jq '.modulatorTargets | length' "$B/base/controls.json")
	if [ "$sounds" != 20 ] || [ "$targets" != 91 ]; then
		echo "eq.sh: the dump has $sounds sounds and $targets targets (want 20 and 91)" >&2
		exit 2
	fi
	mkdir -p "$HOME/.cache/strudeland-0122"
	cp "$0" "$HOME/.cache/strudeland-0122/eq.sh"
	echo "baseline: $(wc -l <"$B/base/render.tsv" | tr -d ' ') jobs, $sounds sounds, $targets targets"
	;;
check)
	if [ ! -d "$B/base" ]; then
		echo "eq.sh: no baseline at $B/base (see 001-split.md step 1)" >&2
		exit 2
	fi
	build current
	record "$B/bin/strudeland-engine-current" "$B/cur"
	if diff -r "$B/base" "$B/cur" >"$B/diff.txt"; then
		echo "equal: $(wc -l <"$B/cur/render.tsv" | tr -d ' ') jobs, the dump and every signatures output"
	else
		echo "DIFFERENT ($(wc -l <"$B/diff.txt" | tr -d ' ') diff lines, $B/diff.txt):"
		head -40 "$B/diff.txt"
		exit 1
	fi
	;;
*)
	echo "usage: eq.sh baseline | check" >&2
	exit 2
	;;
esac
```

`extra/` is written once, at baseline, and both records read the same jobs from it, so it needs
no diff of its own. Nothing in `base/` or `cur/` holds a timestamp or a path, because `renderMs`
is stripped and no WAV is kept.

### Step 2: the three files

Line numbers are `controls.rs` at `385724a`. Move code verbatim, in its current order within
each file.

1. **`src/capability.rs`.**
   - **Module doc:** "What the engine plays: the control allowlist (audited in `controls.md` in
     the crate root; a test keeps the two in step), the sounds, the FX-chain and modulator keys,
     the only unsupported detection (`unsupported_signatures`, behind the binary's `signatures`
     subcommand, the render refusal and the live warning) and the `controls --json` dump."
   - **Code:**
     - `ALLOWED_KEYS`, `ALLOWED_SOUNDS` and `SILENCE` (`:12-444`);
     - `is_silence` (`:1659-1661`), now `pub(crate)`;
     - `CHAIN_KEYS`, `CHAIN_UNSUPPORTED_KEYS`, `MODULATOR_KEYS` and `MODULATOR_TARGETS`
       (`:446-656`);
     - everything from `// ---- unsupported detection` (`:1685`) through `controls_json`
       (`:1859`).
   - **Imports:**
     - `std::collections::BTreeSet`;
     - `crate::job::{EngineJob, Value, ValueMap, bar_of}`;
     - `crate::coerce::{js_to_string, js_truthy}`;
     - `crate::params::sound_of`.

     Keep only what the compiler needs.
2. **`src/coerce.rs`.**
   - **Module doc:** "superdough's JS coercions as the engine needs them: `Number(x)`,
     `String(x)`, truthiness, `Math.max`/`min` and ECMAScript `StringToNumber`. Unlike `js`,
     these have no JS vectors; their unit tests pin them."
   - **Code:** `// ---- JS coercions` (`:665-835`), with its private helpers.
   - **Imports:** `crate::job::Value` and `crate::js::number_to_string`, as the code needs.
3. **`src/params.rs`.**
   - **Module doc:** "The controls one voice reads (`VoiceParams`, `EntryParams`), typed per
     superdough read site, with the defaults that sit outside a node, and superdough's mutation
     of a hap value (`prepare_value`)."
   - **Code:**
     - `ControlError` (`:658-663`);
     - `:837-1683` without `is_silence`;
     - the six filter key consts `LOWPASS_KEYS`, `HIGHPASS_KEYS`, `BANDPASS_KEYS`,
       `LOWPASS_LFO_KEYS`, `HIGHPASS_LFO_KEYS` and `BANDPASS_LFO_KEYS`, made `pub(crate)`. So are
       their `FilterKeys`/`FilterLfoKeys` aliases, if the compiler asks.
   - **Imports:**
     - `crate::chain::FxKey`;
     - `crate::compressor::CompressorParams`;
     - `crate::job::{Value, ValueMap}`;
     - `crate::coerce::*` (as used);
     - `crate::capability::is_silence`.
4. **`src/lib.rs`.** Replace `pub mod controls;` with `pub mod capability;`,
   `pub(crate) mod coerce;` and `pub(crate) mod params;`, in alphabetical order with the
   others. Change the re-export to `pub use capability::{ALLOWED_KEYS, ALLOWED_SOUNDS, controls_json};`.
5. **Delete `src/controls.rs`.**

### Step 3: the tests

Each file gets its own `#[cfg(test)] mod tests` with `use super::*;` and the imports it needs.

- **`capability`:**
  - `a_non_scalar_distorttype_is_unsupported`;
  - `job_with`, `unsupported_signature_rules` and `signatures_are_distinct_with_first_bar`;
  - `allowlists_are_sorted_and_unique` and `chain_and_modulator_tables`. The latter imports the
    six `params::*_KEYS` consts and keeps every assertion, the length pin included until 002;
  - `AuditRow`, `is_task_ref`, `audit_rows` and `refused_sound_rows`. `include_str!("../controls.md")`
    is unchanged, because the path is relative to `src/`;
  - `refused_sounds_are_not_allowed`, `only_sample_and_user_dsp_keys_stay_unsupported` and
    `the_audit_table_matches_the_allowlist`. That last one imports `params::{FM_MODULATORS,
    fm_control_name, fm_suffix}`;
  - `table_job`, `shared_signature_table` and `controls_json_shape`;
  - **`bus_signatures`**, the second half of today's `bus_controls`: from the comment "The bus
    sound and its keys are allowed" to the end, unchanged.
- **`coerce`:** `js_number_coercion` and `value_coercions`.
- **`params`:** every other test, with today's names and bodies. That covers
  `entry_params_defaults_and_the_main_entry`, `frequency_matches_the_js_vectors`, `frequency_rules`,
  the FM tests, `superdough_mutation` and `tremolo_filter_lfo_and_pulse_width_controls`, among
  others. It also keeps **`bus_controls`**, the `params(...)` half of today's test.
  `fm_control_names_follow_the_matrix_rule` imports `crate::capability::ALLOWED_KEYS`.
- **Helpers.** `map`, `params` and `close` go with the tests that call them. Copy one only when
  both modules call it.

**Check that every test survived:** the number of `#[test]` functions across the three files
equals today's count in `controls.rs` plus one, for the `bus_controls` split.

```sh
git show 385724a:crates/strudeland-engine/src/controls.rs | grep -c '#\[test\]'
cat crates/strudeland-engine/src/{capability,params,coerce}.rs | grep -c '#\[test\]'
```

### Step 4: the imports

Change every `crate::controls::` and `strudeland_engine::controls::` site to the module that now
holds the item. The compiler finds them. The sites are:

- **Coercions** (`crate::coerce::`):
  - `adsr.rs:5`, `delay.rs:27` and `voice.rs:43` (`js_max`, `js_min`);
  - `job.rs:196` (`js_to_number`);
  - `periodic.rs:18` and `:437`;
  - `reverb.rs:62` and `orbit.rs:59`;
  - `vowel.rs:145`;
  - `voice.rs:1102` (`js_truthy`);
  - the coercion parts of `chain.rs:33`, `voice.rs:39`, `modulate.rs:20`, `stems.rs:6` and
    `bus.rs:12`.
- **Params** (`crate::params::`):
  - the rest of `chain.rs:33`, plus `chain.rs:1498` and `:1523`;
  - the rest of `voice.rs:39`, plus `voice.rs:2266`;
  - the rest of `modulate.rs:20`, plus `modulate.rs:621` and `:1254`;
  - `render_core.rs:7`;
  - `stems.rs:6` and `bus.rs:12` (`prepare_value`);
  - `js.rs:343`.
- **Capability** (`crate::capability::`): `render.rs:10` and `live.rs:16`.
- **The public path** (`strudeland_engine::capability::`):
  - `bin/strudeland-engine.rs:20`;
  - `tests/fixtures.rs:19` and `tests/micro.rs:21`;
  - `tests/bin.rs:432` and `tests/never_panics.rs:1261` (`CHAIN_KEYS`);
  - `crates/strudeland-render/src/plan.rs:7`.

`tests/never_panics.rs:6` and `tests/bin.rs:421,428,443` use the root re-exports and do not
change.

When it is done, `git grep -nE '\bcontrols::' -- crates src-tauri` prints nothing.

### Step 5: docs and comments

- **AGENTS.md:212-213:** replace "`controls` (the allowlist, audited in `controls.md`) and
  `signatures` (the binary's unsupported detection, called by `scripts/render/rust-engine.ts`)"
  with the proposal's text (Docs, first bullet), which names `capability`, `params` and `coerce`.
  Keep the line's other module entries and AGENTS.md:25 as they are.
- **`crates/strudeland-engine/README.md`:**
  - `:487`: `src/controls.rs` becomes `src/capability.rs`;
  - `:507`: `controls::signature_of_with_bus` becomes `capability::signature_of_with_bus`.
- **`crates/strudeland-engine/controls.md:41`:** `src/controls.rs` becomes `src/capability.rs`.
  Line 42's `ALLOWED_SOUNDS` stays until 002.
- **`src/js.rs:320`:** `controls::prepare_value` becomes `params::prepare_value`.
- **`tests/bin.rs:41`:** "`table_job` in `src/controls.rs`" becomes "in `src/capability.rs`".
- **Leave alone:** `src/lib/render/schedule.ts:166` (see the proposal's Scope).

When it is done, `git grep -n 'controls\.rs' -- ':!plans'` prints only
`src/lib/render/schedule.ts:166`.

## Validation

- **Run the gates** in [000-prompt.md](000-prompt.md) with `N=001`. Every log ends `exit 0`, and
  `0122-001-eq.log` says `equal`.
- **The move diff.** It shows only the `use` lines, the module docs, the two visibility changes,
  the test module headers, the `bus_controls` split and any copied helpers:

  ```sh
  git show 385724a:crates/strudeland-engine/src/controls.rs | grep -v '^use ' | sort >"$S/0122-001-old.txt"
  cat crates/strudeland-engine/src/{capability,params,coerce}.rs | grep -v '^use ' | sort >"$S/0122-001-new.txt"
  diff "$S/0122-001-old.txt" "$S/0122-001-new.txt" >"$S/0122-001-move.diff"
  ```

  Read `0122-001-move.diff`. Expect the continuation lines of any multi-line `use` that rustfmt
  wrote, and the indented `use` lines of the test modules. Any other line is a logic change:
  undo it.
- **The test count** matches (step 3).
- **Nothing outside the engine moved.** `git diff --stat main -- src scripts tests vendor
  crates/strudeland-host crates/strudeland-live` is empty, and
  `git diff --stat main -- crates/strudeland-render` shows only `src/plan.rs`.
- **The grep checks** in steps 4 and 5 hold.

## Commit message

```
refactor(engine): split controls into capability, params and coerce

The 3252-line controls module splits by job. capability holds what the
engine plays: the allowlists audited in controls.md, the unsupported
detection behind the signatures subcommand, the render refusal and the
live warning, and the controls --json dump. params holds the typed
controls a voice reads and superdough's mutation of a hap value. coerce
holds the JS coercions. Callers import the real paths (strudeland-render
takes capability); there is no re-export facade.

No logic changes: the dump, the signatures output of every reference
job and every render are byte-identical. AGENTS.md now names the
modules that exist.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
