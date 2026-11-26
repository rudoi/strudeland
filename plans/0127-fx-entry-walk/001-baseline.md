# Stage 001: the A/B guard, before any engine edit

## Goal

Build the absolute guard that every later stage must pass with 0 differences, from a tree whose
engine sources are `416caa6`'s. Record it twice to prove it is reproducible, then snapshot it
outside the scratchpad.

This stage edits no file in the repository and commits nothing.

## Files

- `$S/0127-baseline/ab.sh`: the guard, copied from the draft below.
- `$S/0127-baseline/digest/`: the #121 digest crate (`Cargo.toml`, `src/main.rs`), copied from
  `~/.cache/strudeland-0121/digest/`. `ab.sh baseline` copies the workspace `Cargo.lock` in.
- `$S/0127-baseline/{bin,extra,base,cur}/`, written by `ab.sh`.
- `~/.cache/strudeland-0127/{ab.sh,digest/,base/,SOURCE_SHA}`, the snapshot `ab.sh baseline` writes.

Throughout:

- `S` is your session's scratchpad;
- `B=$S/0127-baseline`;
- `REPO=/Volumes/rudenet/code/strudeland-worktrees/poptart`.

## Prerequisites

- The branch is `llm/andrew/strudeland/0127-fx-entry-walk`, at `416caa6` plus the plan files.
- `git -C "$REPO" diff --quiet 416caa6 -- 'crates/*/src/*' crates/strudeland-engine/tests/refs`
  exits 0. **If it does not, stop:** the baseline must come from unedited sources.
- `jq`, `shasum` and `rsync` are on the path.
- `~/.cache/strudeland-0121/digest/` exists, with `Cargo.toml` and `src/main.rs`. Its
  `main.rs` is the #121 digest: 7 live variants and 3 stems variants per job, `skip` for a job
  the live variants do not apply to, `panic` for a variant that panics. The 0121 archived
  `001-baseline.md` (in git history at `25a3341c:plans/0121-engine-core/001-baseline.md`)
  describes it. **Do not edit it:** the baseline and the check build the same source.

## Steps

1. **Set up the directory.**

   ```sh
   mkdir -p "$B"
   cp "$S/0127-draft/ab.sh" "$B/ab.sh"   # or write it from the quote below
   rsync -a --exclude target ~/.cache/strudeland-0121/digest/ "$B/digest/"
   ```

   A later session has no `$S/0127-draft/`. Write `$B/ab.sh` from the quote below,
   byte for byte. The script finds its own directory, so no path in it names a session.
2. **Record the baseline.**

   ```sh
   sh "$B/ab.sh" baseline > "$S/0127-001-baseline.log" 2>&1; echo "exit $?" >> "$S/0127-001-baseline.log"
   ```

   It must end `exit 0`, after a line that reads
   `baseline: 276 jobs, 2760 digest lines, 20 sounds, 91 targets`. That is 181 refs jobs and 95
   extra jobs, with 10 digest lines per job. Any of its refusals (exit 2) is a STOP: report the
   message.
3. **Record again, and compare.**

   ```sh
   sh "$B/ab.sh" check > "$S/0127-001-ab.log" 2>&1; echo "exit $?" >> "$S/0127-001-ab.log"
   ```

   It must end `exit 0` after a line starting `0 differences`. That proves the renders, the
   signatures, the dump and the digest are reproducible run to run. `check` refuses (exit 2)
   when `$B/digest/src/main.rs` differs from the snapshot's `~/.cache/strudeland-0127/digest/`
   copy, or the snapshot has none: the check must build the baseline's digest. A difference here is a
   STOP: a guard that varies by itself cannot be used. Report the first lines of `$B/diff.txt`.
4. **Check the snapshot.** `~/.cache/strudeland-0127/` holds `ab.sh`, `digest/` (no `target/`),
   `base/` and `SOURCE_SHA` (the HEAD sha; not `BASE`, which a case-insensitive volume confuses with `base/`). `diff -r "$B/base" ~/.cache/strudeland-0127/base` prints
   nothing.
5. **Write the stage report.** Include:
   - the baseline line;
   - the `y01`-`y07` "Available targets" messages, copied from `$B/base/render.tsv` (the
     stdout column);
   - the `y08`-`y32` and `y46`-`y50` error messages, one per job;
   - the `skip` count in the digests (`grep -c 'skip$' "$B"/base/digest-*.txt`).

## The extra jobs

Every extra job puts each value's hap at time 0 in one pause: 44100 Hz, 22050 frames, seed 0,
cps 1, duration 0.25, one stereo orbit. That is #122's shape.

- **`x01`-`x45`** are #122's `eq.sh` jobs, unchanged:
  - the 20 sounds, `bus` twice (with a writer, and alone);
  - the silence and bank edge cases;
  - the refused sounds;
  - an FX entry with `i`/`ir`;
  - an `lfo` on a noise source's `note`.
- **`y01`-`y07`: an `lfo` at a missing node.** Each logs "Available targets", which shows the
  registration order:
  - `y01`: an oscillator with a filter and pan;
  - `y02`: an oscillator with vib and FM (vib before the FM);
  - `y03`: supersaw with vib and FM (vib after the FM);
  - `y04`: pulse with `pwsweep`, vib and FM (`pw_lfo` first);
  - `y05`: a chain entry;
  - `y06`: the main sends and djf;
  - `y07`: a chain entry with every key, its local delay and room included.
- **`y08`-`y23`: one throw site each.** In order:
  - the filter Q, the gain before pan, the vowel before pan, the tremolo depth, the pan;
  - a chain entry's throw before the main entry's;
  - `postgain`, the filter LFO;
  - `coarse`, `shapevol`, `distortvol`;
  - the compressor, the phaser;
  - the local delay, the local reverb;
  - the amp envelope's release ramp (`release: Infinity`). Its `FXrelease: 0.5` is not above
    the release, so no Release stage is built and this job never reaches the FXrelease gain;
    `y46` and `y47` do.
- **`y24`-`y32`: the throw ladder.** The first job arms every site. Each next job drops the
  first armed site, so each site in turn is the first to throw.
- **`y33`-`y37`: the FM matrix.**
  - `y33`: a noise target, with `fmi` and `fmh` modulators;
  - `y34`: an `lfo` on the noise modulator's `fmi3`/`fmh3` (refused as `lfo:fmh3`);
  - `y35`-`y37`: the FM throws (an oscillator, and a supersaw after its vibrato).
- **`y38`-`y40` and `y45`: filters.**
  - `y38`: three ladders with LFOs;
  - `y39`: a hap-level 24 dB filter with an entry-level ladder;
  - `y40`: a chain entry with every key, under a main filter, `FXrelease` and an `lfo` with no
    control data;
  - `y45`: 24 dB lowpass and highpass with their LFOs, and a chain entry's bandpass LFO.
- **`y41`-`y44`: the pulse and the worklet synths.**
  - `y41`, `y42`: the pulse with a NaN `pwsweep`, and with a NaN `pwrate`;
  - `y43`, `y44`: the worklet synths with vib and FM under an `lfo` on `vib` and on `vibmod`.
- **`y46`-`y50`: the order between sites, by text.** Each arms two sites whose texts differ, so
  the first thrower shows:
  - `y46`: `FXrelease: Infinity` and `postgain: "x"`. The Release stage's ramp to
    `endWithRelease = Infinity` throws the `linearRampToValueAtTime` text before post's `gain`;
  - `y47`: a chain entry's `pan: "x"` and `FXrelease: Infinity`. The entry's `pan` throws before
    the Release stage;
  - `y48`: `fmi13: 1` onto a white-noise modulator 3 with `fmh3: "x"`. The target side's
    `idl_float(modfreq, "gain")` throws `gain`, before the noise skip;
  - `y49`: the pulse with `pwsweep: "x"` and `velocity: "x"`. `pw_lfo` throws the AudioParam
    `value` text before the entry gain's `gain`;
  - `y50`: a chain entry with `room: 1e300` and `delaytime: 1e300`. The local delay throws the
    AudioParam `value` text before the local reverb's `gain` (`y22`'s text).

The planning dry run (release binary at `416caa6`) rendered all 50 `y` jobs, with `signatures`
exit 0 on each, and the #121 digest ran over `y46`-`y50` without a panic. It gave:

- **`y02`:** `Available targets: source, vib, vib_gain, fm_1, fm_1_gain, fm_2, fm_2_gain, gain, post`.
- **`y03`:** `… source, fm_1, fm_1_gain, fm_2, fm_2_gain, vib, vib_gain, gain, post`.
- **`y04`:** `… source, pw_lfo, fm_1, fm_1_gain, fm_3, fm_3_gain, vib, vib_gain, gain, post`.
- **`y07`:** its 23 keys, from `transient` to `room_mix`.
- **The ladder `y24`-`y32`:** `gain`, `Q`, `unknown vowel zz`, `value is not a finite`, the
  AudioParam `gain` text, the AudioParam `value` text, `pan`, the AudioParam `value` text,
  `gain`.
- **`y34`:** `s=sine keys=lfo:fmh3`.
- **`y23`:** the `linearRampToValueAtTime` text, from the amp envelope.
- **`y46`-`y50`:** the `linearRampToValueAtTime` text, `pan`, `gain`, the AudioParam `value`
  text, the AudioParam `value` text.

## The script

The draft is in the planning scratchpad (`$S/0127-draft/ab.sh`). Copied here in full:

```sh
#!/bin/sh
# The 0127 A/B guard: ab.sh baseline | check | restore
#   baseline  build the engine binary and the digest crate from an unedited tree (416caa6's
#             crates/*/src) and record every output into base/
#   check     build both from the working tree, record into cur/, diff -r against base/
#   restore   copy digest/ (no target/) and base/ back from ~/.cache/strudeland-0127/ (copy
#             ab.sh itself from there first)
# Merges #121's ab.sh (refs renders, the live and stems digest) and #122's eq.sh (the
# controls --json dump, signatures per job, the x01-x45 jobs) and adds the y01-y50 jobs.
set -eu

B=$(cd "$(dirname "$0")" && pwd)
REPO=/Volumes/rudenet/code/strudeland-worktrees/poptart
REFS="$REPO/crates/strudeland-engine/tests/refs"
BASE_SRC=416caa6
CACHE="$HOME/.cache/strudeland-0127"

target_dir() {
	cargo metadata --no-deps --format-version 1 --manifest-path "$REPO/Cargo.toml" | jq -r .target_directory
}

# build SUFFIX: the release engine binary and the digest crate from the working tree, into bin/.
build() {
	mkdir -p "$B/bin"
	(cd "$REPO" && cargo build --release -p strudeland-engine --bin strudeland-engine)
	cp "$(target_dir)/release/strudeland-engine" "$B/bin/strudeland-engine-$1"
	CARGO_TARGET_DIR="$B/digest/target" cargo build --release --offline --manifest-path "$B/digest/Cargo.toml"
	cp "$B/digest/target/release/engine-digest" "$B/bin/engine-digest-$1"
}

# write_jobs PREFIX: one job per non-empty stdin line of values (every value's hap at time 0, in
# one pause, 44100 Hz, 22050 frames, seed 0, cps 1, duration 0.25, one stereo orbit).
write_jobs() {
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
		}' >"$B/extra/$(printf '%s%02d' "$1" "$i").job.json"
	done
}

extra_jobs() {
	rm -rf "$B/extra"
	mkdir -p "$B/extra"
	# x01-x45: #122's eq.sh jobs, unchanged (the sounds, the silence and bank edge cases, the
	# refused sounds, an FX entry with i/ir, an lfo onto a noise source's note).
	write_jobs x <<'EOF'
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
[{"s":null}]
[{"s":""}]
[{"s":true}]
[{"s":"Sine"}]
[{"s":"sine "}]
[{"s":[["sine"]]}]
[{"bank":"x"}]
[{"s":"sine","bank":""}]
[{"s":"white","lfo":{"__ids":{"$set":[0]},"0":{"control":"note"}}}]
EOF
	# y01-y07: an lfo at a missing node ("Available targets", the registration order: y02 the
	# oscillator's vib before its FM, y03/y04 the worklet synths' vib after it, y05/y07 a chain
	# entry's keys, y06 the main entry's sends and djf). y08-y23: one throw site each (y23 is
	# the amp envelope's release ramp: with release Infinity no FXrelease gain is built).
	# y24-y32: the throw ladder (every site armed, then the first one dropped, job by job).
	# y33-y37: the FM matrix (a noise target, fmi/fmh modulators, throws). y38-y40, y45:
	# ladders, 24 dB and a chain entry with every key. y41-y44: the pulse's NaN width LFO and
	# worklet synths with vib and FM under modulators. y46-y50: the order between sites that
	# share a text: the FXrelease gain's ramp before post (y46), a chain entry before the
	# FXrelease gain (y47), a noise FM target's modfreq gain (y48), pw_lfo before the entries
	# (y49), a chain entry's local delay before its local reverb (y50).
	write_jobs y <<'EOF'
[{"s":"sawtooth","cutoff":500,"pan":0.3,"lfo":{"__ids":{"$set":[0]},"0":{"control":"hcutoff"}}}]
[{"s":"sawtooth","vib":2,"vibmod":0.5,"fmi":1,"fmi2":0.5,"lfo":{"__ids":{"$set":[0]},"0":{"control":"hcutoff"}}}]
[{"s":"supersaw","vib":2,"vibmod":0.5,"fmi":1,"fmi2":0.5,"lfo":{"__ids":{"$set":[0]},"0":{"control":"hcutoff"}}}]
[{"s":"pulse","pwsweep":0.3,"vib":2,"vibmod":0.5,"fmi":1,"fmi13":2,"lfo":{"__ids":{"$set":[0]},"0":{"control":"hcutoff"}}}]
[{"s":"sine","FX":[{"cutoff":500,"pan":0.2,"lfo":{"__ids":{"$set":[0]},"0":{"control":"hcutoff"}}}]}]
[{"s":"sine","delay":0.5,"delaytime":0.2,"delayfeedback":0.3,"room":0.3,"djf":0.3,"lfo":{"__ids":{"$set":[0]},"0":{"control":"hcutoff"}}}]
[{"s":"sine","FX":[{"cutoff":500,"lpattack":0.1,"postgain":0.5,"transient":0.5,"hcutoff":100,"bandf":1000,"vowel":"a","coarse":2,"crush":4,"shape":0.3,"distort":0.5,"tremolo":2,"compressor":-10,"pan":0.3,"phaserrate":2,"delay":0.5,"delaytime":0.1,"delayfeedback":0.4,"room":0.3,"roomsize":0.1,"lfo":{"__ids":{"$set":[0]},"0":{"control":"postgain"}}}]}]
[{"s":"sine","cutoff":100,"resonance":"x"}]
[{"s":"sine","velocity":"x","pan":"x"}]
[{"s":"sine","vowel":"zz","pan":"x"}]
[{"s":"sine","tremolo":2,"tremolodepth":"x"}]
[{"s":"sine","pan":"x"}]
[{"s":"sine","velocity":0.5,"FX":[{"pan":"x"}],"cutoff":100,"resonance":"x"}]
[{"s":"sine","postgain":"x"}]
[{"s":"sine","cutoff":500,"lprate":"x"}]
[{"s":"sine","coarse":"x"}]
[{"s":"sine","shape":0.3,"shapevol":"x"}]
[{"s":"sine","distort":0.3,"distortvol":"x"}]
[{"s":"sine","compressor":{"$num":"NaN"}}]
[{"s":"sine","phaserrate":"x"}]
[{"s":"sine","FX":[{"delay":0.3,"delaytime":1e300}]}]
[{"s":"sine","FX":[{"room":1e300,"roomsize":0.1}]}]
[{"s":"sine","FXrelease":0.5,"release":{"$num":"Infinity"}}]
[{"s":"sine","velocity":"x","cutoff":100,"resonance":"x","vowel":"zz","coarse":"x","tremolo":2,"tremolodepth":"x","compressor":{"$num":"NaN"},"pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","cutoff":100,"resonance":"x","vowel":"zz","coarse":"x","tremolo":2,"tremolodepth":"x","compressor":{"$num":"NaN"},"pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","vowel":"zz","coarse":"x","tremolo":2,"tremolodepth":"x","compressor":{"$num":"NaN"},"pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","coarse":"x","tremolo":2,"tremolodepth":"x","compressor":{"$num":"NaN"},"pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","tremolo":2,"tremolodepth":"x","compressor":{"$num":"NaN"},"pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","compressor":{"$num":"NaN"},"pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","pan":"x","phaserrate":"x","postgain":"x"}]
[{"s":"sine","phaserrate":"x","postgain":"x"}]
[{"s":"sine","postgain":"x"}]
[{"s":"sine","fmi":1,"fmi13":3,"fmwave3":"white","fmi2":2,"fmh":2,"lfo":{"__ids":{"$set":[0,1]},"0":{"control":"fmi"},"1":{"control":"fmh"}}}]
[{"s":"sine","fmi13":3,"fmwave3":"white","fmi2":2,"lfo":{"__ids":{"$set":[0,1]},"0":{"control":"fmi3"},"1":{"control":"fmh3"}}}]
[{"s":"sine","fmi":"x"}]
[{"s":"sine","fmi":1,"fmh":"x"}]
[{"s":"supersaw","vib":2,"fmi":"x"}]
[{"s":"sawtooth","cutoff":500,"hcutoff":200,"bandf":900,"lprate":2,"hprate":3,"ftype":"ladder"}]
[{"s":"sawtooth","cutoff":500,"hcutoff":200,"bandf":900,"lprate":2,"ftype":"24db","FX":[{"cutoff":400,"ftype":"ladder"}]}]
[{"s":"sawtooth","FXrelease":0.3,"FX":[{"transient":0.5,"cutoff":500,"hcutoff":100,"bandf":1000,"lprate":2,"vowel":"a","coarse":2,"crush":4,"shape":0.3,"distort":0.5,"tremolo":2,"compressor":-10,"pan":0.3,"phaserrate":2,"delay":0.5,"delaytime":0.1,"delayfeedback":0.4,"room":0.3,"roomsize":0.1,"lfo":{"__ids":{"$set":[0]},"0":{"control":"lpattack"}}}],"cutoff":300}]
[{"s":"pulse","pwsweep":"x"}]
[{"s":"pulse","pwrate":"x"}]
[{"s":"supersaw","vib":3,"vibmod":0.5,"fmi":1.5,"fmh":2,"fmi13":1,"fmwave3":"white","lfo":{"__ids":{"$set":[0]},"0":{"control":"vib"}}}]
[{"s":"pulse","vib":3,"vibmod":0.5,"fmi":1.5,"fmi2":0.7,"pwrate":2,"lfo":{"__ids":{"$set":[0]},"0":{"control":"vibmod"}}}]
[{"s":"sawtooth","cutoff":500,"hcutoff":300,"ftype":"24db","lprate":2,"hprate":2,"FX":[{"bandf":700,"bprate":1}]}]
[{"s":"sine","FXrelease":{"$num":"Infinity"},"postgain":"x"}]
[{"s":"sine","FX":[{"pan":"x"}],"FXrelease":{"$num":"Infinity"}}]
[{"s":"sine","fmi13":1,"fmwave3":"white","fmh3":"x"}]
[{"s":"pulse","pwsweep":"x","velocity":"x"}]
[{"s":"sine","FX":[{"room":1e300,"roomsize":0.1,"delay":0.3,"delaytime":1e300}]}]
EOF
}

# record ENGINE DIGEST OUT: the dump, every job's signatures and render, and the digests, into OUT.
record() {
	eng=$1
	dig=$2
	out=$3
	rm -rf "$out"
	mkdir -p "$out/sig" "$out/stderr" "$out/tmp"
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
		"$eng" render --job "$job" --out "$wav" >"$out/tmp/stdout" 2>"$out/stderr/$name.txt"
		rcode=$?
		set -e
		echo "exit $scode" >>"$out/sig/$name.txt"
		if [ -f "$wav" ]; then
			wsum=$(shasum -a 256 "$wav" | cut -d' ' -f1)
		else
			wsum=none
		fi
		esum=$(shasum -a 256 <"$out/stderr/$name.txt" | cut -d' ' -f1)
		if ! stdout=$(jq -c 'del(.renderMs)' "$out/tmp/stdout" 2>/dev/null); then
			stdout=$(cat "$out/tmp/stdout")
		fi
		printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$rcode" "$wsum" "$esum" "$stdout" >>"$out/render.tsv"
		rm -f "$wav"
	done
	rm -rf "$out/tmp"
	"$dig" "$REFS" >"$out/digest-refs.txt" 2>"$out/digest-refs.stderr"
	"$dig" "$B/extra" >"$out/digest-extra.txt" 2>"$out/digest-extra.stderr"
}

# expect NAME TEXT: the baseline render line of job NAME contains TEXT.
expect() {
	if ! awk -F'\t' -v n="$1" '$1 == n' "$B/base/render.tsv" | grep -qF -- "$2"; then
		echo "ab.sh: job $1 does not log: $2" >&2
		exit 2
	fi
}

case "${1:-}" in
baseline)
	if ! git -C "$REPO" diff --quiet "$BASE_SRC" -- 'crates/*/src/*' crates/strudeland-engine/tests/refs; then
		echo "ab.sh: the sources or refs differ from $BASE_SRC; refusing to record a baseline" >&2
		exit 2
	fi
	cp "$REPO/Cargo.lock" "$B/digest/Cargo.lock"
	build baseline
	extra_jobs
	record "$B/bin/strudeland-engine-baseline" "$B/bin/engine-digest-baseline" "$B/base"
	bad=$(grep -L '^exit 0$' "$B"/base/sig/x*.txt "$B"/base/sig/y*.txt || true)
	if [ -n "$bad" ]; then
		echo "ab.sh: extra jobs the engine rejects: $bad" >&2
		exit 2
	fi
	bad=$(awk -F'\t' '$1 >= "x04" && $1 <= "x21" && $2 != 0 { print $1 }' "$B/base/render.tsv")
	if [ -n "$bad" ]; then
		echo "ab.sh: sound jobs that do not render with exit 0: $bad" >&2
		exit 2
	fi
	sounds=$(jq '.sounds | length' "$B/base/controls.json")
	targets=$(jq '.modulatorTargets | length' "$B/base/controls.json")
	if [ "$sounds" != 20 ] || [ "$targets" != 91 ]; then
		echo "ab.sh: the dump has $sounds sounds and $targets targets (want 20 and 91)" >&2
		exit 2
	fi
	# The new jobs reach what they are there for.
	for n in y01 y02 y03 y04 y05 y06 y07; do expect "$n" "Available targets"; done
	expect y02 "Available targets: source, vib, vib_gain, fm_1, fm_1_gain, fm_2, fm_2_gain, gain, post"
	expect y03 "Available targets: source, fm_1, fm_1_gain, fm_2, fm_2_gain, vib, vib_gain, gain, post"
	expect y04 "Available targets: source, pw_lfo, fm_1, fm_1_gain, fm_3, fm_3_gain, vib, vib_gain, gain, post"
	expect y07 "delay, delay_mix, room, room_mix"
	expect y08 "Q is not a finite"
	expect y13 "pan is not a finite"
	expect y34 "lfo:fmh3"
	expect y46 "linearRampToValueAtTime"
	expect y47 "pan is not a finite"
	expect y48 "gain is not a finite"
	expect y49 "'value' on 'AudioParam'"
	expect y50 "'value' on 'AudioParam'"
	panics=$(cat "$B/base/digest-refs.txt" "$B/base/digest-extra.txt" | grep -c 'panic$' || true)
	if [ "$panics" != 0 ]; then
		echo "ab.sh: $panics digest variants panic on the baseline" >&2
		exit 2
	fi
	mkdir -p "$CACHE"
	cp "$0" "$CACHE/ab.sh"
	rsync -a --delete --exclude target "$B/digest/" "$CACHE/digest/"
	rsync -a --delete "$B/base/" "$CACHE/base/"
	git -C "$REPO" rev-parse HEAD >"$CACHE/SOURCE_SHA"
	echo "baseline: $(wc -l <"$B/base/render.tsv" | tr -d ' ') jobs, $(cat "$B/base/digest-refs.txt" "$B/base/digest-extra.txt" | wc -l | tr -d ' ') digest lines, $sounds sounds, $targets targets"
	;;
check)
	if [ ! -d "$B/base" ]; then
		echo "ab.sh: no baseline at $B/base (ab.sh restore, or see 001-baseline.md)" >&2
		exit 2
	fi
	if ! git -C "$REPO" diff --quiet "$BASE_SRC" -- crates/strudeland-engine/tests/refs; then
		echo "ab.sh: tests/refs differs from $BASE_SRC; the guard compares against unchanged refs" >&2
		exit 2
	fi
	if ! cmp -s "$B/digest/src/main.rs" "$CACHE/digest/src/main.rs"; then
		echo "ab.sh: digest/src/main.rs differs from (or is missing in) $CACHE; the check builds the baseline's digest" >&2
		exit 2
	fi
	build current
	extra_jobs
	record "$B/bin/strudeland-engine-current" "$B/bin/engine-digest-current" "$B/cur"
	if diff -r "$B/base" "$B/cur" >"$B/diff.txt"; then
		echo "0 differences: $(wc -l <"$B/cur/render.tsv" | tr -d ' ') jobs (renders, signatures), the dump, $(cat "$B/cur/digest-refs.txt" "$B/cur/digest-extra.txt" | wc -l | tr -d ' ') digest lines"
	else
		echo "DIFFERENT ($(wc -l <"$B/diff.txt" | tr -d ' ') diff lines, $B/diff.txt):"
		head -60 "$B/diff.txt"
		exit 1
	fi
	;;
restore)
	if [ ! -d "$CACHE/base" ]; then
		echo "ab.sh: no snapshot at $CACHE" >&2
		exit 2
	fi
	rsync -a --exclude target "$CACHE/digest/" "$B/digest/"
	rsync -a --delete "$CACHE/base/" "$B/base/"
	echo "restored base/ and digest/ from $CACHE (recorded at $(cat "$CACHE/SOURCE_SHA"))"
	;;
*)
	echo "usage: ab.sh baseline | check | restore" >&2
	exit 2
	;;
esac
```

## Recovery

**The scratchpad was cleared, or this is a new session.** The cache holds everything `check`
needs:

```sh
B=$S/0127-baseline
mkdir -p "$B"
cp ~/.cache/strudeland-0127/ab.sh "$B/"
sh "$B/ab.sh" restore
sh "$B/ab.sh" check   # on a committed stage: 0 differences, as that stage's own log said
```

**The cache is lost too.** Rebuild from `416caa6`. No stage edits the refs, so the extra jobs and
refs are the same:

1. Make a WIP commit if the working tree is dirty.
2. `git switch --detach 416caa6`. Never use a scratch worktree.
3. Set up `$B` as in step 1 (the script from the quote above, the digest from
   `~/.cache/strudeland-0121/digest/`). If that is gone as well, stop and report: the digest's
   `main.rs` is not quoted here, and a rewritten digest is a different guard.
4. `sh "$B/ab.sh" baseline`, then `sh "$B/ab.sh" check` (0 differences).
5. `git switch -`, then undo the WIP commit with `git reset HEAD~1`, if there is one.

## Tests

None added, deleted or moved. This stage writes no repository file.

## Gates

The two logs above, `$S/0127-001-baseline.log` and `$S/0127-001-ab.log`. Both end `exit 0`.
The cargo gates are not run: nothing changed. `git status --short crates/` prints nothing.

## STOP conditions

- The refusal in the prerequisites, or any `ab.sh baseline` refusal.
- `check` reports any difference right after `baseline`.
- A digest variant panics on `416caa6` (the baseline refuses). That is a finding: report the job
  and the variant.
- A `y` job does not log what its comment says it reaches (the baseline refuses). Report the
  job's stdout. Do not edit the job to make it pass without saying so in the report.

## Commit message

None. This stage commits nothing: the guard lives in the scratchpad and the cache, and the
plan files are committed by the workflow's sequencing step.
