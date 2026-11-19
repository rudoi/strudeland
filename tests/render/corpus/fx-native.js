// Corpus fixture: the native effects for the Rust engine: vowel, phaser, tremolo, a per-voice
// compressor, FXrelease and an orbit duck of two pads, each on its own orbit.
const meta = { title: 'fx-native', bpm: 120, meter: '4/4' };

$: note("<[a2 ~ e3 ~] [c3 ~ ~ ~]>").s('sawtooth').vowel("<a e i o u>").fxr(0.3).gain(0.15).orbit(1)
$: note("<c3 ~ [e3 g3] ~>").s('sawtooth').phaser(2).phaserdepth(0.6).phasercenter(1000).phasersweep(1500).gain(0.12).orbit(2)
$: note("<~ [a3 ~ ~ ~]>").s('triangle').tremolo(4).tremolodepth(0.8).tremoloshape("<sine tri square>").gain(0.2).orbit(3)
$: note("<[c2 c2 ~ c3] [~ g2 ~ ~]>").s('sawtooth').compressor("-20:12:6:0.01:0.1").gain(0.2).orbit(4)
$: note("<~ [e3 ~ ~ ~] ~ [~ ~ a3 ~]>").s('sawtooth').lpf(600).lpq(12).release(0.05).fxr(0.4).gain(0.15).orbit(5)
$: note("<[c1 ~ ~ ~ c1 ~ ~ ~] [c1 ~ ~ ~ ~ ~ c1 ~]>").s('sine').decay(0.2).sustain(0).gain(0.3).orbit(6).duckorbit("7:8").duckdepth(0.7).duckattack(0.2)
$: note("<e3 g3>").s('sawtooth').lpf(1200).gain(0.08).orbit(7)
$: note("<c4 b3>").s('sawtooth').lpf(1500).gain(0.06).orbit(8)
