// Micro case: supersaw onsets exactly on a quantum boundary, so `currentTime <= begin` is decided
// by the float32 rounding of begin. cps 2, so t = (slot / 2 + late) / 2 seconds:
//   first:  late 0.017414965986394557 cycles, t = 384 / 44100 = 0.008707482993197279 s (frame
//           384 = 128 * 3), f32(t) - t = +3.06e-10: the processor first runs one quantum later,
//           in the quantum starting at frame 512;
//   second: late -0.01818594104308391 cycles, t = 10624 / 44100 = 0.24090702947845805 s (frame
//           10624 = 128 * 83), f32(t) - t = -1.16e-09: it already runs in the quantum starting at
//           frame 10624.
// The first note (legato 0.5) has ended before the second starts. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('supersaw').unison(1).legato(0.5).late(0.017414965986394557)
$: note("~ e4").s('supersaw').unison(1).late(-0.01818594104308391)
