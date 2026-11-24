import { expect, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import { get } from 'svelte/store';
import {
  createInstrumentTranspiler,
  registerInstrumentOperation,
  type InstrumentSnapshot,
} from '../src/lib/strudel/instruments';
import { song } from '../src/lib/strudel/song';
import { fakeRuntime } from './helpers/fake-runtime';

const lead = (cutoff: number): InstrumentSnapshot => ({
  lead: {
    schema: 1,
    parameters: { cutoff },
    voices: [
      {
        name: 'lead',
        chain: [
          { control: 's', value: 'sawtooth' },
          { control: 'lpf', value: 100 },
          { control: 'lpf', value: { param: 'cutoff' } },
        ],
      },
    ],
  },
});

const kit: InstrumentSnapshot = {
  drums: {
    schema: 1,
    parameters: {},
    voices: [
      { name: 'kick', chain: [{ control: 's', value: 'sine' }] },
      { name: 'hat', chain: [{ control: 's', value: 'white' }] },
    ],
  },
};

const prebake = async () => {
  await core.evalScope(core, mini, { song });
  registerInstrumentOperation();
};

test('applies ordered controls with defaults and overrides without adding song offsets', async () => {
  let locations: number[][] = [];
  const { rt } = fakeRuntime({
    prebake,
    hooks: { afterEval: ({ meta }) => (locations = meta.miniLocations) },
  });
  const source = `const { lead } = instruments;
stack(note("c4").apply(lead()), note("d4").apply(lead({ cutoff: 900 })))`;
  expect(await rt.evaluate(source, { instruments: lead(1200), autostart: false })).toBe(true);
  const haps = rt.query(0, 1)!;
  expect(haps.map(({ value }) => value)).toEqual([
    { note: 'c4', s: 'sawtooth', cutoff: 1200 },
    { note: 'd4', s: 'sawtooth', cutoff: 900 },
  ]);
  expect(locations.map(([start, end]) => source.slice(start, end))).toEqual(['c4', 'd4']);
  expect(haps.map(({ context }) => context.locations)).toEqual(
    locations.map(([start, end]) => [{ start, end }]),
  );
  expect(get(rt.store).activeCode).toBe(source);
});

test('layers kit voices and assigns qualified stem labels only to unlabeled haps', async () => {
  const { rt } = fakeRuntime({ prebake });
  const source = `drums: stack(note("c4"), note("d4").label('solo')).apply(instruments.drums())`;
  expect(await rt.evaluate(source, { instruments: kit, autostart: false })).toBe(true);
  expect(rt.query(0, 1)!.map(({ value }) => value)).toEqual([
    { note: 'c4', s: 'sine', label: 'drums/kick' },
    { note: 'd4', s: 'sine', label: 'solo' },
    { note: 'c4', s: 'white', label: 'drums/hat' },
    { note: 'd4', s: 'white', label: 'solo' },
  ]);
});

test('the runtime queue gives a lazy pattern its own bindings through failures and later updates', async () => {
  const { rt } = fakeRuntime({ prebake });
  const source = 'new Pattern(state => note("c4").apply(instruments.lead()).query(state))';
  const first = rt.evaluate(source, { instruments: lead(1000), autostart: false });
  const invalid = rt.evaluate('note("c4").apply(instruments.lead({ cutoff: \'bad\' }))', {
    instruments: lead(2000),
    autostart: false,
  });
  expect(await first).toBe(true);
  expect(await invalid).toBe(false);
  expect(get(rt.store).evalError).toContain("Instrument 'lead': option 'cutoff' must be a number");
  expect(rt.query(0, 1)![0].value.cutoff).toBe(1000);
  const oldPattern = rt.scheduler.pattern;
  expect(await rt.evaluate(source, { instruments: lead(2000), autostart: false })).toBe(true);
  expect(rt.query(0, 1)![0].value.cutoff).toBe(2000);
  expect(oldPattern.queryArc(0, 1)[0].value.cutoff).toBe(1000);
  expect(Object.hasOwn(globalThis, Symbol.for('strudeland.instrument-namespace'))).toBe(false);
});

test('rejects malformed chains and unknown options, and permits ordinary and shadowed scopes', async () => {
  const { rt } = fakeRuntime({ prebake });
  const options = { instruments: lead(1200), autostart: false };
  expect(await rt.evaluate('note("c4").apply(instruments.lead({ extra: 1 }))', options)).toBe(
    false,
  );
  expect(get(rt.store).evalError).toContain("Instrument 'lead': unknown option 'extra'");
  const malformed = {
    ...options,
    instruments: {
      ...options.instruments,
      bass: { schema: 1, parameters: {}, voices: [{ name: 'bass', chain: [] }] },
    } satisfies InstrumentSnapshot,
  };
  expect(await rt.evaluate('note("c4")', malformed)).toBe(false);
  expect(get(rt.store).evalError).toContain("Instrument 'bass': voice 'bass' has no controls");
  expect(await rt.evaluate('const instruments = 7; note(instruments)', options)).toBe(true);
  expect(rt.query(0, 1)![0].value.note).toBe(7);
  expect(await rt.evaluate('note(Object.keys(instruments).length)', { autostart: false })).toBe(
    true,
  );
  expect(rt.query(0, 1)![0].value.note).toBe(0);
});

test('song-local var and function declarations retain function-body semantics', async () => {
  const { rt } = fakeRuntime({ prebake });
  const source = `var voice;
function voice() { return 1 }
function voice() { return 2 }
var instruments;
note(typeof instruments === 'undefined' ? voice() : 0)`;
  for (const instruments of [undefined, lead(1200)]) {
    expect(await rt.evaluate(source, { instruments, autostart: false })).toBe(true);
    expect(rt.query(0, 1)![0].value.note).toBe(2);
  }
});

test('raw render-page snapshots enforce the project schema bounds', () => {
  registerInstrumentOperation();
  const transpiler = createInstrumentTranspiler();
  const check = (snapshot: InstrumentSnapshot) => {
    try {
      transpiler.transpile('silence', {}, snapshot);
    } finally {
      transpiler.clear();
    }
  };
  const base = kit.drums;
  const voice = base.voices[0];
  const voices = Array.from({ length: 33 }, (_, i) => ({ ...voice, name: `v${i}` }));
  const checkVoices = (voices: typeof base.voices) => check({ kit: { ...base, voices } });
  expect(() => checkVoices(voices.slice(0, 32))).not.toThrow();
  expect(() => checkVoices(voices)).toThrow('32 voices');
  expect(() => checkVoices([{ ...voice, chain: Array(65).fill(voice.chain[0]) }])).toThrow(
    '64 controls',
  );

  const parameters = (count: number, value: string) =>
    Object.fromEntries(Array.from({ length: count }, (_, i) => [`p${i}`, value]));
  const defs = (count: number, definition: typeof base) =>
    Object.fromEntries(Array.from({ length: count }, (_, i) => [`i${i}`, definition]));
  expect(() => check(defs(129, base))).toThrow('128 instruments');
  expect(() => check({ kit: { ...base, parameters: parameters(65, 'x') } })).toThrow(
    '64 parameters',
  );
  const maxString = '🎵'.repeat(256);
  expect(() => check({ kit: { ...base, parameters: { text: maxString + 'x' } } })).toThrow(
    "invalid parameter 'text'",
  );
  const large = { ...base, parameters: parameters(63, maxString) };
  expect(() => check(defs(32, large))).not.toThrow();
  expect(() => check(defs(33, large))).toThrow('snapshot exceeds size limit');
  expect(() => check({ kit: { ...base, parameters: parameters(64, maxString) } })).toThrow(
    'size limit',
  );
});
