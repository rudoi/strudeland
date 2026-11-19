// Micro case: supersaw with unison(5).spread(0.6).detune(0.3): the detuner (n * 0.3 / 4 - 0.15
// semitones), the sqrt pan gains swapped from voice to voice, five phases drawn in voice order and
// the float32 += accumulation of the voices into the output. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('supersaw').unison(5).spread(0.6).detune(0.3)
