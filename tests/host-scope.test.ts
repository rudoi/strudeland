// The headless host's eval scope against the app's (`src/lib/strudel/prebake.ts`): every app
// name is real, shimmed, stubbed or listed as not reachable, and the lists stay exact.
import { expect, test } from 'bun:test';
import * as codemirror from '@strudel/codemirror';
import * as core from '@strudel/core';
import * as draw from '@strudel/draw';
import * as mini from '@strudel/mini';
import * as tonal from '@strudel/tonal';
import * as webaudio from '@strudel/webaudio';
import { song } from '../src/lib/strudel/song';
import { NOT_REACHABLE } from '../src/host/not-reachable';
import { SUPERDOUGH_FUNCTIONS } from '../src/host/superdough-names';
import { snapshotOwnProperties } from './helpers/prototype-snapshot';

// The name checks use the exports alone. Restore the real prototype after importing them.
await import('@strudel/transpiler');
const restore = snapshotOwnProperties(core.Pattern.prototype);
const [{ shims }, { ASYNC_STUBS, superdoughStubs }] = await Promise.all([
  import('../src/host/shims'),
  import('../src/host/prebake'),
]).finally(restore);

const keys = (...mods: object[]): Set<string> => new Set(mods.flatMap((m) => Object.keys(m)));
const app = keys(core, mini, tonal, webaudio, draw, codemirror, { song });
const host = keys(core, mini, tonal, draw, { song }, shims, superdoughStubs());
const appOnly = [...app].filter((n) => !host.has(n)).sort();
const webaudioFunctions = Object.keys(webaudio)
  .filter((k) => typeof (webaudio as any)[k] === 'function')
  .sort();

test('every app-scope name the host lacks is listed as not reachable', () => {
  expect(appOnly.filter((n) => !(n in NOT_REACHABLE))).toEqual([]);
});

test('every not-reachable entry is an app-only name with a reason', () => {
  for (const [name, reason] of Object.entries(NOT_REACHABLE)) {
    expect({ name, appOnly: appOnly.includes(name) }).toEqual({ name, appOnly: true });
    expect({ name, reason: reason.trim().length > 0 }).toEqual({ name, reason: true });
  }
  expect(Object.keys(NOT_REACHABLE).sort()).toEqual(appOnly);
});

test('every webaudio function is in the host scope, and the stub list is exactly them', () => {
  expect(webaudioFunctions.filter((n) => !host.has(n))).toEqual([]);
  const shimmedByName = new Set(Object.keys(shims));
  expect([...SUPERDOUGH_FUNCTIONS]).toEqual(webaudioFunctions.filter((n) => !shimmedByName.has(n)));
});

test('SUPERDOUGH_FUNCTIONS is sorted, without duplicates', () => {
  const names = [...SUPERDOUGH_FUNCTIONS];
  expect(names).toEqual([...names].sort());
  expect(new Set(names).size).toBe(names.length);
  expect(names.length).toBeGreaterThan(0);
});

test('no webaudio function is on the not-reachable list', () => {
  expect(webaudioFunctions.filter((n) => n in NOT_REACHABLE)).toEqual([]);
});

test('the stubs: no-ops, and the async ones resolve to undefined', async () => {
  const stubs = superdoughStubs();
  expect(Object.keys(stubs)).toEqual([...SUPERDOUGH_FUNCTIONS]);
  for (const name of ASYNC_STUBS) {
    expect({ name, listed: SUPERDOUGH_FUNCTIONS.includes(name) }).toEqual({ name, listed: true });
    const r = (stubs[name] as () => Promise<unknown>)();
    expect(r).toBeInstanceOf(Promise);
    expect(await r).toBeUndefined();
  }
  for (const name of SUPERDOUGH_FUNCTIONS.filter((n) => !ASYNC_STUBS.has(n))) {
    expect({ name, value: stubs[name]!() }).toEqual({ name, value: undefined });
  }
});

test('shims and stubs override no real module export', () => {
  const real = keys(core, mini, tonal, draw, { song });
  expect(Object.keys(shims).filter((n) => real.has(n))).toEqual([]);
  // superdough's logger and errorLogger shadow core's on the page too (webaudio comes later).
  expect(SUPERDOUGH_FUNCTIONS.filter((n) => real.has(n))).toEqual(['errorLogger', 'logger']);
});
