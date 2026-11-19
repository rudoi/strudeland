// Micro case: a bandpass filter envelope (bpenv with its ADSR). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 ~ e3 ~").s('sawtooth').bpf(400).bpq(3).bpenv(3).bpattack(0.03).bpdecay(0.1).bpsustain(0.2).bprelease(0.05)
