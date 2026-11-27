import { buildStructure } from '../../src/lib/strudel/structure';

/** The `meta` of tests/render/mixed-meter-loop.js: a looped song whose meter changes move no bar. */
export const mixedMeterStructure = buildStructure({
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'opening', bars: 1 },
    { name: 'F1', bars: 3 },
    { name: 'F2', bars: 3 },
    { name: 'E', bars: 4 },
    { name: 'ending', bars: 1 },
  ],
  loop: { from: 'F1', through: 'E' },
  meterChanges: [
    { bar: 4, meter: '3/8' },
    { bar: 5, meter: '4/4' },
    { bar: 7, meter: '3/8' },
    { bar: 8, meter: '4/4' },
  ],
  loops: 2,
});
