// Micro case: a highpass filter envelope (hpenv, hpattack, hpdecay, hpsustain). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').hpf(200).hpq(2).hpenv(3).hpattack(0.1).hpdecay(0.05).hpsustain(0.5)
