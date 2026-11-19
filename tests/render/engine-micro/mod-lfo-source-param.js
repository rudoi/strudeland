// Micro case: lfos on the worklet synths' source params, read per sample by the processors:
// `pw` on a pulse maps to its pulsewidth ({ c: 'pw', r: 5, da: 0.2 }), `detune` on a supersaw to
// its freqspread ({ c: 'detune', r: 4, da: 0.1 }). The params are rendered from the scheduling
// frame on, so the lfos are pulled from then. legato(0.9) keeps the two pulse notes apart. One
// 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e4").s('pulse').legato(.9).pw(.3).lfo({ c: 'pw', r: 5, da: .2 })
$: note("a3").s('supersaw').unison(3).detune(.3).lfo({ c: 'detune', r: 4, da: .1 })
