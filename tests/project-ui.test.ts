import { describe, expect, test } from 'bun:test';
import {
  defaultProjectName,
  matchShortcut,
  primaryModifier,
  songStem,
} from '../src/lib/project/ui';

const key = (key: string, extra: Record<string, boolean> = {}) => ({
  key,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...extra,
});

describe('defaultProjectName', () => {
  test('last path segment', () => expect(defaultProjectName('/a/b/my song')).toBe('my song'));
  test('trailing slashes', () => expect(defaultProjectName('/a/b/')).toBe('b'));
  test('windows separators', () => expect(defaultProjectName('C:\\x\\proj\\')).toBe('proj'));
  test('root and empty', () => {
    expect(defaultProjectName('/')).toBe('');
    expect(defaultProjectName('')).toBe('');
  });
  test('no separator', () => expect(defaultProjectName('solo')).toBe('solo'));
});

describe('matchShortcut', () => {
  for (const [platform, expected, other] of [
    ['MacIntel', 'metaKey', 'ctrlKey'],
    ['Win32', 'ctrlKey', 'metaKey'],
    ['Linux x86_64', 'ctrlKey', 'metaKey'],
  ] as const) {
    const modifier = primaryModifier(platform);
    const pressed = { [modifier]: true };

    test(`${platform} uses its primary modifier for O and N`, () => {
      expect(modifier).toBe(expected);
      expect(matchShortcut(key('o', pressed), modifier)).toBe('openProject');
      expect(matchShortcut(key('N', pressed), modifier)).toBe('newSong');
    });

    test(`${platform} ignores other modifiers, repeats, and unrelated keys`, () => {
      expect(matchShortcut(key('o'), modifier)).toBeNull();
      expect(matchShortcut(key('o', { [other]: true }), modifier)).toBeNull();
      for (const extra of [other, 'altKey', 'shiftKey', 'repeat']) {
        expect(matchShortcut(key('o', { ...pressed, [extra]: true }), modifier)).toBeNull();
      }
      expect(matchShortcut(key('s', pressed), modifier)).toBeNull();
    });
  }
});

describe('songStem', () => {
  test('trims and drops .js', () => {
    expect(songStem('  intro ')).toBe('intro');
    expect(songStem('intro.js')).toBe('intro');
    expect(songStem('')).toBe('');
  });
});
