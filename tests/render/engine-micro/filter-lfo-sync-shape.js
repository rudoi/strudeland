// Micro case: the other filter LFO keys: lpsync(2) is rate = cps * 2, lpshape('sine') the shape
// name, lpskew(0.3) the skew (unused by sine) and lpdc(-0.25) the dcoffset; depth defaults to 1
// (modDepth = 800). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(800).lpsync(2).lpshape('sine').lpskew(.3).lpdc(-.25)
