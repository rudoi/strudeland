import { writable } from 'svelte/store';
import { errorMessage } from '../errors';
import type { InspectionRequest, InspectionResult } from './types';

type Status =
  | { request: InspectionRequest; phase: 'loading' }
  | { request: InspectionRequest; phase: 'ready'; result: InspectionResult }
  | { request: InspectionRequest; phase: 'error'; message: string };

/** Runs at most one native call and ignores replies for superseded requests. */
export function createInspectionQueue(
  inspect: ((request: InspectionRequest) => Promise<InspectionResult>) | null,
  debounceMs = 120,
) {
  const store = writable<Status | null>(null);
  let current: InspectionRequest | null = null;
  let pending: InspectionRequest | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = false;

  async function drain(): Promise<void> {
    if (!inspect || active || !pending) return;
    const request = pending;
    pending = null;
    active = true;
    try {
      const result = await inspect(request);
      if (request === current) store.set({ request, phase: 'ready', result });
    } catch (error) {
      if (request === current) store.set({ request, phase: 'error', message: errorMessage(error) });
    } finally {
      active = false;
      void drain();
    }
  }

  return {
    available: inspect !== null,
    store: { subscribe: store.subscribe },
    request(request: InspectionRequest | null) {
      if (request === current) return;
      current = request;
      pending = null;
      clearTimeout(timer);
      store.set(request ? { request, phase: 'loading' } : null);
      if (!request) return;
      timer = setTimeout(() => {
        pending = request;
        void drain();
      }, debounceMs);
    },
  };
}
