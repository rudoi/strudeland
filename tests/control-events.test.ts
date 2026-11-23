import { expect, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import {
  barPosition,
  cleanValue,
  offsetToLineCol,
  toEvents,
  toLocations,
} from '../src/lib/control/events';
import { song } from '../src/lib/strudel/song';
import { buildStructure, type Meter } from '../src/lib/strudel/structure';
import { fakeRuntime } from './helpers/fake-runtime';

await core.evalScope(core, mini, { song });

const M44: Meter = { num: 4, den: 4 };

/** A hap-like object; `whole: null` makes one without a whole. */
function hap(
  begin: number,
  end: number,
  value: unknown,
  opts: { whole?: [number, number] | null; locations?: { start: number; end: number }[] } = {},
) {
  const whole =
    opts.whole === null
      ? undefined
      : { begin: opts.whole?.[0] ?? begin, end: opts.whole?.[1] ?? end };
  const part = { begin, end };
  return {
    whole,
    part,
    value,
    context: opts.locations ? { locations: opts.locations } : {},
    hasOnset: () => whole !== undefined && whole.begin === part.begin,
  };
}

const opts = (o: Partial<Parameters<typeof toEvents>[1]> = {}) => ({
  code: '',
  structure: null,
  meter: M44,
  all: false,
  limit: 2000,
  ...o,
});

// ---------------------------------------------------------------- offsetToLineCol

test('offsetToLineCol: multi-line code, 1-based', () => {
  const code = 'ab\ncde\n\nf';
  expect(offsetToLineCol(code, 0)).toEqual({ line: 1, col: 1 });
  expect(offsetToLineCol(code, 1)).toEqual({ line: 1, col: 2 });
  expect(offsetToLineCol(code, 2)).toEqual({ line: 1, col: 3 }); // the newline itself
  expect(offsetToLineCol(code, 3)).toEqual({ line: 2, col: 1 });
  expect(offsetToLineCol(code, 5)).toEqual({ line: 2, col: 3 });
  expect(offsetToLineCol(code, 7)).toEqual({ line: 3, col: 1 });
  expect(offsetToLineCol(code, 8)).toEqual({ line: 4, col: 1 });
});

test('offsetToLineCol: past the end and negative clamp; empty code', () => {
  const code = 'ab\ncd';
  expect(offsetToLineCol(code, 5)).toEqual({ line: 2, col: 3 });
  expect(offsetToLineCol(code, 99)).toEqual({ line: 2, col: 3 });
  expect(offsetToLineCol(code, -4)).toEqual({ line: 1, col: 1 });
  expect(offsetToLineCol('', 3)).toEqual({ line: 1, col: 1 });
});

test('offsetToLineCol: CRLF splits on the LF, the CR stays in the line', () => {
  const code = 'ab\r\ncd';
  expect(offsetToLineCol(code, 2)).toEqual({ line: 1, col: 3 });
  expect(offsetToLineCol(code, 4)).toEqual({ line: 2, col: 1 });
  expect(toLocations([{ start: 0, end: 2 }], code)[0]!.text).toBe('ab');
});

// ---------------------------------------------------------------- toLocations

test('toLocations: sorted by position, whatever order Strudel combined them in', () => {
  const code = 'a\n$: s("bd").lpf("800")';
  const bd = code.indexOf('bd');
  const lpf = code.indexOf('800');
  const locs = toLocations(
    [
      { start: lpf, end: lpf + 3 },
      { start: bd, end: bd + 2 },
      { start: 0, end: 1 },
    ],
    code,
  );
  expect(locs.map((l) => [l.line, l.col])).toEqual([
    [1, 1],
    [2, 7],
    [2, 17],
  ]);
});

test('toLocations: positions, exclusive end, and the trimmed line text', () => {
  const code = 'const x = 1;\n   $: s("bd hh")   \n';
  const bd = code.indexOf('bd');
  expect(toLocations([{ start: bd, end: bd + 2 }], code)).toEqual([
    { line: 2, col: 10, endLine: 2, endCol: 12, text: '$: s("bd hh")' },
  ]);
});

test('toLocations: a location spanning lines', () => {
  const code = 'a\nbc\nd';
  expect(toLocations([{ start: 1, end: 4 }], code)).toEqual([
    { line: 1, col: 2, endLine: 2, endCol: 3, text: 'a' },
  ]);
});

test('toLocations: duplicates removed, sorted, undefined and malformed input', () => {
  const code = 's("a b")';
  const locs = toLocations(
    [
      { start: 3, end: 4 },
      { start: 5, end: 6 },
      { start: 3, end: 4 },
      { start: 3, end: 5 },
    ],
    code,
  );
  expect(locs.map((l) => [l.col, l.endCol])).toEqual([
    [4, 5],
    [4, 6],
    [6, 7],
  ]);
  expect(toLocations(undefined, code)).toEqual([]);
  expect(toLocations([null as any, { start: 'x' } as any], code)).toEqual([]);
});

test('toLocations: text cut to 120 characters', () => {
  const line = 'x'.repeat(200);
  const [loc] = toLocations([{ start: 150, end: 151 }], `  ${line}`);
  expect(loc!.text).toBe('x'.repeat(120));
  expect(loc!.col).toBe(151);
});

// ---------------------------------------------------------------- onsets

test('onsets only by default; all keeps continuations and wholeless haps', () => {
  const haps = [
    hap(0, 1, { s: 'a' }),
    hap(1, 1.5, { s: 'cont' }, { whole: [0.5, 1.5] }),
    hap(2, 3, { s: 'signal' }, { whole: null }),
  ];
  const onsets = toEvents(haps, opts());
  expect(onsets.events.map((e) => e.value.s)).toEqual(['a']);
  expect(onsets.events[0]!.onset).toBe(true);

  const all = toEvents(haps, opts({ all: true })).events;
  expect(all.map((e) => [e.value.s, e.onset, e.begin, e.end])).toEqual([
    ['a', true, 0, 1],
    ['cont', false, 0.5, 1.5], // begin/end from the whole
    ['signal', false, 2, 3], // no whole: from the part
  ]);
});

test('begin and end go through Number() (fraction-like values)', () => {
  const frac = (n: number) => ({ valueOf: () => n });
  const h = {
    whole: { begin: frac(0.25), end: frac(0.5) },
    part: { begin: frac(0.25), end: frac(0.5) },
    value: { s: 'x' },
    context: {},
    hasOnset: () => true,
  };
  const [e] = toEvents([h], opts()).events;
  expect([e!.begin, e!.end, e!.bar, e!.beat]).toEqual([0.25, 0.5, 1, 2]);
});

// ---------------------------------------------------------------- bar and beat

test('bar and beat: 4/4, 3/4, 7/8, fractional beats', () => {
  const at = (begin: number, meter: Meter) => {
    const [e] = toEvents([hap(begin, begin + 0.1, {})], opts({ meter })).events;
    return [e!.bar, e!.beat];
  };
  expect(at(0, M44)).toEqual([1, 1]);
  expect(at(8.5, M44)).toEqual([9, 3]);
  expect(at(2.875, M44)).toEqual([3, 4.5]);
  expect(at(1 + 2 / 3, { num: 3, den: 4 })[0]).toBe(2);
  expect(at(1 + 2 / 3, { num: 3, den: 4 })[1]).toBeCloseTo(3);
  expect(at(0.5, { num: 3, den: 4 })).toEqual([1, 2.5]);
  expect(at(3 + 6 / 7, { num: 7, den: 8 })[0]).toBe(4);
  expect(at(3 + 6 / 7, { num: 7, den: 8 })[1]).toBeCloseTo(7);
  expect(at(0.25, { num: 7, den: 8 })).toEqual([1, 2.75]);
});

// ---------------------------------------------------------------- section

const STRUCTURE = buildStructure({
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
});

test('section and bar within it; null past the end and without a structure', () => {
  const haps = [
    hap(0, 1, {}),
    hap(1.5, 2, {}),
    hap(2, 3, {}),
    hap(5.25, 6, {}),
    hap(7, 8, {}),
    hap(8, 9, {}),
  ];
  const sections = toEvents(haps, opts({ structure: STRUCTURE })).events.map((e) => e.section);
  expect(sections).toEqual([
    { name: 'intro', bar: 1 },
    { name: 'intro', bar: 2 },
    { name: 'body', bar: 1 },
    { name: 'body', bar: 4 },
    { name: 'tail', bar: 2 },
    null,
  ]);
  expect(toEvents(haps, opts()).events.every((e) => e.section === null)).toBe(true);
});

test('a continuation that began before bar 1: bar 0 or less, no section', () => {
  const [e] = toEvents(
    [hap(0, 0.5, {}, { whole: [-2, 0.5] })],
    opts({ structure: STRUCTURE, all: true }),
  ).events;
  expect(e!.onset).toBe(false);
  expect(e!.bar).toBe(-1);
  expect(e!.section).toBeNull();
});

test('section of an empty structure is null', () => {
  const [e] = toEvents([hap(0, 1, {})], opts({ structure: buildStructure({}) })).events;
  expect(e!.section).toBeNull();
});

// ---------------------------------------------------------------- sort and limit

test('sort: begin, then line and col, located before unlocated, then input order', () => {
  const code = 'aaaa\nbbbb\n';
  const haps = [
    hap(1, 2, { s: 'late' }),
    hap(0, 1, { s: 'none-1' }),
    hap(0, 1, { s: 'line2' }, { locations: [{ start: 6, end: 7 }] }),
    hap(0, 1, { s: 'line1-col3' }, { locations: [{ start: 2, end: 3 }] }),
    hap(0, 1, { s: 'none-2' }),
    hap(0, 1, { s: 'line1-col1' }, { locations: [{ start: 0, end: 1 }] }),
  ];
  expect(toEvents(haps, opts({ code })).events.map((e) => e.value.s)).toEqual([
    'line1-col1',
    'line1-col3',
    'line2',
    'none-1',
    'none-2',
    'late',
  ]);
});

test('limit keeps the first events after sorting; equal to the limit is not truncated', () => {
  const haps = [hap(2, 3, { n: 2 }), hap(0, 1, { n: 0 }), hap(1, 2, { n: 1 })];
  const two = toEvents(haps, opts({ limit: 2 }));
  expect(two.events.map((e) => e.value.n)).toEqual([0, 1]);
  expect(two.truncated).toBe(true);
  const three = toEvents(haps, opts({ limit: 3 }));
  expect(three.events.length).toBe(3);
  expect(three.truncated).toBe(false);
  expect(toEvents([], opts({ limit: 1 }))).toEqual({ events: [], truncated: false });
});

// ---------------------------------------------------------------- value cleanup

test('value cleanup: _ keys, functions, undefined and non-JSON dropped; nested kept if JSON', () => {
  class Odd {
    x = 1;
  }
  const value = {
    s: 'bd',
    note: 60,
    gain: 0.5,
    on: true,
    nil: null,
    _private: 1,
    fn: () => 1,
    undef: undefined,
    nan: NaN,
    inf: Infinity,
    arr: [1, 'a', null, false],
    badArr: [1, () => 2],
    nested: { a: 1, b: ['x'] },
    badNested: { a: 1, f: () => 1 },
    instance: new Odd(),
  };
  expect(cleanValue(value)).toEqual({
    s: 'bd',
    note: 60,
    gain: 0.5,
    on: true,
    nil: null,
    arr: [1, 'a', null, false],
    nested: { a: 1, b: ['x'] },
  });
});

test('value cleanup: a non-object value is wrapped', () => {
  expect(cleanValue(3)).toEqual({ value: 3 });
  expect(cleanValue('c4')).toEqual({ value: 'c4' });
  expect(cleanValue([1, 2])).toEqual({ value: [1, 2] });
  expect(cleanValue(null)).toEqual({ value: null });
  expect(cleanValue(undefined)).toEqual({});
  expect(cleanValue(() => 1)).toEqual({});
  const [e] = toEvents([hap(0, 1, 7)], opts()).events;
  expect(e!.value).toEqual({ value: 7 });
});

// ---------------------------------------------------------------- real haps

const META = `const meta = { bpm: 120, sections: [{ name: 'intro', bars: 2 }, { name: 'body', bars: 2 }] };`;

test('real haps: line and col point at the mini-notation tokens that made them', async () => {
  const { rt } = fakeRuntime({ running: false });
  const code = `${META}
$: song(meta, {
  intro: s("bd <hh oh>"),
  body: note("c e").s('sine'),
})
`;
  expect(await rt.evaluate(code)).toBe(true);
  let state: any;
  rt.store.subscribe((s) => (state = s))();
  expect(state.activeCode).toBe(code);
  const { events, truncated } = toEvents(rt.query(0, 4)!, {
    code: state.activeCode,
    structure: state.structure,
    meter: state.meter,
    all: false,
    limit: 2000,
  });
  expect(truncated).toBe(false);
  const rows = events.map((e) => [
    e.bar,
    e.beat,
    e.section,
    e.value.s,
    e.value.note,
    e.locations.map((l) => [l.line, l.col, l.endLine, l.endCol]),
  ]);
  const introLine = 3;
  const col = (token: string) => code.split('\n')[introLine - 1]!.indexOf(token) + 1;
  const bodyLine = 4;
  const bcol = (token: string) => code.split('\n')[bodyLine - 1]!.indexOf(token) + 1;
  expect(rows).toEqual([
    [
      1,
      1,
      { name: 'intro', bar: 1 },
      'bd',
      undefined,
      [[introLine, col('bd'), introLine, col('bd') + 2]],
    ],
    [
      1,
      3,
      { name: 'intro', bar: 1 },
      'hh',
      undefined,
      [[introLine, col('hh'), introLine, col('hh') + 2]],
    ],
    [
      2,
      1,
      { name: 'intro', bar: 2 },
      'bd',
      undefined,
      [[introLine, col('bd'), introLine, col('bd') + 2]],
    ],
    [
      2,
      3,
      { name: 'intro', bar: 2 },
      'oh',
      undefined,
      [[introLine, col('oh'), introLine, col('oh') + 2]],
    ],
    // `'sine'` is single-quoted: no location of its own, only the note's.
    [
      3,
      1,
      { name: 'body', bar: 1 },
      'sine',
      'c',
      [[bodyLine, bcol('c e'), bodyLine, bcol('c e') + 1]],
    ],
    [
      3,
      3,
      { name: 'body', bar: 1 },
      'sine',
      'e',
      [[bodyLine, bcol('c e') + 2, bodyLine, bcol('c e') + 3]],
    ],
    [
      4,
      1,
      { name: 'body', bar: 2 },
      'sine',
      'c',
      [[bodyLine, bcol('c e'), bodyLine, bcol('c e') + 1]],
    ],
    [
      4,
      3,
      { name: 'body', bar: 2 },
      'sine',
      'e',
      [[bodyLine, bcol('c e') + 2, bodyLine, bcol('c e') + 3]],
    ],
  ]);
  expect(events[0]!.locations[0]!.text).toBe('intro: s("bd <hh oh>"),');
});

test('real haps: a plain pattern has no sections and numeric values keep their type', async () => {
  const { rt } = fakeRuntime({ running: false });
  const code = `$: n("0 2").s("piano")`;
  expect(await rt.evaluate(code)).toBe(true);
  let state: any;
  rt.store.subscribe((s) => (state = s))();
  const { events } = toEvents(rt.query(1, 2)!, {
    code,
    structure: state.structure,
    meter: state.meter,
    all: false,
    limit: 10,
  });
  expect(events.map((e) => [e.bar, e.beat, e.section, e.value])).toEqual([
    [2, 1, null, { n: 0, s: 'piano' }],
    [2, 3, null, { n: 2, s: 'piano' }],
  ]);
  // Both strings are double-quoted: each event carries the n token and the s token, in source order.
  expect(events[0]!.locations.map((l) => code.slice(l.col - 1, l.endCol - 1))).toEqual([
    '0',
    'piano',
  ]);
});

test('barPosition floors fractional song and local bars to 1-based whole bars', () => {
  const structure = buildStructure({
    bpm: 120,
    meter: '4/4',
    sections: [
      { name: 'intro', bars: 2 },
      { name: 'body', bars: 4 },
    ],
  });
  expect(barPosition({ section: 1, localCycle: 2.75, songBar: 4.75 }, structure)).toEqual({
    songBar: 5,
    section: { name: 'body', bar: 3 },
  });
  expect(barPosition({ section: 0, localCycle: 0, songBar: 0 }, structure)).toEqual({
    songBar: 1,
    section: { name: 'intro', bar: 1 },
  });
  expect(barPosition({ section: 2, localCycle: 0, songBar: 6 }, structure)).toBeNull();
});

test('event beats use the meter of their authored bar', () => {
  const structure = buildStructure({
    sections: [{ name: 'body', bars: 3 }],
    meterChanges: [
      { bar: 2, meter: '7/8' },
      { bar: 3, meter: '3/4' },
    ],
  });
  const { events } = toEvents(
    [hap(0.5, 1, 'a'), hap(1.5, 2, 'b'), hap(2.5, 3, 'c')],
    opts({ structure }),
  );
  expect(events.map(({ bar, beat, section }) => [bar, beat, section?.name])).toEqual([
    [1, 3, 'body'],
    [2, 4.5, 'body'],
    [3, 2.5, 'body'],
  ]);
});
