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
  const source = `const { presets: instruments } = app;\n${expression}`;
  const from = source.indexOf(expression);
  expect(() => instrumentApplication(source, from, source.length, 'lead')).toThrow(
    'declares its own instruments namespace',
  );
});
