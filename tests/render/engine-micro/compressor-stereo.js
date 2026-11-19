// Micro case: the compressor always outputs two channels (a mono input feeds both pre-delay
// buffers). Bar 0: an unpanned compressed sawtooth sends a stereo signal, which turns the orbit
// delay stereo; bar 1: a compressed sawtooth with pan(0.5), which takes the panner's stereo law
// (no -3 dB centre). Only the delay returns are heard (dry 0). Four 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 4 }] };

$: note("<[e3 ~] ~ ~ ~>").s('sawtooth').compressor(-20).dry(0).delay(0.5).delaytime(0.03)
$: note("<~ [a3 ~] ~ ~>").s('sawtooth').compressor(-20).pan(0.5).dry(0).delay(0.5).delaytime(0.03)
