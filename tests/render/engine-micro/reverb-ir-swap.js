// Micro case: IR swaps at pause frames. Every bar asks for a new roomsize; each new IR lands at
// its bar's pause frame, cutting the ringing tail and the input history of the previous one (the
// 0.3 s room of bar 0 is cut at bar 1). Four 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 4 }] };

$: note("<a3 c4 e3 g3>").s('sawtooth').decay(0.08).sustain(0).room(0.7).roomsize("<0.3 0.1 0.2 0.05>").roomlp(0)
