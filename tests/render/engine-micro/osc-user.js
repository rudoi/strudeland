// Micro case: `s('user')` with a magnitude list (sine partials, terms [0, 1]) and without partials
// (superdough falls back to the triangle). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~ a5 ~").s('user').partials([1, 0.3, 0, 0.6, 0.1])
$: note("~ c4 ~ ~").s('user')
