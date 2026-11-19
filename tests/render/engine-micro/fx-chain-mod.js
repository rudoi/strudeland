// Micro case: modulators addressing an FX chain entry. Hap 1's lfo sits inside the entry
// (`FX(lpf(500).lfo())`): its fxi defaults to the entry's index (0, the modulator-fxi-copy patch),
// so it targets the entry's lowpass. Hap 2's lfo is on the main value with fxi 0: it reaches the
// entry's lowpass from main. currentValue 500 >= 30 gives the frequency range. Two 0.5 s notes at
// once. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sawtooth').FX(lpf(500).lfo())
$: note("e3").s('sawtooth').FX(lpf(500)).lfo({ c: 'cutoff', fxi: 0 })
