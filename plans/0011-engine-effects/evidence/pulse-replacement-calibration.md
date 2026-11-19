# Pulse oscillator calibration

The browser and Rust implementations use the same four-point edge correction with two samples
of oscillator delay. The correction uses fractional crossings and applies to both edges,
including when pulse width changes during the note.

Chromium rendered each of the following micro cases bit-identically across repeats. Each Rust
render matched its Chromium reference exactly. The cases retain their existing −140 dB maximum
residual threshold.

| Case | Coverage |
|---|---|
| `pulse-basic` | Stereo pulse on two notes |
| `pulse-fm-vibrato` | Pitch envelope, vibrato and FM |
| `pulse-pan-center` | Center pan with stereo and mono sources |
| `pulse-pw` | Pulse widths of 0.1 and 0.9 |
| `pulse-pwm` | Audio-rate pulse-width modulation |
| `mod-lfo-source-param` | LFOs on pulse width and supersaw detune |
| `transient-stereo` | Transient processing on a stereo pulse source |

The canonical [engine references](../../../crates/strudeland-engine/tests/refs/)
contain the regenerated jobs and WAVs. The current `voice-worklets` fixture passed its mix and
twin gates in both engines. Its regenerated
[superdough golden](../../../tests/render/goldens/voice-worklets.json) keeps the same frame
count and peak.
