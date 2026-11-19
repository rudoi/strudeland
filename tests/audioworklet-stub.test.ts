import { expect, test } from 'bun:test';

test('superdough loads under bun test through the audioworklet preload', async () => {
  const m = await import('superdough');
  expect(typeof m.superdough).toBe('function');
});
