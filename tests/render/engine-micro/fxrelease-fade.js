// Micro case: FXrelease. Hap 1 (FXrelease 0.3 > release 0.01): the release gain holds 1 until
// end + release (setValueAtTime) then ramps linearly to 0 at end + 0.3, fading the resonant
// lowpass ring (superdough's cleanup waits for endWithRelease, after the render). Hap 2
// (FXrelease 0.005 <= release) builds no release gain. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a3").s('sawtooth').lpf(800).lpq(25).release(0.01).FXrelease("0.3 0.005")
