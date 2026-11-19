/**
 * Imported first by the render page, before any superdough module: a live `AudioContext` is
 * impossible there. Superdough creates one only from `getAudioContext()` when no context was set,
 * so this fires only when something asks for audio before `setAudioContext(offlineCtx)`.
 */

export const GUARD_MESSAGE =
  'render page: a live AudioContext was requested before setAudioContext';

class GuardedAudioContext {
  constructor() {
    throw new Error(GUARD_MESSAGE);
  }
}

(globalThis as any).AudioContext = GuardedAudioContext;
