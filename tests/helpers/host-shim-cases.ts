// Each host shim (`src/host/shims.ts`) against the real function it replaces: over
// `note("c e").s("sine")`, equal hap `value` objects and `context.tags` (and, for `rawdsp`,
// `context.dominantSchedule`). The real ones run under happy-dom with a fake canvas context, a
// fixed scheduler time and no animation frames.
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
const fakeCtx = new Proxy(
  {},
  { get: (_t, k) => (k === 'canvas' ? { width: 1, height: 1 } : () => {}) },
);
const getContext = HTMLCanvasElement.prototype.getContext;
(HTMLCanvasElement.prototype as any).getContext = () => fakeCtx;
const raf = globalThis.requestAnimationFrame;
globalThis.requestAnimationFrame = (() => 0) as any;

const core = await import('@strudel/core');
const { importHostRealModules, snapshotOwnProperties } = await import('./prototype-snapshot');
const codemirror = await import('@strudel/codemirror');
await import('@strudel/webaudio');
await importHostRealModules();
const restorePrototype = snapshotOwnProperties(core.Pattern.prototype);
const { note } = core as any;
core.setTime(() => 0);

const SHIMMED = [
  '_scope',
  '_pianoroll',
  '_punchcard',
  '_spiral',
  '_pitchwheel',
  '_spectrum',
  'scope',
  'tscope',
  'fscope',
  'spectrum',
  'pianoroll',
  'pitchwheel',
  'rawdsp',
] as const;
const proto = core.Pattern.prototype as any;
const real: Record<string, (...a: any[]) => any> = Object.fromEntries(
  SHIMMED.map((n) => [n, proto[n]]),
);
const realMarkcssMethod = proto.markcss;

const base = () => note('c e').s('sine');
/** What the planner reads from a hap: its value, tags and schedule routing. */
const view = (pat: any) =>
  pat.queryArc(0, 2).map((h: any) => ({
    value: h.value,
    tags: h.context.tags,
    dominantSchedule: h.context.dominantSchedule,
  }));

const CASES: [name: string, args: unknown[]][] = [
  ['_scope', ['_widget__scope_0']],
  ['_scope', ['w', { pos: 0.2 }]],
  ['_pianoroll', ['_widget__pianoroll_0']],
  ['_pianoroll', ['w', { cycles: 2 }]],
  ['_punchcard', ['_widget__punchcard_0']],
  ['_spiral', ['_widget__spiral_0']],
  ['_spiral', ['w', { size: 100 }]],
  ['_pitchwheel', ['_widget__pitchwheel_0']],
  ['_spectrum', ['_widget__spectrum_0']],
  ['scope', []],
  ['scope', [{ id: 3 }]],
  ['tscope', [{ id: 'x' }]],
  ['fscope', []],
  ['fscope', [{ id: 2 }]],
  ['spectrum', []],
  ['spectrum', [{ id: 4 }]],
  ['pianoroll', []],
  ['pianoroll', [{ id: 5, cycles: 2 }]],
  ['pitchwheel', []],
  ['pitchwheel', [{ id: 6 }]],
  ['rawdsp', []],
];

// The real haps, computed before the shims replace anything (the real widget methods call the
// real `scope`, `pianoroll`, ... through the prototype).
const wants = CASES.map(([name, args]) => view(real[name]!.apply(base(), args)));

const { PATTERN_SHIMS, WIDGET_TYPES, shims, HOST_SLIDER_VALUES, installShims } =
  await import('../../src/host/shims');
// Explicitly: another test file may have imported (and so already run) the module, then restored
// the prototype.
installShims();

afterAll(() => {
  restorePrototype();
  core.setTime(undefined);
  (HTMLCanvasElement.prototype as any).getContext = getContext;
  globalThis.requestAnimationFrame = raf;
  GlobalRegistrator.unregister();
});

describe('pattern methods: shim haps equal the real ones', () => {
  CASES.forEach(([name, args], i) => {
    test(`${name}(${JSON.stringify(args).slice(1, -1)})`, () => {
      const want = wants[i];
      const got = view(PATTERN_SHIMS[name]!.apply(base(), args));
      expect(got).toEqual(want);
      expect(got.length).toBe(2);
    });
  });
});

test('every shimmed method has a case, and the shims are installed', () => {
  expect([...new Set(CASES.map(([n]) => n))].sort()).toEqual([...SHIMMED].sort());
  expect(Object.keys(PATTERN_SHIMS).sort()).toEqual([...SHIMMED].sort());
  for (const name of SHIMMED) {
    expect({ name, installed: proto[name] === PATTERN_SHIMS[name] }).toEqual({
      name,
      installed: true,
    });
    expect({ name, replaced: real[name] !== PATTERN_SHIMS[name] }).toEqual({
      name,
      replaced: true,
    });
  }
  expect(PATTERN_SHIMS.scope).toBe(PATTERN_SHIMS.tscope!);
});

test('rawdsp routes away from the default output, as the real one does', () => {
  const [hap] = PATTERN_SHIMS.rawdsp!.call(base()).queryArc(0, 1);
  expect(hap.context.dominantSchedule).toBe(1);
  expect(typeof hap.context.onSchedule).toBe('function');
  expect(base().queryArc(0, 1)[0].context.dominantSchedule).toBeUndefined();
});

test('widget types are registered in widget.mjs order', () => {
  expect([...WIDGET_TYPES]).toEqual([
    '_pianoroll',
    '_punchcard',
    '_spiral',
    '_scope',
    '_pitchwheel',
    '_spectrum',
  ]);
});

test('markcss: the same value key as codemirror', () => {
  const want = view(base().set(codemirror.markcss.markcss('color:red')));
  const got = view(base().set(shims.markcss.markcss('color:red')));
  expect(got).toEqual(want);
  expect(got[0].value.markcss).toBe('color:red');
  expect(view(realMarkcssMethod.call(base(), 'x'))).toEqual(view(proto.markcss.call(base(), 'x')));
});

test('slider: passes the value through with the same warning', () => {
  const warnings: unknown[][] = [];
  const warn = console.warn;
  console.warn = (...a: unknown[]) => void warnings.push(a);
  try {
    const want = view(base().gain(codemirror.slider(0.25)));
    const got = view(base().gain(shims.slider(0.25)));
    expect(got).toEqual(want);
    expect(got[0].value.gain).toBe(0.25);
  } finally {
    console.warn = warn;
  }
  expect(warnings).toHaveLength(2);
  expect(warnings[1]).toEqual(warnings[0]!);
});

test('sliderWithID: the value set at eval time is read at query time', () => {
  const want = view(base().gain(codemirror.sliderWithID('slider_12', 0.5, 0, 1)));
  const got = view(base().gain(shims.sliderWithID('slider_12', 0.5, 0, 1)));
  expect(got).toEqual(want);
  expect(got[1].value.gain).toBe(0.5);
  expect(HOST_SLIDER_VALUES.slider_12).toBe(0.5);
  expect(shims.sliderValues).toBe(HOST_SLIDER_VALUES);
  // A later write is what the next query reads (the page's slider moves; nothing does here).
  const pat = base().gain(shims.sliderWithID('slider_40', 0.1));
  HOST_SLIDER_VALUES.slider_40 = 0.9;
  expect(view(pat)[0].value.gain).toBe(0.9);
});
