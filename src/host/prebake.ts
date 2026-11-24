/**
 * The headless host's prebake: `src/lib/strudel/prebake.ts` without `@strudel/webaudio`,
 * `@strudel/codemirror` and superdough's sound registrations. Superdough's functions are stubs
 * (no fetch, no audio) and the editor's value-affecting names are shims (`./shims`).
 */

import * as core from '@strudel/core';
import * as draw from '@strudel/draw';
import * as mini from '@strudel/mini';
import * as tonal from '@strudel/tonal';
import { registerInstrumentOperation } from '../lib/strudel/instruments';
import { song } from '../lib/strudel/song';
import { shims } from './shims';
import { SUPERDOUGH_FUNCTIONS } from './superdough-names';

/** Superdough functions whose callers may `await` them (the transpiler adds `await` to
 *  `samples`): they resolve at once. */
export const ASYNC_STUBS: ReadonlySet<string> = new Set([
  'samples',
  'aliasBank',
  'initAudio',
  'initAudioOnFirstClick',
  'fetchSampleMap',
  'loadWorklets',
  'renderPatternAudio',
  'rawdsp',
  'dspWorklet',
]);

/** One no-op per name in `SUPERDOUGH_FUNCTIONS`; the `ASYNC_STUBS` resolve. */
export function superdoughStubs(): Record<string, () => unknown> {
  const stubs: Record<string, () => unknown> = {};
  for (const name of SUPERDOUGH_FUNCTIONS) {
    stubs[name] = ASYNC_STUBS.has(name) ? () => Promise.resolve() : () => undefined;
  }
  return stubs;
}

/** Runs once per runtime, before the first evaluate. `shims` and the stubs come last, so they
 *  win over any same-named export of the real modules. */
export async function hostPrebake(): Promise<void> {
  await core.evalScope(core, mini, tonal, draw, { song }, shims, superdoughStubs());
  registerInstrumentOperation();
}
