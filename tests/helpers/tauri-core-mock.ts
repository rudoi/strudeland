import { mock } from 'bun:test';

/** A recorded `invoke` call. */
export interface InvokeCall {
  cmd: string;
  args: unknown;
}

/** The fake `Channel`: holds the handler it was built with, as the real one does. */
export class FakeChannel<T> {
  constructor(public onmessage: (m: T) => void = () => {}) {}
}

/**
 * Mocks the Tauri module `specifier` with its real exports, then `overrides` over them.
 * `mock.module` is process-wide, so a partial mock breaks every later importer of a name it
 * left out (`dpi.js` imports `SERIALIZE_TO_IPC_FN` from core, `window.js` imports
 * `TauriEvent` from event), whichever test file loads them first.
 */
export function mockTauriModule(specifier: string, overrides: Record<string, unknown>) {
  const real: Record<string, unknown> = { ...require(specifier) };
  mock.module(specifier, () => ({ ...real, ...overrides }));
}

/**
 * Mocks `@tauri-apps/api/core`: the real exports, with `invoke`, `Channel` and `isTauri` (the
 * surface the app imports) replaced. `answer` sets how each invoke resolves or throws; `calls`
 * logs them.
 */
export function mockTauriCore() {
  const state = {
    calls: [] as InvokeCall[],
    impl: (() => undefined) as (cmd: string, args: unknown) => unknown,
    tauri: true,
  };
  mockTauriModule('@tauri-apps/api/core', {
    invoke: async (cmd: string, args: unknown) => {
      state.calls.push({ cmd, args });
      return state.impl(cmd, args);
    },
    Channel: FakeChannel,
    isTauri: () => state.tauri,
  });
  return {
    calls: state.calls,
    /** Each later invoke returns (or throws from) `impl`; the call log is cleared. */
    answer(impl: (cmd: string, args: unknown) => unknown) {
      state.calls.length = 0;
      state.impl = impl;
    },
    /** Each later invoke rejects with `value`. */
    reject(value: unknown) {
      this.answer(() => {
        throw value;
      });
    },
    setTauri(on: boolean) {
      state.tauri = on;
    },
  };
}
