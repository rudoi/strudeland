// Micro case: an FX chain entry's own gain stage. `.FX(lpf(500))` runs the entry's stages before
// the main ones: its gain stage is gainNode(0.8) (the default gain, then velocity 1), then its
// lowpass; the main entry adds its own gain stage (0.8 again). Hap 2's entry sets gain(1) and the
// hap's ftype('24db') doubles the entry's lowpass (the 24 dB doubling reads the hap value, not the
// entry). Two 0.5 s notes at once. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sawtooth').FX(lpf(500))
$: note("e3").s('sawtooth').FX(lpf(500).gain(1)).ftype('24db')
