// Micro case: mod-lfo-cutoff with `ftype('24db')`: one lfo connected to the frequency of both
// biquads of the 24 dB lowpass (both elements of nodes.lpf). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(800).lfo({ r: 4, dep: 0.5 }).ftype('24db')
