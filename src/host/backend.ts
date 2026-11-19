/**
 * The headless host's audio backend: inert, like `createOfflineBackend`
 * (`src/lib/audio/offline.ts`). Its clock never runs, so `createRuntime().evaluate` evaluates
 * without starting the scheduler and the host can only `query`. No superdough import.
 */

import type { AudioBackend } from '../lib/audio/backend';

export function createHostBackend(): AudioBackend {
  return {
    // The value the runtime's `backend` state field carries on the page, so `info` is unaffected.
    id: 'superdough',
    init: () => Promise.reject(new Error('host backend: init must not be called')),
    resume: () => Promise.resolve(false),
    isRunning: () => false,
    getTime: () => 0,
    setSeed: () => {},
    output: () => {},
  };
}
