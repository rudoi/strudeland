import { describe, expect, test } from 'bun:test';
import { defaultProjectName, matchShortcut, songStem } from '../src/lib/project/ui';

const key = (key: string, extra: Record<string, boolean> = {}) => ({
  key,
  metaKey: true,
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
  test('Cmd-O and Cmd-N, any case', () => {
    expect(matchShortcut(key('o'))).toBe('openProject');
    expect(matchShortcut(key('N'))).toBe('newSong');
  });
  test('ignores Ctrl, Alt, Shift variants', () => {
    for (const mod of ['ctrlKey', 'altKey', 'shiftKey']) {
      expect(matchShortcut(key('o', { [mod]: true }))).toBeNull();
    }
  });
  test('needs Cmd; ignores repeats and other keys', () => {
    expect(matchShortcut(key('o', { metaKey: false }))).toBeNull();
    expect(matchShortcut(key('o', { repeat: true }))).toBeNull();
    expect(matchShortcut(key('s'))).toBeNull();
  });
});

describe('songStem', () => {
  test('trims and drops .js', () => {
    expect(songStem('  intro ')).toBe('intro');
    expect(songStem('intro.js')).toBe('intro');
    expect(songStem('')).toBe('');
  });
});
