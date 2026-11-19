// Corpus fixture: FX chains for the Rust engine: two entries, local delay and reverb with fxr, pan
// inside an entry, per-entry gain and velocity beside a plain voice, and an lfo inside an entry.
const meta = { title: 'fx-chain', bpm: 120, meter: '4/4' };

$: note("<[a2 ~ e3 ~] [c3 ~ ~ ~]>").s('sawtooth').FX(lpf(500)).FX(hpf(200)).gain(0.2).orbit(1)
$: note("<[c4 ~ ~ ~] ~ [~ ~ g3 ~] ~>").s('sawtooth').clip(0.3).FX(lpf(800).delay(0.5).delaytime(0.2).delayfeedback(0.4)).fxr(0.5).gain(0.2).orbit(2)
$: note("<~ [e4 ~ ~ ~] ~ [~ a4 ~ ~]>").s('triangle').clip(0.3).FX(room(0.6).roomsize(2)).fxr(0.8).gain(0.3).orbit(3)
$: note("<[~ c3 ~ e3] ~>").s('square').lpf(1500).FX(pan(0.2)).gain(0.12).orbit(4)
$: note("<~ [a3 ~ c4 ~]>").s('sawtooth').FX(gain(0.6).velocity(0.5)).gain(0.2).orbit(5)
$: note("<~ [~ e3 ~ g3]>").s('sawtooth').gain(0.1).orbit(5)
$: note("<[a2 ~ ~ ~] [~ ~ e2 ~]>").s('sawtooth').FX(lpf(500).lfo()).gain(0.2).orbit(6)
