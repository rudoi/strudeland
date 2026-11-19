// Micro case: the orbit djf as a lowpass at (0.4 * 11)^4 = 374.8 Hz and a highpass at
// (0.6 * 11)^4 = 1897.5 Hz, switching per bar on the value read at each quantum's first frame,
// both channels, the filter state kept across the switches. One sawtooth sounds through the whole
// window (clip); the djf values come from silent carrier haps (gain 0) on the same orbit, so one
// input sounds at the summing node and no voice ends inside the window. Four 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 4 }] };

$: note("<a2 ~ ~ ~>").s('sawtooth').clip(8)
$: s("one").gain(0).djf("<0.2 0.8 0.2 0.8>")
