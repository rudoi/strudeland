// Micro case: the sine oscillator at low, mid and high pitch (55, 440, 4186 Hz) with the default
// amp envelope. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a1 a4 c8").s('sine')
