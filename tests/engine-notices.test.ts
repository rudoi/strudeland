import { expect, test } from 'bun:test';
import { engineNotices } from '../src/lib/audio/engine-notices';

test('one notice per warning, the signature verbatim', () => {
  expect(engineNotices([{ signature: 's=gm_piano', keys: [], firstCycle: 3 }])).toEqual([
    { key: 's=gm_piano', text: 'rust engine skipped s=gm_piano' },
  ]);
});

test('the keys are the detail, joined with commas; none without keys', () => {
  const [a, b] = engineNotices([
    { signature: 'sine+vowel', keys: ['vowel', 'phaser'], firstCycle: 0 },
    { signature: 'x', keys: [], firstCycle: 1 },
  ]);
  expect(a!.detail).toBe('keys: vowel, phaser');
  expect(b!.detail).toBeUndefined();
  expect('detail' in b!).toBe(false);
});

test('keys are stable per signature and nothing for no warnings', () => {
  const ws = [
    { signature: 'a', keys: ['k'], firstCycle: 0 },
    { signature: 'b', keys: [], firstCycle: 2 },
  ];
  expect(engineNotices(ws).map((n) => n.key)).toEqual(['a', 'b']);
  expect(engineNotices(ws.slice(1)).map((n) => n.key)).toEqual(['b']);
  expect(engineNotices([])).toEqual([]);
});
