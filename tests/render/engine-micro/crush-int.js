// Micro case: crush(4) lands on exact halves: Math.round rounds half toward +infinity. A constant
// source through Gain(0.3), the sustain-1 envelope and the gain stage stores exactly +0.0625 and
// -0.0625 at sustain (f32 products 1 * 0.3 * 1 * g), so in * 8 is exactly 0.5 (giving 1/8) and
// -0.5 (giving -0). g = 0.2083333283662796 is the only float32 with f32(f32(0.3) * g) = 0.0625,
// found by stepping float32 values around 0.0625 / f32(0.3) with numpy (scratch script):
//   p = f32(1) * f32(0.3); for g near f32(0.0625 / p): keep g if f32(p * g) == 0.0625
// One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one one").sustain(1).gain("0.2083333283662796 -0.2083333283662796").crush(4)
