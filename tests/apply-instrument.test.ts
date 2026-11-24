import { expect, test } from 'bun:test';
import { instrumentApplication } from '../src/lib/editor/apply-instrument';

test('wraps a complete nested multiline expression using editor offsets', () => {
  const expression = 'note("c4 e4")\n    .gain(.7)';
  const source = `// 🎶\n$: stack(\n  ${expression},\n  s("bd")\n)\n`;
  const from = source.indexOf(expression);
  const result = instrumentApplication(source, from - 2, from + expression.length, 'my_lead');
  expect(result).toEqual({
    from,
    to: from + expression.length,
    insert: `(${expression}).apply(instruments.my_lead())`,
  });
});

test('rejects partial tokens, metadata, bindings, and unsafe replacement contexts', () => {
  const cases = [
    ['note("c4 e4")', 'c4 e4'],
    ['note("c4 e4")', 'note'],
    ['(note)("c4 e4")', '(note)'],
    ['const count = (4); note("c4 e4")', '(4)'],
    ['const meta = { bpm: getBpm() };\nnote("c4")', 'getBpm()'],
    ['const part = note("c4");', 'part'],
    ['voice.lead', 'lead'],
    ['init()\nvoice', 'voice'],
    ['note("c4"); s("bd")', 'note("c4"); s("bd")'],
  ];
  for (const [source, selection] of cases) {
    const from = source.indexOf(selection);
    expect(() => instrumentApplication(source, from, from + selection.length, 'lead')).toThrow();
  }
});

test('refuses to inject the pinned lookup if a song declares an instruments namespace', () => {
  const expression = 'note("c4")';
  for (const source of [
    `const { presets: instruments } = app;
${expression}`,
    `const C = class instruments { method() { return ${expression}; } };`,
  ]) {
    const from = source.indexOf(expression);
    expect(() => instrumentApplication(source, from, from + expression.length, 'lead')).toThrow(
      'has its own instruments namespace',
    );
  }
});

test('allows a target outside a shadowing scope but detects a visible hoisted binding', () => {
  const expression = 'note("c4")';
  const sources = [
    `const route = instruments => instruments.id;
${expression}`,
    `{ let instruments; }
${expression}`,
  ];
  for (const source of sources) {
    const from = source.indexOf(expression);
    expect(instrumentApplication(source, from, source.length, 'lead').insert).toContain(
      '.apply(instruments.lead())',
    );
  }
  const source = `function pattern() {
  if (false) { var instruments; }
  return ${expression};
}`;
  const from = source.indexOf(expression);
  expect(() => instrumentApplication(source, from, from + expression.length, 'lead')).toThrow(
    'has its own instruments namespace',
  );
});

test('accepts whole enabled mini patterns and explicitly constructed patterns', () => {
  const expressions = ['"c4 e4"', '`c4 e4`', 'new Pattern(state => note("c4").query(state))'];
  for (const expression of expressions) {
    expect(instrumentApplication(expression, 0, expression.length, 'lead').insert).toBe(
      `(${expression}).apply(instruments.lead())`,
    );
  }
  const literals = ['// mini-off\n"c4 e4"', "'c4 e4'", '"use strict"'];
  for (const source of literals) {
    const from = source.startsWith('//') ? source.indexOf('"') : 0;
    expect(() => instrumentApplication(source, from, source.length, 'lead')).toThrow();
  }
});
