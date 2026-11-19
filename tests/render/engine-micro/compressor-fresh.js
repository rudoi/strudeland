// Micro case: a compressor created after the warm-up window. The only voice is a loud sawtooth in
// bar 7, whose pause (1.75 s, frame 77184) is past the 71936-frame boundary of Blink's silence
// rule (last_non_silent_time 0 + latency 264/44100 + tail 1.625 s): the node never processed
// silence, so its onset state is fresh (detector 0, the gain dive), then superdough's defaults
// (ratio 10, knee 10, attack 0.005, release 0.05) with threshold -20, the 264-frame pre-delay,
// the makeup gain, the 32-frame divisions and the adaptive release. Eight 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 8 }] };

$: note("<~ ~ ~ ~ ~ ~ ~ a2>").s('sawtooth').compressor(-20)
