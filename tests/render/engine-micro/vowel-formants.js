// Micro case (measured floor): the vowel bank, five bandpass formants (frequencies, Qs and gains
// of vowel.mjs) summed into the makeup gain 8, for the vowels a and i, both held past the render
// end. Chromium sums the five formant outputs in hash-set order, so its reference varies in the
// last bit from run to run. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e3").s('sawtooth').vowel("a i").clip(8)
