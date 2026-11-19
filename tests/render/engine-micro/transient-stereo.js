// Micro case: transient(0.5) on a pulse: the transient processor on two channels, channel-major,
// with avgGain shared and carried from channel 0 into channel 1 within a block. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('pulse').transient(0.5)
