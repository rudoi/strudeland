// Micro case: vibrato (a sine oscillator at vib Hz through Gain(vibmod * 100) into the carrier's
// detune): the carrier is a-rate for its whole life. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sine').vib(8).vibmod(1)
$: note("~ e4").s('sawtooth').vib(5.5).vibmod(0.5)
