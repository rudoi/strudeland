// Micro case: crackle, regenerated per voice from its voice seed (randomFromSeed), at
// density 0.02 (the default value), 2 and 30 (the hit probability is density * 0.01). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("crackle crackle crackle ~").density("0.02 2 30 0").gain(2)
