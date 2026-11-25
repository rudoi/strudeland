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

One recording of a voice memo, from count-in until it is stopped and saved, possibly with the song
or a metronome playing as backing.
_Avoid_: recording (for the act), session

**Audible memo**

A voice memo that is loading or playing back, with or without song backing.
_Avoid_: playing memo, memo playback (for the state)
