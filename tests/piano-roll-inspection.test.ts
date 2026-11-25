import { expect, test } from 'bun:test';
import { get } from 'svelte/store';
import { createInspectionQueue } from '../src/lib/piano-roll/inspection';
import type { InspectionRequest, InspectionResult } from '../src/lib/piano-roll/types';

const request = (code: string): InspectionRequest => ({
  root: '/project',
  code,
  view: 'song',
  from: 0,
  to: 1,
});

test('inspection admits one active request and keeps only the newest pending request', async () => {
  const calls: { request: InspectionRequest; reply(result: InspectionResult): void }[] = [];
  const queue = createInspectionQueue(
    (req) => new Promise((reply) => calls.push({ request: req, reply })),
    0,
  );
  const a = request('a');
  const b = request('b');
  const c = request('c');
  queue.request(a);
  await Bun.sleep(5);
  queue.request(b);
  await Bun.sleep(5);
  queue.request(c);
  await Bun.sleep(5);
  expect(calls.map((call) => call.request)).toEqual([a]);
  calls[0]!.reply({ notes: [], truncated: false });
  await Promise.resolve();
  expect(get(queue.store)).toEqual({ request: c, phase: 'loading' });
  expect(calls.map((call) => call.request)).toEqual([a, c]);
  queue.request(null);
  calls[1]!.reply({ notes: [], truncated: true });
  await Promise.resolve();
  expect(get(queue.store)).toBeNull();
});

test('inspection surfaces a current failure and accepts a later successful request', async () => {
  const a = request('a');
  const b = request('b');
  const result = { notes: [], truncated: false };
  const queue = createInspectionQueue(async (req) => {
    if (req === a) throw new Error('timed out');
    return result;
  }, 0);
  queue.request(a);
  await Bun.sleep(5);
  expect(get(queue.store)).toEqual({ request: a, phase: 'error', message: 'timed out' });
  queue.request(b);
  await Bun.sleep(5);
  expect(get(queue.store)).toEqual({ request: b, phase: 'ready', result });
});
