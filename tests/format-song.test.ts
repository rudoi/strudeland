import { describe, expect, test } from 'bun:test';
import { m } from '@strudel/mini';
import { getLanguages, transpiler } from '@strudel/transpiler';
import { formatSong } from '../src/lib/editor/format-song';

const SIXTEEN = 'c4 ~ e4 ~ g4 ~ e4 ~ a4 ~ g4 ~ e4 ~ d4 ~';
const THIRTY_TWO = SIXTEEN + ' ' + SIXTEEN;
const ROW = 'c4 ~  e4 ~   g4 ~  e4 ~     a4 ~  g4 ~   e4 ~  d4 ~';
const SIXTEENTHS = ROW.replace('     ', '   ');

function formatted(source: string): string {
  const edits = formatSong(source);
  let end = 0;
  for (const edit of edits) {
    expect(edit.from).toBeGreaterThanOrEqual(end);
    expect(edit.to).toBeGreaterThanOrEqual(edit.from);
    // Atomic changes leave every note and musical delimiter in its own mapped editor range.
    expect(source.slice(edit.from, edit.to)).toMatch(/^(?:\s*|["`])$/);
    expect(edit.insert).toMatch(/^(?:\s*|["`])$/);
    end = edit.to;
  }
  const next = edits.reduceRight(
    (text, e) => text.slice(0, e.from) + e.insert + text.slice(e.to),
    source,
  );
  expect(formatSong(next)).toEqual([]);
  return next;
}

// Run only the static pattern fixtures through the real plugin and mini runtime.
function musicalEvents(script: string) {
  const pattern = Function('m', transpiler(script).output)(m);
  return {
    steps: pattern._steps?.toString(),
    events: pattern
      .queryArc(0.25, 4.75)
      .map((h: any) => [
        h.value,
        h.whole.begin.toString(),
        h.whole.end.toString(),
        h.part.begin.toString(),
        h.part.end.toString(),
      ]),
  };
}

describe('4/4 pattern layout', () => {
  test('32 slots use two rows aligned in quartets and eighths, preserve indentation, spelling and inline comments', () => {
    const prefix = '/* 🎵 */\n\t$: note(';
    const suffix = ").s('piano') // original comment";
    const source = prefix + '"' + THIRTY_TWO + '"' + suffix;
    const expected = prefix + '`\n\t  ' + ROW + '\n\t  ' + ROW + '\n\t`' + suffix;
    expect(formatted(source)).toBe(expected);
    for (const quote of ['"', '`']) {
      const script = '(' + quote + THIRTY_TWO + quote + ')';
      const next = formatted(script);
      expect(musicalEvents(next)).toEqual(musicalEvents(script));
      const locations = transpiler(next).miniLocations as [number, number][];
      const words = THIRTY_TWO.split(' ');
      expect(locations.map(([start, end]) => next.slice(start, end).trimEnd())).toEqual(words);
      expect(locations.map(([start], i) => next.slice(start, start + words[i].length))).toEqual(
        words,
      );
    }
  });

  test('16 equal slots keep their delimiter and fit on one line; different token widths line up', () => {
    for (const quote of ['"', '`']) {
      expect(formatted('note(' + quote + ' ' + SIXTEEN + ' ' + quote + ')')).toBe(
        'note(' + quote + SIXTEENTHS + quote + ')',
      );
    }
    const words = [
      'kick',
      'sn',
      'hh',
      '-',
      '~',
      '03',
      'a#4',
      'hat_1',
      'a',
      's',
      'q',
      'r',
      'x',
      'β',
      'z',
      'é',
    ];
    const output = formatted('s("' + words.join(' ') + '")');
    expect(output).toContain('kick  sn    hh    -      ~');
    expect(output.match(/[\w#βé~-]+/gu)).toEqual(['s', ...words]);
  });

  test('preserves CRLF outside the mini and gives each token its exact source start', () => {
    const next = formatted('// song\r\nnote("' + THIRTY_TWO + '")\r\n');
    expect(next).toBe('// song\r\nnote(`\n  ' + ROW + '\n  ' + ROW + '\n`)\r\n');
    const starts = transpiler(next).miniLocations as [number, number][];
    expect(starts.map(([start]) => next[start])).toEqual(
      THIRTY_TWO.split(' ').map((word) => word[0]),
    );
  });

  test('uses the resulting indentation if another mini ends on the same source line', () => {
    const source = '  stack(`' + SIXTEEN + '\n      `, "' + THIRTY_TWO + '")';
    expect(formatted(source)).toBe(
      '  stack(`' + SIXTEENTHS + '`, `\n    ' + ROW + '\n    ' + ROW + '\n  `)',
    );
  });

  test('explicit slow bars stay bracketed in order with exact ordered rational whole/part arcs', () => {
    const script = '("<[' + SIXTEEN + '] [' + THIRTY_TWO + ']>")';
    const expected = '(`<\n  [' + SIXTEENTHS + ']\n  [' + ROW + '\n   ' + ROW + ']\n>`)';
    expect(formatted(script)).toBe(expected);
    expect(musicalEvents(expected)).toEqual(musicalEvents(script));
  });

  test('ignores metadata and directives, template substitutions, property keys, and escaped strings', () => {
    const q = '"' + THIRTY_TWO + '"';
    const t = '`' + THIRTY_TWO + '`';
    const scripts = [
      q + '; n(1)',
      'function f(){' + q + ';return 1}',
      'const meta = {title: ' + q + '}; n(1)',
      'const meta = {custom: {nested: [' + q + ']}}; n(1)',
      "s('" + THIRTY_TWO + "')",
      'const obj = {' + q + ': 1, [' + t + ']: 2}; obj[' + q + ']',
      'class C { ' + q + ' = 1; [' + t + '](){} }',
      'tag' + t,
      'tag`str ${note(' + q + ')} tail`',
      'note(`' + THIRTY_TWO + ' ${note(' + q + ')}`)',
      'n("' + THIRTY_TWO.replace(' ', '\\x20') + '")',
      'const regex = /' + THIRTY_TWO + '/;',
    ];
    for (const input of scripts) expect(formatSong(input)).toEqual([]);
  });

  test('reports unsupported syntax individually, and never changes the mini language', () => {
    const invalidBodies = [
      'a b c d',
      SIXTEEN + ' ~',
      '^' + SIXTEEN,
      SIXTEEN.replace('c4', 'c4@1'),
      SIXTEEN.replace('c4', 'c4@3'),
      SIXTEEN.replace('c4', 'c4 _'),
      SIXTEEN.replace('c4', 'c4!4'),
      SIXTEEN.replace('c4', 'c4*4'),
      SIXTEEN.replace('c4', 'c4?'),
      SIXTEEN.replace('c4', 'c4(3,8)'),
      SIXTEEN.replace('c4', '[c4 a4]'),
      SIXTEEN.replace('c4', 'c4:3'),
      SIXTEEN + ' | b',
      SIXTEEN + ' . b',
      SIXTEEN + ', b',
      '{' + SIXTEEN + '}%4',
      '<[' + SIXTEEN + ']>!2',
      '<[' + SIXTEEN + '] ~>',
      '[' + SIXTEEN + ']/2',
      '<[' + SIXTEEN + ']',
      '(((((',
    ];
    const invalid = invalidBodies.map((b) => 'n("' + b + '")').join(';');
    expect(formatted(invalid + ';n("' + SIXTEEN + '")')).toBe(invalid + ';n("' + SIXTEENTHS + '")');
  });

  test('respects nested, unclosed and unmatched mini toggle comments', () => {
    const literal = 's("' + SIXTEEN + '");';
    const off =
      '/* mini-off */\n' +
      literal +
      '\n// mini-off for FX\n' +
      literal +
      '\n/* mini-on */\n' +
      literal +
      '\n/* mini-on */\n';
    const unclosed = '\n// mini-off\n' + literal;
    const unmatched = '/* mini-on */\n';
    expect(formatted(unmatched + off + literal + unclosed)).toBe(
      unmatched + off + 's("' + SIXTEENTHS + '");' + unclosed,
    );
  });

  test('requires valid JavaScript and metadata, 4/4 meter, and the stock mini language', () => {
    const song = 'await setup(); n("' + SIXTEEN + '")';
    expect(formatted("const meta = {meter: '4/4'}; " + song)).toContain(SIXTEENTHS);
    for (const prefix of [
      "const meta = {meter: '6/8'}; ",
      "const meta = {meter: '3/4'}; ",
      'const meta = {meter: "4/4"}; ',
      'const meta = { bpm: -1 }; ',
      'const meta = {title: 10}; ',
    ]) {
      expect(formatSong(prefix + song)).toEqual([]);
    }
    expect(formatSong(song + '; f(')).toEqual([]);
    const languages = getLanguages();
    const previous = languages.get('minilang');
    languages.set('minilang', { name: 'testCustom' });
    try {
      expect(formatSong(song)).toEqual([]);
    } finally {
      if (previous) languages.set('minilang', previous);
      else languages.delete('minilang');
    }
  });

  test('a template cannot absorb the preceding expression across ASI, even with optional chaining or a comment', () => {
    for (const previous of ['p()', 'foo?.bar', 'x = 1n']) {
      const unsafe = previous + '\n/* bare */ "' + THIRTY_TWO + '".slow(1);\n';
      const safe = 'n("' + THIRTY_TWO + '")';
      expect(formatted(unsafe + safe)).toBe(unsafe + formatted(safe));
    }
  });
});
