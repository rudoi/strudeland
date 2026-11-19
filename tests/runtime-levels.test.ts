import { afterEach, expect, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import type { LevelFrame } from '../src/lib/levels/levels';
import { loadLimiter, saveLimiter } from '../src/lib/audio/limiter-pref';
import type { RuntimeState } from '../src/lib/strudel/runtime';
import { fakeRuntime as fake } from './helpers/fake-runtime';

await core.evalScope(core, mini);

const PLAIN = 'note("c e g").s("sine")';
const frame = (peak: number, overs = 0, limitDb = 0): LevelFrame => ({
  peak: [peak, peak / 2],
  rms: [peak / 2, peak / 4],
  overs: [overs, 0],
  limitDb,
});
const snap = (rt: { store: { subscribe: (f: (s: RuntimeState) => void) => () => void } }) => {
  let s!: RuntimeState;
  rt.store.subscribe((v) => (s = v))();
  return s;
};

test('frames from both backends feed the hold, and drainLevels merges and clears', () => {
  const { b, rb, rt } = fake({ rust: true });
  expect(rt.levels()).toBeNull();
  expect(rt.drainLevels()).toBeNull();
  b.level(frame(0.5, 1, -1));
  rb.level(frame(0.8, 2, -3));
  expect(rt.levels()).toEqual({ peak: [0.8, 0.4], overs: 3, limitDb: -3 });
  expect(rt.drainLevels()).toEqual({
    peak: [0.8, 0.4],
    rms: [0.4, 0.2],
    overs: [3, 0],
    limitDb: -3,
  });
  expect(rt.drainLevels()).toBeNull();
  // Draining does not touch the hold.
  expect(rt.levels()?.overs).toBe(3);
});

test('the hold resets on a start from stopped and on each evaluate, not on a stop', async () => {
  const { b, rt } = fake();
  expect(await rt.evaluate(PLAIN, { autostart: false })).toBe(true);
  b.level(frame(0.9, 4));
  expect(await rt.start()).toBe(true);
  expect(rt.levels()).toBeNull();
  b.level(frame(0.9, 4));
  rt.stop();
  expect(rt.levels()?.overs).toBe(4);
  // An evaluate while stopped resets.
  expect(await rt.evaluate(PLAIN, { autostart: false })).toBe(true);
  expect(rt.levels()).toBeNull();
  expect(await rt.start()).toBe(true);
  b.level(frame(0.9, 4));
  // An evaluate while playing resets.
  expect(await rt.evaluate(PLAIN)).toBe(true);
  expect(rt.levels()).toBeNull();
  rt.stop();
});

test('frames merged while stopped never reach the meter after a start or evaluate', async () => {
  const { b, rt } = fake();
  expect(await rt.evaluate(PLAIN, { autostart: false })).toBe(true);
  // Stopped: the meter does not drain, so these would pile up.
  b.level(frame(1.5, 9));
  expect(await rt.start()).toBe(true);
  expect(rt.drainLevels()).toBeNull();
  b.level(frame(1.5, 9));
  expect(await rt.evaluate(PLAIN)).toBe(true);
  expect(rt.drainLevels()).toBeNull();
  rt.stop();
});

test('a failed evaluate does not reset the hold', async () => {
  const { b, rt } = fake();
  b.level(frame(0.9, 4));
  expect(await rt.evaluate('note("c").nonexistent()', { autostart: false })).toBe(false);
  expect(rt.levels()?.overs).toBe(4);
});

test('an engine switch while playing keeps the hold', async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN)).toBe(true);
  // Evaluating a rust song while playing arms a switch at the next bar line.
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  b.level(frame(0.9, 4));
  expect(rt.levels()?.overs).toBe(4);
  tick(80);
  await new Promise((r) => setTimeout(r, 20));
  expect(snap(rt).engine).toBe('rust');
  // The new engine's start inside the swap did not reset; its frames add up.
  rb.level(frame(0.5, 1));
  expect(rt.levels()?.overs).toBe(5);
  rt.stop();
});

test('setLimiter reaches both backends and the store; the preference is applied at creation', () => {
  const { b, rb, rt } = fake({ rust: true, limiter: true });
  expect(snap(rt).limiter).toBe(true);
  expect(b.limiters).toEqual([true]);
  expect(rb.limiters).toEqual([true]);
  rt.setLimiter(false);
  expect(snap(rt).limiter).toBe(false);
  expect(b.limiters).toEqual([true, false]);
  expect(rb.limiters).toEqual([true, false]);
});

test('limiterAvailable follows the active backend on the interval', () => {
  const { b, rt, tick } = fake();
  expect(snap(rt).limiterAvailable).toBe(true);
  b.limiterOk = false;
  tick(1);
  expect(snap(rt).limiterAvailable).toBe(false);
  b.limiterOk = true;
  tick(1);
  expect(snap(rt).limiterAvailable).toBe(true);
});

const realStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
afterEach(() => {
  if (realStorage) Object.defineProperty(globalThis, 'localStorage', realStorage);
  else delete (globalThis as any).localStorage;
});
const useStorage = (s: unknown) =>
  Object.defineProperty(globalThis, 'localStorage', { value: s, configurable: true });

test('limiter-pref round-trips, defaults to off and survives a throwing localStorage', () => {
  const m = new Map<string, string>();
  useStorage({
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
  });
  expect(loadLimiter()).toBe(false);
  saveLimiter(true);
  expect(m.get('strudeland.limiter')).toBe('1');
  expect(loadLimiter()).toBe(true);
  saveLimiter(false);
  expect(m.get('strudeland.limiter')).toBe('0');
  expect(loadLimiter()).toBe(false);
  const boom = () => {
    throw new Error('blocked');
  };
  useStorage({ getItem: boom, setItem: boom });
  expect(loadLimiter()).toBe(false);
  expect(() => saveLimiter(true)).not.toThrow();
});
