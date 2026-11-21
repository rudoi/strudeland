# Implementation contracts

## Segmentation

`segment` runs before both grid resolution and key detection. Use decoded pitch
and the existing voicing-probability threshold when counting supporting frames.
A piece with no support yields no `RawNote`, even when a valid frequency remains
in the Viterbi track. One supported frame keeps a piece eligible, without an
amplitude gate or changes to its pitch, length, or onset logic. Leave debug
frames available, so users can see why a signal was rejected.

## Sustained notation

`quantize` keeps the per-bar `Note` records used by the CLI and the key scorer.
`render_notation` joins adjacent reported parts only when the earlier part is tied
and the following part is its contiguous same-pitch continuation. A genuine
repeat, any intervening silence, or an excluded continuation at the edge of the
requested span must preserve the appropriate boundary. Preserve the span's
first and last bars, including any visible pickup and trailing rest bars.
For ties, emit weighted Strudel mini-notation with a multi-cycle divisor and
verify actual Strudel onsets and lengths. Keep ordinary bar-group output for
untied material.

## Validation and privacy

Compare baseline and candidate for identical commands on the local review
recording, both at the default alignment and at the fitted alignment. Rate
including and excluding count-in artifacts, scoring intended pitch and onset
separately from duration. Count generated playback attacks as they actually
occur in Strudel. Keep fixture paths, audio and annotations out of commits.
Record synthetic accuracy and runtime to catch regressions.
