// Micro case: a linear pitch envelope on the oscillator's detune (penv 12 semitones, anchor =
// psustain): a-rate detune, exp2 per frame. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a4").s('sawtooth').penv(12).pattack(0.05).pdecay(0.1).psustain(0.3).prelease(0.05)
