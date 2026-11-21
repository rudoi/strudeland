---
name: strudeland
description: Drive the running strudeland app (a Strudel live-coding editor) from the shell. Use when the working folder has project.json and songs/*.js, or when the user asks to play, stop, edit, write, check or debug Strudel music or a song in strudeland.
allowed-tools: Bash(strudeland:*)
---

# strudeland

strudeland is a desktop Strudel editor. A project is a folder with `project.json` and one Strudel
script per song in `songs/*.js`. While the app is open, the `strudeland` command reads and edits
its editor, plays songs and reports what they play. Work through the CLI, not by editing files.

## Start

1. `strudeland ping`: `strudeland X is running` (exit 0) means go ahead.
   - Exit 2 (`error: strudeland is not running`): ask the user to launch the app.
     Do not try to start it yourself.
   - Exit 3 (`strudeland X is running but not ready`): the app is open but no window is attached
     yet. Wait a moment and ping again; if it persists, ask the user to check the window.
2. `strudeland state`: the open project, songs, tempo, audio, keys and playback.

```
project: demo (/Users/me/demo)
songs:
  ● main   "Main Theme"   active, playing
    verse  "Verse"        conflict: changed
tempo: 120 bpm 4/4
audio: running
playing: Main Theme
```

`●` marks a song with unsaved edits. `active` is the song in the editor. `conflict: changed` means
the file changed on disk while the editor had unsaved edits (`conflict: deleted`: the file is
gone). Other lines appear when they apply: `playhead: bar 5 (body 3)`, `transport: loop body,
armed tail` (or `ending`), `seed: 7` (the song's `meta.seed`, after the tempo), `keys: vim` (after `audio:`), `eval error: …`, `meta error: …`, `sound error: …`, `engine: rust` (only when the Rust engine plays the song), `live: underruns 0, late 0, haps …, headroom 38 ms, invoke p50 … p95 … max … ms` (the Rust engine's stats; underruns and late haps should be 0, `headroom` is the audio buffer's minimum fill, `skipped` counts haps the engine does not support, `invoke` is the app's round trip for a batch), `levels  peak -0.4 / +2.1 dBFS  clipped 1834 samples  limiter off` (the master output's held peak per channel, how many samples went over full scale, and whether the safety limiter is on, with its deepest gain reduction, e.g. `limiter on (-2.3 dB)`; `-` is silence; absent until audio has played), and `(setcps in the
code overrides meta.bpm)` after the tempo. `audio: suspended` means nothing can sound yet (see
Audio).

To check the mix for clipping: play (`strudeland eval`), let it run through a loud part, then read
`levels` in `strudeland state`. The numbers are held since the last play or evaluate (a stop keeps
them). The peaks and `clipped` are measured before the limiter, so they describe the mix as
written: `clipped` above 0 means the mix went past full scale (a peak above 0 dBFS is the same
problem), even when the limiter caught it. With the limiter off that is heard as crackle; fix it by
lowering `gain`/`postgain` in the song. With it on, nothing past -1 dBFS reaches the output and
`limiter on (-2.3 dB)` is how far it had to pull the level down; tell the user the mix still
clips under the limiter rather than silently changing the song. Only the user can turn the
limiter on or off (in the app; the CLI cannot).

`keys: vim` means the user's editor has vim keybindings on, so it may be in normal mode when
they type. `code get`, `code set` and `eval` behave the same either way: they replace or read
the text and the editor keeps its vim mode.

## Commands

| Command | What it does |
|---|---|
| `strudeland state` | project, songs, tempo, audio, keys, playing song, playhead, transport, last errors, engine, live stats, master levels |
| `strudeland code get [--song S]` | print the song's editor text (unsaved edits included), exactly as is |
| `strudeland code set [--song S] [--file F] [--eval \| --no-eval] [--expect-file F]` | replace the song's editor text |
| `strudeland eval [--song S]` | evaluate the song's editor text and play it |
| `strudeland play [--song S]` | the same: evaluate and start playing |
| `strudeland stop` | stop playback (`stopped` or `not playing`) |
| `strudeland song new NAME` | create `songs/NAME.js` from the template and switch the editor to it |
| `strudeland song switch SONG` | show a song in the editor |
| `strudeland song rename [--song S] NEW` | rename a song's file; unsaved edits stay in the editor |
| `strudeland song delete [--song S] [--discard]` | delete a song's file |
| `strudeland save [--song S \| --all]` | write the editor text of a song (or every song with unsaved edits) to disk |
| `strudeland project open PATH [--discard]` | open another project in the app |
| `strudeland events [--bars N\|N-M] [--section NAME] [--all] [--limit N]` | the evaluated song's events with bar, beat, value and source position |
| `strudeland screenshot [PATH]` | PNG of the app window; prints `wrote PATH (WxH)` |
| `strudeland docs NAME` / `docs --search TERM` / `docs --list` | Strudel function docs (exactly one of the three) |
| `strudeland render SONG [--out PATH] [--format F] [--stems orbit\|voice\|instrument \| --preset tagged-loop\|intro-loop\|folded-loop\|one-shot] [--section NAME \| --length N] [--loops N] [--lufs DB] [--true-peak DBTP] [--loudness on\|off] [--bit-depth B] [--mp3-mode M] [--quality Q] [--jobs N] [--plan-timeout SECS]` | render a song file to audio, offline (see Rendering) |
| `strudeland memo list [--project DIR]` / `strudeland memo show NAME [--project DIR]` | list the project's voice memos / show one memo's timing and settings, offline (see Memos) |
| `strudeland transcribe NAME [--project DIR] [--grid G] [--bpm N] [--meter N/D] [--bar1 S] [--shift MS] [--key ROOT:SCALE] [--tuning CENTS] [--start S --end S \| --bars A-B] [--debug]` | turn a voice memo into proposed Strudel code, offline; never writes a song (see Transcribing memos) |
| `strudeland ping` | is the app running and ready |
| `strudeland skill print` / `strudeland skill install` | print this skill / install it for Claude Code |

`--song` exists on `code get`, `code set`, `eval`, `play`, `save`, `song rename` and `song delete`
only; `song switch` takes its song as an argument. It takes a stem (`main`, `main.js`) or a path
(`songs/main.js`) and defaults to the active song. `code set`, `eval`, `play`, `song new` and `song
switch` switch the editor to that song; `code get`, `save`, `song rename` and `song delete` do not.
An unknown song fails with `error: no such song 'x' (available: main, verse)`.

## Editing

`code set` replaces the editor text as one undoable step, marks the lines it changed in the
editor, and leaves the song unsaved. Input is `--file F`, or stdin when `--file` is absent or `-`,
sent as is. Always pass `--file` or pipe something: an empty non-terminal stdin empties the song.

- Evaluation: by default `code set` evaluates only if that song is the one playing and the text
  actually changed. `--eval` always evaluates; `--no-eval` never does. Both together is a usage
  error.
- `--expect-file F`: fail unless the song's current editor text equals F. On a mismatch it changes
  nothing, prints `error: the song changed since you read it` and exits 1. Re-read with `code get`
  and redo the edit on the new text; the user has probably been typing.
- Output, first line: `main: updated (unsaved)` (applied, song now has unsaved edits), `main:
  updated` (applied, and the text now equals the saved file), or `main: no change` (identical
  text; nothing evaluated in auto mode). Then, if evaluation ran: `playing Main Theme`,
  `evaluated Main Theme (not playing)`, or `error: …` on stderr with exit 1.
- Conflict notes, printed after the first line:
  - `note: the file changed on disk; the conflict bar is still open`: someone else changed the
    file. Your edit is in the editor, but the user has to choose in the app's bar between the
    editor text and the disk text. Tell them.
  - `note: the file was deleted on disk`: the file is gone; the user's save shortcut would recreate it.
    Tell them.

**Save only when the user asks.** `code set`, `eval` and `play` work on the editor buffer, and
that buffer is what gets evaluated. The user saves with Command-S on macOS or Control-S on Windows
and Linux. Pressing Enter with the same modifier saves and evaluates. When they ask you to save,
run `strudeland save` (the active song) or `strudeland save --song S` / `--all`; it prints `saved main` or `nothing to save`. Do not write over `songs/*.js`
to "save" for them.

**Writing files directly.** Create songs with `strudeland song new NAME`, not by writing a file.
Avoid writing an existing song's file: it hot-reloads (and re-evaluates if it is the active song
and playing) only when that song has no unsaved edits; otherwise it raises a conflict the user has
to resolve. Do it only when the user asks for the change on disk, and check `state` first (no `●`).

### The edit loop

```sh
strudeland state                                    # which song is active, is it playing
strudeland code get --song main > /tmp/main.orig.js
# write the changed song to /tmp/main.new.js with the Write tool
strudeland code set --song main --file /tmp/main.new.js --expect-file /tmp/main.orig.js --eval
strudeland events --bars 1-2                        # did it play what you meant
strudeland screenshot main.png                      # then Read the reported file
```

Use `--eval` when the song is not playing yet and you want to hear and inspect it. If `code set`
prints an `error:` from evaluation, fix the code and `code set` again (the broken text is in the
editor; the previous pattern keeps playing). Keep `/tmp/main.new.js` as the new `--expect-file`
for the next round.

## Songs and projects

- `strudeland song new NAME` creates `songs/NAME.js` from the template and switches to it; it
  prints `created main2`. NAME is a stem: letters, digits, space, `-` and `_`, 1 to 64 characters,
  no `.js`. An existing name fails with `error: song songs/main2.js already exists`.
- `strudeland song switch SONG` shows a song in the editor: `switched to verse`.
- `strudeland song rename [--song S] NEW` renames a song (the active one by default): `renamed
  main → intro`. Unsaved edits and undo history move with it, so nothing is lost and no flag is
  needed. It does not switch the editor. A song whose file was deleted on disk cannot be renamed
  until it is saved (`strudeland save --song S` recreates the file).
- `strudeland song delete [--song S] [--discard]` deletes a song's file: `deleted verse`, plus
  `stopped playback` when it was the playing song. It does not switch the editor.
- `strudeland save [--song S | --all]` writes editor text to disk, only for songs that need it:
  `saved main, verse`, or `nothing to save`. When the desktop app's **Format on Save** setting is
  enabled, saving can align eligible mini-notation in both the editor and the file. Use this command
  only when the user asks you to save.
- `strudeland project open PATH [--discard]` opens the project in PATH (relative to the current
  directory) in the app: `opened demo (/path)`, or `demo is already open`. A path that is not a
  project fails with `error: … is not a strudeland project (no project.json)`.

`song delete` and `project open` would lose unsaved edits, so they refuse: exit 1, `error: … (use
discard to …)` and a `hint:` line. Run `state` (`●` marks unsaved songs) and ask the user whether
to save them (`strudeland save`) or throw them away. **Never pass `--discard` unless the user said
to.**

`error: the app is waiting on an open dialog; try again after it closes` (exit 1) means the user
has an open-project dialog up: ask them to finish it, then try again.

A `project open` or `save --all` that times out (exit 3) may still complete in the app. Run
`state` before retrying.

## Memos

The user records voice memos in the app. A memo is a mono 32-bit float WAV in the project's
`recordings/` folder plus a `NAME.json` sidecar that says how it was recorded. **You cannot record:
the CLI never opens the microphone.** `strudeland memo` only reads, and it works offline.

- `strudeland memo list [--project DIR]` prints one row per memo (`main-01  0:12.4  120 bpm  4/4
  song+metronome+count-in  bar 1 known`), or `no memos`. `?` or `-` means unknown: the WAV header
  could not be read, there is no sidecar, or it is bad (`bad sidecar`, with a `warning:` on stderr).
- `strudeland memo show NAME [--project DIR]` prints the path, duration, input, tempo, backing
  and bar 1 position, one `key: value` line each. A trailing `.wav` on NAME is ignored. Use
  `--json` for exact values.
- The project is `--project DIR` (that folder only, no searching), else the project the current
  directory is in (searching upwards), else the project open in the running app. With none, exit 4
  and `error: not in a strudeland project; ...`.
- `--json`: `memo list` prints `{"project": ROOT, "memos": [...]}`; each memo has `name`, `path`
  (absolute WAV), `duration`, `frames`, `sampleRate`, `size`, `modifiedMs`, `hasSidecar`,
  `sidecarError`, and from a valid sidecar `tempo`, `backing`, `bar1Frame`, `bar1Seconds`,
  `recovered` (else `null`). `memo show` prints the whole sidecar (`schema`, `createdAt`, `app`,
  `input`, `output`, `tempo`, `backing`, `timing`, `xruns`, `recovered`) plus `name`, `path`,
  `duration`, `frames`, `sampleRate`, `bar1Seconds` and `hasSidecar`.
- `timing.bar1Frame` is a fractional 0-based sample index into the WAV where bar 1's downbeat is;
  `bar1Seconds` is that divided by the WAV's sample rate. With a count-in, the audio before it is
  the count-in. `null` means the take has no clock: infer the phase from the audio instead.
- `tempo.bpm` is quarter notes per minute; a bar lasts `tempo.beatsPerBar x 60 / tempo.bpm`
  seconds. `tempo.tempoOverride` is true when the song's own `setcps` replaced `meta.bpm`.
- `recovered: true` means the take was salvaged after a crash: its tempo is a 120 bpm 4/4
  placeholder and it has no timing. Do not trust it.
- The host-nanosecond fields in `timing` (`bar1HostNs`, `anchors`, ...) are strings, not numbers.
- Errors: `error: memo NAME not found` and a bad or unsupported sidecar are exit 1; an invalid
  NAME and no project are exit 4.
- While the user is recording a take, `eval`, `play` and `code set` never start playback into it,
  and `strudeland stop` ends and saves the take (as the app's stop button does). Do not run
  `stop` while the user is recording unless they ask. These commands also stop a memo the user
  is playing back.

## Transcribing memos

`strudeland transcribe NAME` listens to a voice memo (a sung or hummed melody) and proposes Strudel
code for it: note names, and scale degrees when it finds a key. It works offline from the WAV and
its sidecar. **It never writes anything and never reads the song file**: the code is a proposal.
Put it in the song with `code set`, which changes the editor but does not save, so the user can
audition it and correct it by ear; save only when they ask (see the workflow below).

```
strudeland transcribe NAME [--project DIR] [--grid 4|8|16|8t|16t] [--bpm N] [--meter N/D]
  [--bar1 SECONDS] [--shift MS] [--key ROOT:SCALE] [--tuning CENTS]
  [--start S --end S | --bars A-B] [--debug]
```

| Flag | What it does |
|---|---|
| `--project DIR` | the project folder, as for `memo` |
| `--grid G` | steps per quarter note: `4`, `8`, `16` (default), `8t` or `16t`; the grid the notes snap to |
| `--bpm N` | tempo in quarter notes per minute, 20 to 400; beats the sidecar |
| `--meter N/D` | meter such as `3/4` or `6/8`; beats the sidecar |
| `--bar1 SECONDS` | where bar 1's downbeat is in the WAV; beats the sidecar |
| `--shift MS` | move the grid later (positive) or earlier (negative), in milliseconds |
| `--key ROOT:SCALE` | skip key detection, e.g. `C:major`, `A:minor`, `D:dorian` |
| `--tuning CENTS` | the singer's tuning offset; `0` turns the correction off |
| `--start S --end S` | analyse only these seconds of the file (give both) |
| `--bars A-B` | analyse only these bars of the memo, 1-based and inclusive; needs a known grid (a take recorded without song or metronome needs `--bpm` and `--bar1`) |
| `--debug` | add `frames` and `onsets` to the JSON; needs `--json` |

A memo over 10 minutes needs `--start/--end` or `--bars`. With a sidecar the grid (tempo, meter and
bar 1) comes from it; each flag replaces its own part. Without a sidecar, or for a `recovered`
memo, whatever the flags do not give is inferred from the audio, which is less reliable.

### Reading the result

Use `strudeland --json transcribe NAME` (or `transcribe NAME --json`). The human output shows the
same code and a table of notes (`bar:beat`, `length` in grid steps, `conf`; a trailing `?` marks a
low-confidence note), but the JSON is exact:

- `grid`: `bpm`, `meter`, `beatsPerBar`, `origin` (bar 1's downbeat in seconds), `stepsPerBar`,
  `division`, `source` (`tempo` and `phase`, each `flag`, `sidecar` or `inferred`) and
  `confidence` (of the inferred parts, else `null`), `suggestedDivision` (a `--grid` value the
  rhythm fits clearly better, else `null`) and `suggestedShiftMs` (a `--shift` value that brings
  the notes clearly closer to the grid, else `null`; when `suggestedDivision` is set it is
  measured on that division, so pass the two together). Both only suggest: the notes and code use
  the grid asked for. The suggested shift measures timing within a grid step. If a comparison
  against a known melody shows a whole-step difference, pass that alignment explicitly via
  `--shift`. `null` when the grid could not be found.
- `tuning`: `cents` and `source` (`estimated` or `flag`). Positive means the singer was sharp, so
  the note names are already corrected down by that much. A big value (over 40) can put a note
  name a semitone out.
- `key`: `name` (`A minor`), `scale`, `source` (`detected` or `flag`), and for a detected key
  `confidence`, `runnerUp` and `correlation`. `null` when there is no key; `warnings` says why.
- `notation`: `note` (mini-notation of note names), `n` and `scale` (scale degrees and the
  `.scale(...)` argument; `null` without a key), `firstBar` and `lastBar`. The code is
  `note("<note>")` or `n("<n>").scale("<scale>")`. `null` without notes or a grid.
  When a note holds over a barline the pattern can use `[...]/N` to place the whole phrase over
  N cycles, keeping the hold as one note.
- `notes`: each has `start` and `end` (raw seconds in the file), `bar`, `step` and `steps` (on the
  grid), `midi`, `name`, `degree`, `cents`, `confidence` (0 to 1, below 0.5 is shaky), its
  `components` (`voiced`, `stability`, `fit`, `onset`), `snapErrorMs` (how far it moved to reach
  the grid), `tiedToNext` (the entry continues into the next bar as one held note) and
  `alternative`: for a note more than 30 cents from its semitone,
  the neighbouring note it leans toward (`{midi, name}`), else `null`. The pitch is ambiguous
  between the two (a singer's intonation varies by up to half a semitone), so treat `name` and
  `alternative.name` as candidates; the table prints it as `c3/c#3`.
- `dropped`: notes the grid removed, with a `reason`. `timingSpread`: milliseconds, how far notes
  sit from the grid (large means the grid or phase is wrong). `warnings`: the same lines as the
  `warning:` output. `backing`: the song and start section the take was recorded against.
- With `--debug`: `frames` (`t`, `f0`, `voiced`, `rms`) and `onsets`; their times are seconds from
  the start of the analysed span, not the file.

**Bars are the memo's bars**, counted from its bar 1. Bar 0 is the pickup bar, the bar before bar 1:
notes sung there (an upbeat during the count-in) are bar 0, and the code then starts with it, so
`notation.firstBar` is 0 and the first cycle is the pickup bar. They are not the song's
bars. The take started at `backing.startSection` (a section name from the song's `meta`), so map
a memo bar to a song bar by that section's first bar: `strudeland events --section NAME` shows
where it starts, or read the song's `meta`. `notation.firstBar` is the memo bar the code's first
bar is; with `--bars 3-6` it is 3. `notation.lastBar` is its last bar: the last bar with a note, or B exactly for `--bars A-B` (empty bars at the end are `~`). The pattern is one bar per cycle, so it plays at the song's
tempo when placed in the song at the right bar.

### Workflow

1. Run `strudeland transcribe NAME --json`. Read `warnings` too. If `grid.suggestedDivision` or
   `grid.suggestedShiftMs` is set, rerun with that `--grid` or `--shift` (both together when
   both are set: the shift is measured on the suggested grid) and tell the user you did and why
   (for example: "the notes are dotted eighths, which a grid of eighths cannot show, so I used
   sixteenths").
2. Tell the user what you heard in musical terms, not as raw JSON: the tempo and where beat 1
   came from (`grid.source`: the sidecar is solid, an `inferred` phase needs their say-so), the
   key (and how sure: a small `confidence` or a close `runnerUp` means it may be the relative
   major or minor), and the rhythm as a feel: dotted notes, a pickup into the first bar (bar 0),
   a bar of silence between phrases. Ask about rhythm in those terms, not in steps.
3. The user may not know note names, so propose by ear: write the transcription into the song
   as a voice (a new `$:` line, or the part they name) with `strudeland code set`, which changes
   the editor but does not save. Keep the memo's rests and phrasing in the music: a phrase, its
   rests and the silent bars between phrases stay as sung (use `firstBar` and the bar numbers to
   place it), never compressed together. Mark the uncertain notes in a code comment that names
   their alternatives: those with a low `confidence` and those with an `alternative` (for
   example `// bar 1: c3 may be c#3; bar 2: the f3 is shaky`). Follow the edit loop:
   `strudeland code get > ORIG.js`, write the song with the voice added to `NEW.js`, `strudeland
   code set --file NEW.js --expect-file ORIG.js` (a change in the meantime is refused), then
   offer `strudeland play`.
4. Ask the user to play it and correct by ear: which notes are wrong, higher or lower, longer or
   shorter. Change the voice as they say (the same edit loop). Their edited version is the
   truth, over the transcription. Save only when they ask (see above): then
   `strudeland save`.

### Rerunning

- Notes sit a beat or a bar out, or `warnings` says the phase was inferred or the timing spread is
  high: rerun with `--bar1 SECONDS` (from the user, or `memo show`) or nudge with `--shift MS`
  (positive if the notes land late against the grid, negative if early). Latency from Bluetooth
  headphones is the usual cause of a constant offset.
- Too busy (many tiny notes, runs of sixteenths where the user sang eighths): `--grid 8`, or `4`.
  Too sparse or notes merged and lengths wrong: `--grid 16`, or `16t` and `8t` for a triplet feel.
- Wrong key (the notes look right but the degrees are odd): `--key ROOT:SCALE`. If the user knows
  the key, pass it.
- Note names a semitone or more off everywhere: `--tuning CENTS` (the singer's offset, positive
  if sharp), or `--tuning 0` when the singer was in tune and the estimate is wrong.
- Wrong tempo or meter (`inferred`, or the take ignored the click): `--bpm N`, `--meter N/D`.
- A long memo, or only one phrase matters: `--bars A-B` or `--start S --end S`.

### Errors and warnings

- Exit 1: the memo is not found, its sidecar is bad, the WAV cannot be read, or bar 1 is past the
  end of the audio (`bar 1 (X s, from --bar1) is past the end of the audio (Y s)`, or `from the
  sidecar`).
- Exit 4: a bad flag, no project found, `--debug` without `--json`, `--bars` without a known grid
  (give `--bpm`, `--meter` and `--bar1`), a `--grid` that does not divide the bar into whole
  steps (`3/8` with `--grid 4`: use a finer grid), a bad `--key` or `--meter`, or a memo over
  10 minutes without a span.
- Warnings (stderr, exit 0, also in `warnings`): `phase inferred from the audio` (check beat 1 with
  the user), `tempo and phase inferred` (pass `--bpm` if known), `too few onsets to infer a grid`
  (notes are not quantized, there is no code; pass `--bpm` and `--bar1`), `the take did not follow
  the song's tempo` (the sidecar tempo was replaced by what was heard), `N notes more than a bar before bar 1`
  (in the JSON as bar 0; only pickup-bar notes appear in the code),
  `tuning is X cents off` (see `--tuning`), `no notes found`, `timing spread is X ms`
  (try `--shift` or `--bar1`), `timing fits --grid G better (spread X ms vs Y ms)` (the rhythm
  falls between the requested grid's steps; see `suggestedDivision`), `notes sit ~N ms after the
  grid; try --shift S` (or `before`: the notes share an offset within a step; see
  `suggestedShiftMs`; with a suggested grid it reads `after the --grid G grid; try --grid G
  --shift S`), `no key: REASON` (too few notes, or two keys too close: the `note`
  form still works), `the memo was recovered after a crash` (its tempo is a placeholder and
  ignored), `no sidecar` (the grid is inferred), `meter assumed 4/4` (pass `--meter`).

## Running

`eval` and `play` both evaluate the song's editor text, make it the playing song, and start
playback. Both print `playing Main Theme`. A failed evaluate prints `error: <message>` (for a bad
header, `error: meta: …`) and exits 1; the previous pattern keeps playing. `stop` stops.

## Audio

The browser engine allows audio only after a click in the app window. Until then `eval` and `play`
still evaluate but print `evaluated Main Theme (not playing)` and, on stderr:

```
warning: audio is suspended: click in the strudeland window to enable it
```

and exit 1. Ask the user to click once in the strudeland window, then run `play` again. Do not
retry in a loop. `state` shows `audio: suspended` until then. `events` works either way.

## Checking the result

`events` lists what the last evaluated song plays (it may differ from the active song; there is no
`--song`). It queries the pattern without playing it.

- `--bars N` or `--bars N-M`: 1-based, inclusive, song bars as written in `meta.sections` (the
  loop section counted once). Default: the whole song (up to the limit); a pattern without
  `song()` defaults to bars 1-4.
- `--section NAME`: that section's bars. Conflicts with `--bars`. Unknown names fail with the list
  of sections.
- `--all`: also continuations and events without a start, marked `~`. Default: onsets only.
- `--limit N`: at most N events (1 to 100000, default 2000).

```
song: main "Main Theme"
bars: 1-2 (4/4)

bar 1  intro 1
  1  s=bd @ 7:15
  1  s=sawtooth note=c2 @ 8:17
  2  s=hh @ 7:18
  3  s=sd @ 7:21
  3  s=sawtooth note=eb2 @ 8:22
  4  s=hh @ 7:24
  4  s=sawtooth note=g2 @ 8:26

bar 2  intro 2
  1  s=bd @ 7:15
  …
```

Each block is a bar, with its section and bar within the section. Each line is the beat in the bar
(1-based, in units of the meter's denominator, up to 3 decimals), the value as `key=value` (`s`,
`note`, `n` first), and `@ line:col` in the evaluated text (1-based; several are joined with `,`;
`-` when there is no source position). `no events` means the range is silent. `--json` gives the
exact values and `begin`/`end` in cycles.

Warnings on stderr (exit stays 0):

| Warning | Meaning / what to do |
|---|---|
| `warning: the song changed since it was evaluated; positions refer to the evaluated text` | the editor text differs from what is playing; `eval` first, then re-run `events` |
| `warning: setcps in the code overrides meta.bpm; bar positions follow meta` | remove the `setcps`/`setcpm` call and set `meta.bpm` instead |
| `warning: showing the first N events; narrow --bars or raise --limit` | the list is cut; ask for fewer bars or a higher `--limit` |

`error: nothing has been evaluated yet` (exit 1): run `eval` or `play` first.

`screenshot [PATH]` writes a PNG of the window: editor, timeline with sections and playhead,
Claude's line marks, and any error bar. A relative PATH is taken against the current directory; no
PATH writes a new file in the temp dir. Open the printed path with the Read tool.

## Rendering

`strudeland render SONG` renders a song file to audio (wav, mp3, ogg or opus). It works
offline: the app does not need to be running, and `--timeout` and `--socket` are ignored. It reads
SONG **as it is on disk**, not the app's editor buffer: unsaved edits are not rendered, so run
`strudeland save` first (or give the path of a file you just wrote).

- Length: a song with `meta.sections` renders its clip by default (its loop section played
  `meta.loops` times; `--loops N` changes that). `--section NAME` renders that section once.
  `--length N` renders the first N bars. A song without sections needs `--length N`; without it
  the render fails with exit 4. `--section`, `--length` and `--loops` exclude each other where they
  would contradict (`--loops` only goes with the default clip).
- Output: `--out PATH`, or in a project `renders/<song>.<ext>` for the clip,
  `renders/<song>-<section>.<ext>` with `--section`, `renders/<song>-len<N>.<ext>` with
  `--length N` and `renders/<song>-loops<N>.<ext>` with `--loops N`, so a short check never
  replaces the full render; outside a project `--out` is required. An existing file of the same
  name is replaced without asking. `--format` is `wav`, `mp3`,
  `ogg` or `opus`; without it the `--out` extension decides, else wav.
- Loudness: by default the output is leveled to -23 LUFS with a -1 dBTP true-peak ceiling (EBU R128:
  dynamic songs reach it by gain alone, so every render plays at the same level; the player sets its
  own volume). `--lufs -14` gives streaming loudness, at the cost of limiting.
  `--lufs DB` (-60 to 0) and `--true-peak DBTP` (-20 to 0) change the targets (a negative value
  works as is: `--lufs -16`); `--loudness off` keeps the engine output untouched (and rejects
  `--lufs` and `--true-peak`).
- Encoders: wav is 24-bit by default (`--bit-depth 16|24|32f`); mp3 is CBR 320 kbps (`--mp3-mode
  vbr` with `--quality 0` to `9` for V0 to V9); ogg is quality 6 (`--quality -1` to `10`); opus is
  160 kbps (`--quality` is kbps, 6 to 510). A flag that does not fit the format is exit 4.
- A render without a preset cuts at the musical end of the window.
- `--section` plays the earlier sections' sound into the start of the section; a sound the engine
  cannot play in that lookback region fails the render too.
- Unsupported sounds fail the whole render with exit 1, nothing written, listing each as
  `s=... keys=... first at bar N` (samples, zzfx, sbd, bytebeat and kabelsalat are not supported
  yet). Fix or remove those sounds, or render another part of the song.
- `--jobs N` sets the worker threads (default: the core count up to 4). `--plan-timeout SECS`
  limits how long evaluating the song may take (default 120); a timeout ends
  `(raise it with --plan-timeout)`.

Progress lines go to stderr; stdout gets the summary (path, size, seconds, bars, loudness before
and after, true peak, timings), or with `--json` one line of camelCase JSON (`path`, `format`,
`bytes`, `window`, `frames`, `seconds`, `loudness`, `timings`, `warnings`, ...).

### Stem sets

Use `strudeland render SONG --stems voice --out MIX_DIR` to export a pre-master reference
mix and aligned stems. The usual clip, section, length and loop-count flags select the range.

- `orbit` groups by numeric orbit, including the default orbit, and includes its effects.
- `voice` groups by `.label(...)`. Successive notes and chords with the same label share a stem
  across orbits, and missing or empty labels go in an unlabeled stem.
- `instrument` groups by the engine's resolved sound name, including defaults and supported bank
  prefixes.

Voice and instrument exports include shared-effects stems by orbit. Soloing a shared-effects
stem can include subtraction of direct audio when an orbit filter is active.

Every WAV is stereo 32-bit float at 44.1 kHz, with unity gain and no loudness adjustment or
limiting. Import the files at the same timeline position. The unity-gain sum of the stems
reconstructs `mix.wav` within floating-point rounding. Peaks can exceed full scale. Omit
`--preset`, `--lufs`, `--true-peak`, `--mp3-mode` and `--quality`. The compatible explicit settings
are `--format wav`, `--bit-depth 32f` and `--loudness off`. Incompatible settings exit 4.

`--out` names a new directory. In a project, the default is `renders/<song>-stems`, with the
usual range suffix before `-stems` when a range flag is given. Outside a project, supply `--out`.
An existing path fails with exit 1. The set contains `mix.wav`, numbered stem WAVs and
`stems.json`, which maps filenames to group identities and records the range, sample rate and
frame count. The manifest is written last. On a partial write, stderr names the files already
published and stdout is empty. Choose a new directory to retry.

Stem JSON includes `grouping`, `directory`, `files` (published paths including the manifest),
`reference` (`pre-master`) and `summary`. The human summary also identifies the reference mix and
lists the published paths.

### Export presets

Use `--preset` to export loop assets or append decay to a one-shot.

| Preset | Output |
| --- | --- |
| `tagged-loop` | One file with any intro, the first pass and the marked repeat region. Start at frame zero and loop from the marker through EOF. |
| `intro-loop` | Always two files. Play the `-intro` file once for any intro and the first pass. Then play the `-loop` file, whose repeat marker starts at frame zero. |
| `folded-loop` | One file starting at the marked section with fresh effect state. Start at frame zero to hear the first pass, then loop from the marker through EOF. |
| `one-shot` | The requested clip, section or length followed by automatic decay. A full clip includes any composed ending. |

The loop presets require one section marked `loop: true` and select their own bars. A composed
ending stays outside those files. Combining a loop preset with
`--section`, `--length` or `--loops` is a usage error (exit 4). `one-shot` accepts the usual length
flags.

Intro effects carry into the first pass for `intro-loop` and `tagged-loop`. The repeat region
uses the body alone and folds its decay back into the loop. The opening and repeat share one
static gain. Use WAV for sample-exact playback.

Project defaults add the preset to single-file names. `intro-loop` adds `-intro` and `-loop` to
the song stem by default, or to the stem of `--out`. For example, `--out music.wav` produces
`music-intro.wav` and `music-loop.wav`. The split pair is encoded before either file is written.
Each file is replaced atomically. If the second write fails, stderr names the completed and failed
paths, stdout is empty, and the command exits 1. You can retry the same command.

Preset JSON wraps the output in `{preset, files}`. Each file includes its regular summary fields,
`path`, `role` and `loopPoints`. The split opening has role `intro`. The split repeat and
`folded-loop` have role `loop`. `tagged-loop` and `one-shot` have role `single`. Files without
a marker have `loopPoints: null`. When present, `loopPoints` contains decoded `startFrame`,
`lengthFrames`, `sampleRate` and `godotLoopOffset` in seconds. Human output also reports the
range, rate and offset. MP3 loop exports warn that players may introduce a gap if they leave
encoder delay or padding in place.

In Godot, use **Detect from WAV** for WAV loop markers. For Ogg Vorbis and MP3, enable **Loop**
and set **Loop Offset** to `godotLoopOffset`. Leave looping disabled for the intro file.
[Godot audio import settings](https://docs.godotengine.org/en/4.5/tutorials/assets_pipeline/importing_audio_samples.html)

## Strudel docs

Before using a Strudel function you have not used in this session, look it up. The docs are
embedded in the CLI, work offline and without the app, and match the Strudel version the app runs.

- `strudeland docs lpf`: description, params and examples (synonyms work too).
- `strudeland docs --search reverb`: names, synonyms and descriptions that match.
- `strudeland docs --list`: every documented name.

A miss exits 1: `error: no docs for 'x'` (possibly followed by `did you mean: …`) for a name,
`error: nothing matches 'x'` for `--search`.

## Writing songs

```js
const meta = {
  title: 'main',
  bpm: 120, // quarter notes per minute
  meter: '4/4', // one cycle is one bar
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true }, // at most one loop section
    { name: 'tail', bars: 2 },
  ],
};

$: song(meta, {
  intro: s("bd hh sd hh"),
  body: note("<c2 g1>*4").s("sawtooth"),
  tail: note("c3").s("supersaw"),
})
```

- The header is `const meta = { … }`, a plain object literal. Never `export const`: songs are
  evaluated as scripts.
- Single quotes for plain strings the app reads: `meter`, section names, `title`. Double quotes
  (and backticks) are mini-notation: a double-quoted `meter` or section name is a header error.
- `song(meta, parts)` goes on a `$:` line. `parts` has one pattern per section, keyed by section
  name (identifiers). One `song()` per file; other `$:` lines play throughout.
- Tempo comes from `meta.bpm` (quarter notes) and `meta.meter`; one cycle is one bar. Do not call
  `setcps` or `setcpm`: they override `meta.bpm` and trigger the setcps warnings.
- `meta.seed` is an optional integer (-2147483648 to 2147483647) that fixes the song's randomness
  (`rand`, `degrade`, `shuffle`, and the sound engine's noise and oscillator phases); a `.seed()` in
  the code wins.
- `meta.engine` is `'superdough'` (the default) or `'rust'` (single quotes): which engine plays the song live. `'rust'` skips sounds the Rust engine does not support and shows `rust engine skipped <signature>` in the status bar; `strudeland render` always uses the Rust engine and ignores it; a recording take with song backing plays on superdough; live playback is not byte-reproducible.
- The song is a plain Strudel script, run exactly as written.

## Output and exit codes

- `--json`: print the result as one line of JSON on stdout (errors and warnings stay on stderr).
  Use it when you need exact values, e.g. `strudeland --json state`.
- `--timeout SECS`: how long to wait for the app (1 to 120, default 10).
- `--socket PATH` or `STRUDELAND_SOCKET`: use another control endpoint. Match the app's endpoint.
  Use a private Unix socket on macOS or Linux and a local `\\.\pipe\NAME` on Windows.
- Global flags go before or after the subcommand. Warnings on stderr never change the exit code.

| Exit | Meaning | What to do |
|---|---|---|
| 0 | ok | carry on (still read any `warning:` lines) |
| 1 | the app returned an error, a render failed, an evaluate failed, audio is suspended, `--expect-file` did not match, `docs` found nothing, a `memo` was not found or has a bad sidecar, a `transcribe` memo not found, bad sidecar or unreadable WAV, or a refusal such as unsaved edits, an existing song name or a busy app | read the `error:`/`warning:` line and act on it; do not blindly retry |
| 2 | the app is not running | ask the user to launch strudeland |
| 3 | timeout, or the app is not ready | re-run with a larger `--timeout`, e.g. `--timeout 30`; the message ends `(raise it with --timeout)` |
| 4 | usage error: bad flags, unreadable or non-UTF-8 file, no input for `code set`, a `render` length or preset request the song cannot meet (`--length`, `--section`, `--loops`, `--preset`), a `memo` name that is not valid, no project found for `memo`, a `transcribe` flag the memo cannot satisfy (a bar span without a known grid, a grid that does not divide the bar, a bad key or meter, a memo over 10 minutes without a span) | fix the command line |

## Setup

`strudeland skill install` writes this skill to `~/.claude/skills/strudeland/SKILL.md` (or under
`CLAUDE_CONFIG_DIR` when that is set);
`strudeland skill install --project=DIR` writes it under `DIR/.claude/skills/` instead (DIR
defaults to the current directory). It prints `installed`, `updated` or `unchanged` and the path.
Re-run it after upgrading the CLI so the skill matches the commands. `strudeland skill print`
shows the text the CLI carries.

While this skill is active it may run `strudeland` without asking. To allow that all the time, the
user can add this to `.claude/settings.json` (project) or `~/.claude/settings.json`:

```json
{ "permissions": { "allow": ["Bash(strudeland:*)"] } }
```
