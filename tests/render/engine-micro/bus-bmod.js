// Micro case: the patched bus modulator (bmod-gate). A sine (note 40) sends post to bus 2 with
// dry 0; a sawtooth's bmod({ b: 2, dc: 0.5 }) (default control cutoff) builds dc =
// ConstantSourceNode(0.5) started at begin, shifted = gainNode(1) summing dc then the bus,
// depthGain = gainNode(600 / 0.3) (currentValue 600, depth 1), the gate gainNode(0) open from
// begin to endWithRelease, down-mixed 0.5 L + 0.5 R into the lowpass frequency. The reader renders
// after its feeder (bus levels). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note(40).s('sine').bus(2).dry(0)
$: note("a2").s('sawtooth').lpf(600).bmod({ b: 2, dc: .5 })
