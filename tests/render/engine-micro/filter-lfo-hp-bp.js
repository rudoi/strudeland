// Micro case: the highpass and bandpass filter LFO keys: hprate(3).hpdepth(0.5) on hpf(200)
// (depth 100, min -170, max 19800) and bprate(5).bpdepth(0.3) on bandf(1000) (depth 300, min
// -970, max 19000), one voice each. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2").s('sawtooth').hpf(200).hprate(3).hpdepth(.5)
$: note("a3").s('sawtooth').bandf(1000).bprate(5).bpdepth(.3)
