import { expect, test } from 'bun:test';
import { stack } from '@strudel/core';
import { m } from '@strudel/mini';
import { getLanguages, transpiler } from '@strudel/transpiler';
import { formatSong } from '../src/lib/editor/format-song';

const SIXTEEN = 'c4 ~ e4 ~ g4 ~ e4 ~ a4 ~ g4 ~ e4 ~ d4 ~';
const THIRTY_TWO = `${SIXTEEN} ${SIXTEEN}`;
const ROW = 'c4 ~  e4 ~   g4 ~  e4 ~     a4 ~  g4 ~   e4 ~  d4 ~';
const SIXTEENTHS = ROW.replace('     ', '   ');

function formatted(source: string, { mini = false } = {}): string {
  const edits = formatSong(source);
  const editable = mini ? /^\s*["`]?\s*$/u : /^\s*$/u;
  let end = 0;
  for (const { from, to, insert } of edits) {
    expect(from).toBeGreaterThanOrEqual(end);
    expect(to).toBeGreaterThanOrEqual(from);
    expect(to).toBeLessThanOrEqual(source.length);
    expect(source.slice(from, to)).toMatch(editable);
    expect(insert).toMatch(editable);
    end = to;
  }
  const result = edits.reduceRight(
    (text, { from, to, insert }) => text.slice(0, from) + insert + text.slice(to),
    source,
  );
  expect(formatSong(result)).toEqual([]);
  return result;
}

test('formats label, variable and callback chains with two-space continuations', () => {
  const source = [
    'lead: note("c4 e4").s(\'triangle\').gain(.8)',
    '',
    'const pattern = note("g4").slow(2);',
    'all(x => x.lpf(800).room(.25));',
    '$: Math.round(1.5)',
  ].join('\n');
  expect(formatted(source)).toBe(
    [
      'lead: note("c4 e4")',
      "  .s('triangle')",
      '  .gain(.8)',
      '',
      'const pattern = note("g4")',
      '  .slow(2);',
      'all(x => x',
      '  .lpf(800)',
      '  .room(.25));',
      '$: Math.round(1.5)',
    ].join('\n'),
  );
});

test('formats meta and song parts, including a nested multiline combiner', () => {
  const source = [
    "const meta={title:'Sketch',sections:[{name:'intro',bars:2},{name:'body',bars:4,loop:true}],bpm:122}",
    '',
    `$: song(meta,{intro:note("c4").s('triangle'),body:stack(s("bd").gain(.8),note("e4").room(.2))})`,
  ].join('\n');
  expect(formatted(source)).toBe(
    [
      'const meta = {',
      "  title: 'Sketch',",
      '  sections: [',
      "    { name: 'intro', bars: 2 },",
      "    { name: 'body', bars: 4, loop: true }",
      '  ],',
      '  bpm: 122',
      '}',
      '',
      '$: song(meta, {',
      '  intro: note("c4")',
      "    .s('triangle'),",
      '  body: stack(',
      '    s("bd")',
      '      .gain(.8),',
      '    note("e4")',
      '      .room(.2)',
      '  )',
      '})',
    ].join('\n'),
  );
});

test('expands recognized unqualified combiners and preserves a compact call with no expanded argument', () => {
  for (const call of ['stack', 'seq', 'cat']) {
    const source = `$: ${call}( (s("bd")),(s("hh")).gain(.8), )`;
    expect(formatted(source)).toBe(
      [`$: ${call}(`, '  (s("bd")),', '  (s("hh"))', '    .gain(.8),', ')'].join('\n'),
    );
    expect(formatSong(`$: ${call}(s("bd"),note("c4"))`)).toEqual([]);
    expect(formatted(`$: other.${call}(s("bd").gain(.8), s("hh"))`)).toBe(
      `$: other.${call}(s("bd")\n  .gain(.8), s("hh"))`,
    );
  }
});

test('aligns a combiner to its opening parenthesis even when it starts on a separate line', () => {
  expect(formatted('$: stack\n    (s("bd").gain(.8), s("hh"))')).toBe(
    '$: stack\n    (\n      s("bd")\n        .gain(.8),\n      s("hh")\n    )',
  );
});

test('compacts simple section objects and keeps commented sections on separate lines', () => {
  const source = [
    'const meta={sections:[{',
    "  name:'intro',bars:2},",
    "  {name:'body',",
    '   // memo',
    '   bars:4,loop:true}]}',
  ].join('\n');
  expect(formatted(source)).toBe(
    [
      'const meta = {',
      '  sections: [',
      "    { name: 'intro', bars: 2 },",
      '    {',
      "      name: 'body',",
      '   // memo',
      '   bars: 4,',
      '      loop: true',
      '    }',
      '  ]',
      '}',
    ].join('\n'),
  );
});

test('uses the resulting receiver line and anchors a chain with a multiline argument at its first line', () => {
  const source = [
    'const a = s("bd").gain(1); const b = s("sd").gain(.8)',
    '$: f(s("bd").gain(.6)).room(.4)',
  ].join('\n');
  expect(formatted(source)).toBe(
    [
      'const a = s("bd")',
      '  .gain(1); const b = s("sd")',
      '    .gain(.8)',
      '$: f(s("bd")',
      '  .gain(.6))',
      '  .room(.4)',
    ].join('\n'),
  );
});

test('keeps comment attachment while formatting an independent voice and retaining disabled mini bytes', () => {
  const commented = [
    '$: s("bd") /* keep this inline */ .gain(.8)',
    "  // .bank('disabled')",
    '  .room(.2) // tail',
  ].join('\n');
  const disabled = `// mini-off\nlead: note("${THIRTY_TWO}").slow(2)\n// mini-on`;
  const original = `${commented}\n\n${disabled}`;
  expect(formatted(original)).toBe(
    `${commented}\n\n// mini-off\nlead: note("${THIRTY_TWO}")\n  .slow(2)\n// mini-on`,
  );
  const inlineComment = '$: stack(s("bd") /* keep inline */ .gain(.8), s("hh"))';
  expect(formatSong(inlineComment)).toEqual([]);
});

test('treats a shadowed meta variable as ordinary JavaScript', () => {
  expect(formatted('all(x => { const meta = x.fast(2).slow(2); return meta; })')).toBe(
    'all(x => { const meta = x\n  .fast(2)\n  .slow(2); return meta; })',
  );
});

test('keeps an invalid header intact and applies JS layout in all meters and with a custom mini language', () => {
  const voice = `$: note("${THIRTY_TWO}")`;
  for (const [header, formattedHeader] of [
    ['const meta={bpm:note("g4").slow(2)}', 'const meta={bpm:note("g4").slow(2)}'],
    ['const meta={meter:"4/4"}', 'const meta={meter:"4/4"}'],
    ["const meta={meter:'6/8'}", "const meta = {\n  meter: '6/8'\n}"],
  ]) {
    expect(formatted(`${header}\n${voice}.gain(.8)`)).toBe(
      `${formattedHeader}\n${voice}\n  .gain(.8)`,
    );
  }

  const languages = getLanguages();
  const previous = languages.get('minilang');
  languages.set('minilang', { name: 'layoutTestCustomMini' });
  try {
    expect(formatted(`${voice}.gain(.8)`)).toBe(`${voice}\n  .gain(.8)`);
  } finally {
    if (previous) languages.set('minilang', previous);
    else languages.delete('minilang');
  }
});

test('keeps malformed JS and opaque expressions intact while formatting a separate chain', () => {
  const opaque = [
    's("bd").gain(.8)?.slow(2).room(.2);',
    's("bd").gain(.8)[fx](2).room(.2);',
    'tag`text ${s("bd").gain(.8)} tail`;',
    '`start ${s("bd").gain(.8)} end`;',
  ].join('\n');
  expect(formatted(`${opaque}\n$: s("hh").gain(.6)`)).toBe(`${opaque}\n$: s("hh")\n  .gain(.6)`);
  expect(formatSong('$: s("hh").gain(.6)\nfoo(')).toEqual([]);
});

test('preserves CRLF, labels, grouping, raw lexical tokens, and an ASI-sensitive return', () => {
  const chain = '$声: ' + String.raw`((s("bd"))).g\u0061in(.8).apply(/a\.b/u)`;
  const source = [
    '/* 🎵 */',
    `'use strict';`,
    'function f() {',
    '  return',
    `  ${chain}`,
    '}',
  ].join('\r\n');
  expect(formatted(source)).toBe(
    [
      '/* 🎵 */',
      `'use strict';`,
      'function f() {',
      '  return',
      '  $声: ((s("bd")))',
      String.raw`    .g\u0061in(.8)`,
      String.raw`    .apply(/a\.b/u)`,
      '}',
    ].join('\r\n'),
  );
});

test('stabilizes chain indentation when a preceding mini literal collapses or moves its closing line', () => {
  for (const [mini, canonical] of [
    [SIXTEEN, SIXTEENTHS],
    [THIRTY_TWO, `\n    ${ROW}\n    ${ROW}\n  `],
  ]) {
    const source = `  f(\`${mini}\n      \`, s("bd").gain(.8))`;
    expect(formatted(source, { mini: true })).toBe(`  f(\`${canonical}\`, s("bd")\n    .gain(.8))`);
  }
});

test('aligns mini rows, quotes, and later methods on the final chain layout', () => {
  const source = `$: stack(note("c4").layer(note("${THIRTY_TWO}")).gain(.8), s("hh"))`;
  expect(formatted(source, { mini: true })).toBe(
    [
      '$: stack(',
      '  note("c4")',
      '    .layer(note(`',
      `      ${ROW}`,
      `      ${ROW}`,
      '    `))',
      '    .gain(.8),',
      '  s("hh")',
      ')',
    ].join('\n'),
  );
});

test('uses the song line ending for new JS rows when the first mini introduces LF', () => {
  const source = `note("${THIRTY_TWO}").gain(.8)\r\n$: s("bd").gain(.6)\r\n`;
  expect(formatted(source, { mini: true })).toBe(
    `note(\`\n  ${ROW}\n  ${ROW}\n\`)\r\n  .gain(.8)\r\n$: s("bd")\r\n  .gain(.6)\r\n`,
  );
});

test('combined JS and mini formatting preserves ordered values and rational hap arcs', () => {
  const source = `stack("${THIRTY_TWO}".slow(2).fast(3), "bd ~".fast(2))`;
  const result = formatted(source, { mini: true });
  const spans = transpiler(result).miniLocations as [number, number][];
  expect(result).toContain('stack(\n  `\n');
  expect(result).toContain('    .slow(2)\n    .fast(3),');
  expect(spans.map(([start]) => result[start])).toEqual(
    `${THIRTY_TWO} bd ~`.split(' ').map((atom) => atom[0]),
  );

  function events(code: string) {
    // Evaluate only the static fixture in the real mini runtime and discard highlight offsets.
    const pattern = Function('m', 'stack', transpiler(code).output)(m, stack);
    return pattern
      .queryArc(0.25, 4.75)
      .map((hap: any) => [
        hap.value,
        hap.whole.begin.toString(),
        hap.whole.end.toString(),
        hap.part.begin.toString(),
        hap.part.end.toString(),
      ]);
  }
  expect(events(result)).toEqual(events(source));
});
