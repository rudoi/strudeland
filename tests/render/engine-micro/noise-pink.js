// Micro case: pink noise (Kellet's filter, two float32 roundings per sample) from meta.seed 7:
// one buffer shared by both voices, an onset on a frame and one at 5512.5 frames. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', seed: 7, sections: [{ name: 'a', bars: 1 }] };

$: s("pink pink ~ ~").attack(0.01).release(0.05)
