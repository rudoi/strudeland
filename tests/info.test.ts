import { expect, test } from 'bun:test';
import { formatStrudel } from '../src/lib/info';

const STRUDEL = {
  commit: '0cb6341299229f739b5090be93528bb4f170d855',
  date: '2026-11-08T02:32:27+01:00',
  kabelsalat: { '@kabelsalat/lib': '0.4.1', '@kabelsalat/web': '0.4.1' },
};

test('formatStrudel: the short commit and day, then one line per kabelsalat package', () => {
  expect(formatStrudel(STRUDEL)).toBe(
    'Strudel 0cb6341 (2026-11-08)\n@kabelsalat/lib 0.4.1\n@kabelsalat/web 0.4.1',
  );
});

test('formatStrudel keeps the key order of kabelsalat', () => {
  const s = { ...STRUDEL, kabelsalat: { b: '2', a: '1' } };
  expect(formatStrudel(s).split('\n').slice(1)).toEqual(['b 2', 'a 1']);
});

test('edge: no kabelsalat entries gives only the Strudel line', () => {
  expect(formatStrudel({ ...STRUDEL, kabelsalat: {} })).toBe('Strudel 0cb6341 (2026-11-08)');
});

test('edge: a short commit or date does not throw', () => {
  expect(formatStrudel({ commit: 'abc', date: '2026', kabelsalat: {} })).toBe('Strudel abc (2026)');
  expect(formatStrudel({ commit: '', date: '', kabelsalat: {} })).toBe('Strudel  ()');
});
