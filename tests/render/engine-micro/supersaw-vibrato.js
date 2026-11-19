// Micro case: supersaw with vib(5).vibmod(1): the vibrato sine through Gain(100) is a connection
// into the worklet's detune param, summed with its timeline per frame. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('supersaw').vib(5).vibmod(1)
