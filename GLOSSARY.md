# Strudeland

Language for writing, arranging, and inspecting Strudel songs.

## Pattern editing

**Shared definition**

A named pattern in a song's source whose notes can be reused at multiple use sites. Editing the
definition changes every use that reads it.

**Use site**

A place in the song's source where a defined pattern is referenced, either directly or through a
transformation.

**Pattern occurrence**

One instance of a pattern at a particular written position in the arrangement. A use site can
produce several occurrences.

**Meter anchor**

The written bar where a piano roll window takes its starting time signature. Subsequent bars
take the time signatures of their corresponding written positions.

## Song structure

**Song**

One plain Strudel script in a project, played and rendered as a whole piece.
_Avoid_: pattern file, track

**Song header**

The `const meta = { … }` statement a song opens with: literal values naming its title, tempo,
meter, sections, loop and seed, read without evaluating the song.
_Avoid_: meta header, meta block, metadata

**Structure**

The bars, sections, loop and tempo a song header describes.
_Avoid_: arrangement (the written order of patterns), layout (the transport's own term)

**Section**

A named run of whole bars in a song's structure, such as `intro` or `chorus`.
_Avoid_: part (the pattern a section plays), block

**Playing song**

The song whose last successful evaluate is sounding, while playback is started.
_Avoid_: evaluated song (in UI copy), current song

**Armed section**

A section a stopped transport will start from or loop when playback starts, named on the song it
was armed for.
_Avoid_: armed index, pending start

**Take**

One recording of a voice memo, from the record click (starting) until it is stopped and saved,
possibly with the song or a metronome playing as backing.
_Avoid_: recording (for the act), session

**Audible memo**

A voice memo that is loading or playing back, with or without song backing.
_Avoid_: playing memo, memo playback (for the state)

## Audio export

**Export request**

What one export renders: the song, its length request, and either a mix (with an optional
preset and its output options) or a stem set (with its grouping).
_Avoid_: render job, export plan

**Destination**

Where an export's files go: an explicit path, a chosen folder, or a project's `renders/`. The
same destination names a mix's files and a stem set's directory differently.
_Avoid_: target (one resolved file), output

**Role**

What one file of a mix export is for: `single`, `intro` or `loop`. A preset fixes its roles
and their order.
_Avoid_: part, kind

**Stem set**

One directory holding a pre-master reference mix, a float WAV per group and a `stems.json`
manifest, rendered together and never replaced.
_Avoid_: stems folder, stem export (the action)

**Finishing**

Turning a render's PCM into finished files: leveling each file cut, encoding it, and for a lossy
format decoding it again and correcting an overshoot, at most twice. Every render, plain,
one-shot or loop preset, ends in the same finishing loop.
_Avoid_: mastering (the user's own step), post-processing

**Leveling strategy**

How finishing sets a render's level. *Limited* (plain renders and one-shots): a static gain to
the loudness target, then the true-peak limiter; an overshoot lowers the limiter's ceiling.
*Repeat-safe static* (loop presets): one static gain shared by every file, capped for the
handoff and the wrap, with no limiter; an overshoot lowers the shared gain.
_Avoid_: mode (a render's audio mode), profile (a render-harness gate set)

**File cut**

The PCM of one file a render will finish, with its role, the bars it covers and its loop range
in input frames. A plain render or a one-shot is one cut; a loop splice yields one or two.
_Avoid_: part (a section's pattern), chunk (one planned engine job), segment (the transport's)

**Loop splice**

The continuous opening-and-repeat PCM a loop preset is cut from: the repeat's decay folded into
one loop body, the opening's last 10 ms blended into it, and the true peak measured across the
handoff and the wrap.
_Avoid_: arrangement (the written order of patterns), loop layout (layout is the transport's)

## App boundary

**Command error**

What a fallible Tauri command rejects with: a kind from one flat list, such as `projectChanged`,
`notFound` or `stale`, and the message the user sees. Each webview adapter narrows the kind to
the ones its feature handles; any other kind is `other`.
_Avoid_: song error, recording error, playback error (the old per-feature shapes), error string

**Stale root**

A command naming a project `root` that is no longer the open one, because another project was
opened meanwhile. It fails with kind `projectChanged` and touches nothing.
_Avoid_: the project changed (the message, not the condition), sentinel

**IPC bindings**

The generated `src/lib/ipc/bindings.ts`: every command's name and argument keys, every event
name and every command error kind, read from the Rust source by `bun run ipc:build`.
_Avoid_: IPC types (the payload types stay hand-written), protocol (the control socket's)

**Wire example**

One committed JSON example of a control method (`crates/strudeland-ctl/tests/wire/`): params
both sides accept, named results, and the state-independent refusals with their exact code
and message. Rust, the webview handler, the CLI and the app test against the same examples.
_Avoid_: fixture (any test input), golden (the render harness's), schema (nothing is
generated from them)
