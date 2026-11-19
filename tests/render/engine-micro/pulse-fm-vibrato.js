// Micro case: the pitch envelope, vibrato and FM wiring on a worklet source: penv on the pulse's
// detune param, vib(5).vibmod(1) as a connection into detune, fmi(2) as a connection into
// frequency. The note starts half way through the bar: the FM modulator starts at the pause frame
// and is pulled by the worklet's param rendering every quantum from there, not from the onset.
// One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("~ a3").s('pulse').penv(-5).pattack(0.08).vib(5).vibmod(1).fmi(2)
