// Micro case: a constant 12 dB lowpass (BiquadFilterNode, Q in dB, RBJ coefficients with the
// f32-precision resonance) on sawtooths: after the pause quantum the filter runs the macOS fast
// path (vDSP_deq22D, double state). The square sits inside the third sawtooth, clear of note
// boundaries, so at most two voices sound at once. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3 a3 c4").s('sawtooth').lpf(800).lpq(5)
$: note("~@9 e4@2 ~@5").s('square').lpf(3000)
