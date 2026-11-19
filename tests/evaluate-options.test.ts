import { expect, test } from 'bun:test';
import {
  EMPTY_STRUCTURE,
  evaluateOptions,
  staticStructure,
} from '../src/lib/project/evaluate-options';
import type { Meta } from '../src/lib/project/meta';
import { cpsFor, DEFAULT_METER } from '../src/lib/strudel/structure';

test('no header gives the default tempo and no seed', () => {
  expect(evaluateOptions({})).toEqual({
    cps: 0.5,
    meter: DEFAULT_METER,
    seed: undefined,
    engine: 'superdough',
  });
});

test('bpm, meter and seed come from the header', () => {
  const o = evaluateOptions({ bpm: 90, meter: '6/8', seed: 7 } as Meta);
  expect(o.cps).toBe(cpsFor(90, o.meter!));
  expect(o.meter).toEqual({ num: 6, den: 8 });
  expect(o.seed).toBe(7);
});

test('an invalid header falls back to the empty structure and never throws', () => {
  const bad = { bpm: -5 } as unknown as Meta;
  expect(staticStructure(bad)).toBe(EMPTY_STRUCTURE);
  expect(evaluateOptions(bad)).toEqual({
    cps: 0.5,
    meter: DEFAULT_METER,
    seed: undefined,
    engine: 'superdough',
  });
});

test('the engine comes from the header, superdough by default', () => {
  expect(evaluateOptions({ engine: 'rust' } as Meta).engine).toBe('rust');
  expect(evaluateOptions({ engine: 'superdough' } as Meta).engine).toBe('superdough');
  expect(evaluateOptions({}).engine).toBe('superdough');
});
