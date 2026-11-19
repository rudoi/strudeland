// Micro case: sustain only (0.3); attack and decay at their 0.001 minimums, so the envelope
// drops to 0.3 within 2 ms and holds. One held sine note in one 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sine').sustain(0.3)
