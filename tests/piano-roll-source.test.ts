import { describe, expect, test } from 'bun:test';
import { planRollEdit, resolveRollTarget, targetsDefinition } from '../src/lib/piano-roll/source';
import type { RollEdit, RollPhrase, SourceSpan } from '../src/lib/piano-roll/types';

function span(source: string, selection: string, from = source.indexOf(selection)): SourceSpan {
  return { from, to: from + selection.length };
}

function editable(source: string, selection = source) {
  const target = resolveRollTarget(source, span(source, selection));
  if (!target?.phrase) throw new Error(`Expected an authored phrase for ${selection}`);
  return { target, phrase: target.phrase };
}

function events(phrase: RollPhrase) {
  return phrase.notes.map(({ pitch, start, end }) => [pitch, start, end]);
}

function edit(source: string, operation: RollEdit, target = span(source, source)) {
  const patches = planRollEdit(source, target, operation);
  if (!patches) throw new Error(`Expected a patch for ${JSON.stringify(operation)}`);
  return patches
    .slice()
    .reverse()
    .reduce((text, { from, to, insert }) => text.slice(0, from) + insert + text.slice(to), source);
}

describe('piano roll source projection', () => {
  test('lifts a written pitch from a host-only alternating phrase', () => {
    const source = 'note("<c4@2 d4>")';
    expect(resolveRollTarget(source, span(source, 'd4'))).toMatchObject({
      expression: { from: 0, to: source.length },
    });
    expect(resolveRollTarget(source, span(source, 'd4'))?.phrase).toBeUndefined();
  });
  test('resolves an exact expression and complete pitch atom with UTF-16 source positions', () => {
    const source = '// 🎵\nlead: note(`  c4    e4  `).s("saw")';
    const { phrase } = editable(source, 'lead: note(`  c4    e4  `).s("saw")');
    expect(events(phrase)).toEqual([
      [60, '0', '1/2'],
      [64, '1/2', '1'],
    ]);
    expect(phrase.notes.map(({ origin, sound }) => [origin, sound])).toEqual([
      [span(source, 'c4'), 'saw'],
      [span(source, 'e4'), 'saw'],
    ]);
    const atom = editable(source, 'e4');
    expect(source.slice(atom.target.expression.from, atom.target.expression.to)).toBe(
      'note(`  c4    e4  `)',
    );
    expect(resolveRollTarget(source, span(source, 'e4  `).s'))).toBeNull();
    expect(resolveRollTarget(source, span(source, '"saw"'))).toBeNull();
  });

  test('trims selected trivia around a pattern and ignores a caret in comments', () => {
    const source = '/* lead */  note("c4")  /* tail */';
    expect(editable(source).target.expression).toEqual(span(source, 'note("c4")'));
    expect(resolveRollTarget(source, { from: 4, to: 4 })).toBeNull();
  });

  test('opens an authored phrase at a caret inside notes, whitespace, and its closing delimiter', () => {
    const source = 'const melody = note("c4 ~ e4 ~");';
    const note = span(source, 'note("c4 ~ e4 ~")');
    for (const position of [note.from, source.indexOf('c4') + 1, source.indexOf(' ~'), note.to]) {
      expect(resolveRollTarget(source, { from: position, to: position })?.expression).toEqual(note);
    }
    expect(resolveRollTarget(source, { from: -1, to: -1 })).toBeNull();
  });

  test('resolves a transformed pattern and aliases while avoiding effect arguments', () => {
    const source = [
      'const melody = note("c4");',
      'const fast = melody.fast(2).gain(rand.range(0.05, 0.1));',
      'song(meta, { intro: fast })',
    ].join('\n');
    function at(fragment: string, last = false) {
      const from = (last ? source.lastIndexOf(fragment) : source.indexOf(fragment)) + 1;
      return resolveRollTarget(source, { from, to: from });
    }
    expect(at('melody')?.phrase?.notes[0].pitch).toBe(60);
    expect(at('fast', true)?.definition?.name).toBe('melody');
    expect(at('0.05')).toBeNull();
    expect(at('range')).toBeNull();
    expect(at('intro')).toBeNull();
  });

  test('keeps a saved event edit as the target when the caret is in its original expression', () => {
    const source = '(note("c4"))._pianoEdits({ entries: [] })';
    for (const fragment of ['c4', 'entries']) {
      const from = source.indexOf(fragment) + 1;
      expect(resolveRollTarget(source, { from, to: from })?.expression).toEqual(
        span(source, source),
      );
    }
  });

  test('opens a carried note from the second grid row and edits where the pitch originated', () => {
    const source = [
      "const rows = ['c4 _ ~ g4:0.20', '_ ~ ~ ~'];",
      'const melody = noteGrid(rows);',
      'song(meta, { intro: melody });',
    ].join('\n');
    const rowTwo = source.indexOf('_ ~ ~ ~');
    const target = resolveRollTarget(source, { from: rowTwo, to: rowTwo });
    expect(target).toMatchObject({
      expression: span(source, 'noteGrid(rows)'),
      label: 'rows',
      gridRow: 1,
      gridBars: 2,
      sections: ['intro'],
      definition: { name: 'melody' },
    });
    const carry = target?.phrase?.notes.find(({ pitch }) => pitch === 67);
    expect(carry).toMatchObject({ start: '3/4', end: '5/4' });
    const result = edit(source, { kind: 'pitch', id: carry!.id, pitch: 69 }, target?.expression);
    expect(result).toBe(source.replace('g4:0.20', 'a4:0.20'));

    const use = span(source, 'melody', source.lastIndexOf('melody'));
    expect(resolveRollTarget(source, use)).toMatchObject({
      gridBars: 2,
      definition: { expression: target?.expression, uses: [{ span: use, sections: ['intro'] }] },
    });
  });

  test('derives the written grid meter anchor through a voice helper and flags uncertain uses', () => {
    const source = [
      "const rows = ['c4 _ ~ g4'];",
      'const melody = noteGrid(rows);',
      'const intro = stack(myVoice(melody));',
      'song(meta, { intro });',
    ].join('\n');
    const from = source.indexOf('g4');
    const target = resolveRollTarget(source, { from, to: from });
    expect(target?.sections).toEqual(['intro']);
    expect(target?.definition?.incomplete).toBe(true);
  });

  test('queries the producer when a recorded event edit wraps a grid', () => {
    const source =
      "const rows = ['c4 _ ~ g4']; const melody = (noteGrid(rows))._pianoEdits({ entries: [] });";
    const from = source.indexOf('g4');
    const target = resolveRollTarget(source, { from, to: from });
    expect(target?.gridBars).toBe(1);
    expect(target?.phrase).toBeUndefined();
    expect(source.slice(target?.expression.from, target?.expression.to)).toBe(
      '(noteGrid(rows))._pianoEdits({ entries: [] })',
    );
  });

  test('recognizes an authored repeated pitch without attributing Euclidean operands', () => {
    const source = 'note("c4!2 e4(2,3)")';
    const { phrase } = editable(source);
    const repetitions = phrase.notes.filter((note) => note.pitch === 60);
    expect(repetitions).toHaveLength(2);
    expect(repetitions[0].origin).toEqual(span(source, 'c4'));
    expect(repetitions[1].origin).toEqual(repetitions[0].origin);
    expect(phrase.notes.find((note) => note.pitch === 64)?.origin).toEqual(span(source, 'e4'));
    expect(phrase.notes.every((note) => !note.timeEditable)).toBe(true);
    const result = edit(source, { kind: 'pitch', id: repetitions[1].id, pitch: 62 });
    expect(result).toBe('note("d4!2 e4(2,3)")');
  });

  test('finds the shared definition through immutable aliases and records written sections', () => {
    const source = [
      'const motif = note("c4 e4");',
      'const direct = motif;',
      'const high = motif.transpose(12);',
      'song(meta, {intro: direct, hook: high})',
    ].join('\n');
    const directUse = resolveRollTarget(
      source,
      span(source, 'direct', source.lastIndexOf('direct')),
    );
    const highUse = resolveRollTarget(source, span(source, 'high', source.lastIndexOf('high')));
    expect(directUse?.sections).toEqual(['intro']);
    expect(events(directUse!.phrase!)).toEqual([
      [60, '0', '1/2'],
      [64, '1/2', '1'],
    ]);
    expect(highUse?.phrase).toBeUndefined();
    expect(highUse?.sections).toEqual(['hook']);
    expect(highUse?.definition?.expression).toEqual(span(source, 'note("c4 e4")'));
    expect(highUse?.definition?.uses.map((use) => use.sections)).toEqual([
      ['intro'],
      ['hook'],
      ['intro'],
      ['hook'],
    ]);
    expect(highUse?.definition?.incomplete).toBe(false);
  });

  test('a caret inside a shared receiver edits its authored definition', () => {
    const source = 'const melody = note("c4 ~ e4 ~").s("triangle");\n$: melody';
    const from = source.indexOf('e4') + 1;
    const authored = resolveRollTarget(source, { from, to: from })!;
    expect(authored.expression).toEqual(span(source, 'note("c4 ~ e4 ~")'));
    expect(authored.definition?.expression).toEqual(
      span(source, 'note("c4 ~ e4 ~").s("triangle")'),
    );
    expect(targetsDefinition(authored)).toBe(true);
    expect(
      edit(
        source,
        { kind: 'pitch', id: authored.phrase!.notes[1]!.id, pitch: 65 },
        authored.expression,
      ),
    ).toBe(source.replace('e4', 'f4'));

    const use = resolveRollTarget(source, span(source, 'melody', source.lastIndexOf('melody')))!;
    expect(targetsDefinition(use)).toBe(false);
  });

  test('ignores shadowed aliases and exposes a potentially escaping reference', () => {
    const source = [
      'const motif = note("c4");',
      'const outside = () => { const motif = note("f4"); return motif; };',
      'unknown(motif);',
      'song(meta, {verse: motif})',
    ].join('\n');
    const main = resolveRollTarget(source, span(source, 'motif', source.lastIndexOf('motif')));
    expect(main?.definition?.incomplete).toBe(true);
    expect(main?.definition?.uses.map(({ span, sections }) => [span.from, sections])).toEqual([
      [source.indexOf('motif);'), []],
      [source.lastIndexOf('motif'), ['verse']],
    ]);
    const shadowed = resolveRollTarget(source, span(source, 'motif', source.indexOf('motif;')));
    expect(shadowed?.definition?.expression).toEqual(span(source, 'note("f4")'));
  });

  test('retains shared use disclosures when selecting one authored pitch inside a larger expression', () => {
    const source = 'const melody = note(seq("c4", "e4")).s("saw"); song(meta, {intro: melody})';
    const { target, phrase } = editable(source, 'e4');
    expect(target.expression).toEqual(span(source, '"e4"'));
    expect(target.definition?.expression).toEqual(target.expression);
    expect(target.definition?.uses).toHaveLength(1);
    expect(target.sections).toEqual(['intro']);
    expect(
      edit(source, { kind: 'pitch', id: phrase.notes[0].id, pitch: 65 }, target.expression),
    ).toBe('const melody = note(seq("c4", "f4")).s("saw"); song(meta, {intro: melody})');

    const wrapped = 'const melody = note("e4").s("saw"); song(meta, {intro: melody})';
    const selected = editable(wrapped, 'e4').target;
    expect(selected.definition?.expression).toEqual(span(wrapped, 'note("e4").s("saw")'));
    expect(selected.definition?.phrase?.notes[0].sound).toBe('saw');
    expect(selected.sections).toEqual(['intro']);
  });

  test('offers inspection only for unsafe pitch input and bound helpers', () => {
    for (const [source, selection] of [
      ['const m = () => 5; note("c4")', 'note("c4")'],
      ['const note = f; note("c4")', 'note("c4")'],
      ['register(\'note\', f); s("sine").note("c4")', 's("sine").note("c4")'],
      ['register(control, f); note("c4")', 'note("c4")'],
      ['function f(note) { return note("c4"); }', 'note("c4")'],
      ['/* mini-off */ note("c4 e4")', 'note("c4 e4")'],
      ['note(`c4 ${foo}`)', 'note(`c4 ${foo}`)'],
      [String.raw`note("c\u0034")`, String.raw`note("c\u0034")`],
      ['note("c4 | e4")', 'note("c4 | e4")'],
      ['note("c4*999999")', 'note("c4*999999")'],
    ]) {
      const target = resolveRollTarget(source, span(source, selection));
      expect(target?.expression).toEqual(span(source, selection));
      expect(target?.phrase).toBeUndefined();
    }
  });

  test('queries receiver timing, numeric strings and periods that local syntax cannot prove', () => {
    for (const source of [
      's("sine").note("c4 ~ e4 ~")',
      "note('60')",
      'note("<c4@2 d4>")',
      'note("[<c4 d4> e4]")',
      'note("c4/2")',
    ]) {
      const target = resolveRollTarget(source, span(source, source));
      expect(target?.expression).toEqual(span(source, source));
      expect(target?.phrase).toBeUndefined();
    }
  });

  test('lifts dynamic literal and numeric sequence selections to the pitch-producing call', () => {
    for (const [source, selection] of [
      ['note(seq("c4|e4", "g4"))', '"c4|e4"'],
      ['note(seq(60, 62))', 'seq(60, 62)'],
      ['s("sine").note("c4 ~ e4 ~")', '"c4 ~ e4 ~"'],
    ]) {
      const target = resolveRollTarget(source, span(source, selection));
      expect(target?.expression).toEqual(span(source, source));
      expect(target?.phrase).toBeUndefined();
    }
  });

  test('disables source edits when an authored expression escapes a resolvable definition', () => {
    const expression = 'note("c4 e4")';
    for (const source of [
      `let motif = ${expression}; stack(motif, motif)`,
      `const parts = { motif: ${expression} }; stack(parts.motif, parts.motif)`,
      `unknown(${expression})`,
    ]) {
      const target = resolveRollTarget(source, span(source, expression));
      expect(target?.expression).toEqual(span(source, expression));
      expect(target?.phrase).toBeUndefined();
      expect(
        planRollEdit(source, span(source, expression), {
          kind: 'insert',
          start: '0',
          end: '1/8',
          pitch: 60,
        }),
      ).toBeNull();
    }
  });
});

describe('piano roll exact phrase edits', () => {
  test('moves and resizes notes while other intervals stay fixed', () => {
    const source = 'note("c4 ~ e4 ~")';
    const { phrase } = editable(source);
    const id = phrase.notes[1].id;
    const moved = edit(source, { kind: 'move', id, start: '3/4' });
    const resized = edit(source, { kind: 'resize', id, end: '7/8' });
    expect(moved).toBe('note("c4 ~@2 e4")');
    expect(events(editable(moved).phrase)).toEqual([
      [60, '0', '1/4'],
      [64, '3/4', '1'],
    ]);
    expect(events(editable(resized).phrase)).toEqual([
      [60, '0', '1/4'],
      [64, '1/2', '7/8'],
    ]);
    expect(
      planRollEdit(source, span(source, source), { kind: 'move', id, start: '1/8' }),
    ).toBeNull();
    expect(
      planRollEdit(source, span(source, source), { kind: 'resize', id, end: '9/8' }),
    ).toBeNull();
  });

  test('inserts into rests and deletes notes without crossing the bar', () => {
    const source = 'note("~")';
    const inserted = edit(source, { kind: 'insert', start: '2/7', end: '4/7', pitch: 64 });
    expect(inserted).toBe('note("~@2 e4@2 ~@3")');
    const note = editable(inserted).phrase.notes[0];
    expect(events(editable(inserted).phrase)).toEqual([[64, '2/7', '4/7']]);
    const deleted = edit(inserted, { kind: 'delete', id: note.id });
    expect(deleted).toBe(source);
    expect(editable(deleted).phrase.insertable).toBe(true);
    expect(editable('note("~").s("square")').phrase.sound).toBe('square');
    expect(
      planRollEdit(source, span(source, source), {
        kind: 'insert',
        start: '1/0',
        end: '1',
        pitch: 64,
      }),
    ).toBeNull();
    expect(
      planRollEdit(source, span(source, source), {
        kind: 'insert',
        start: '1/257',
        end: '2/257',
        pitch: 64,
      }),
    ).toBeNull();
  });

  test('touches only the edited bar and keeps its enclosing whitespace', () => {
    const source = 'note(`<\n  [  c4 ~ e4 ~  ]\n  [~@5 g4 ~]\n>`)';
    const { phrase } = editable(source);
    const result = edit(source, { kind: 'move', id: phrase.notes[1].id, start: '3/4' });
    expect(result).toBe('note(`<\n  [  c4 ~@2 e4  ]\n  [~@5 g4 ~]\n>`)');
    expect(events(editable(result).phrase)).toEqual([
      [60, '0', '1/4'],
      [64, '3/4', '1'],
      [67, '12/7', '13/7'],
    ]);
  });

  test('inserts into one bare step of a slow sequence without changing the bar count', () => {
    const source = 'note("<c4 ~ f4>")';
    expect(editable(source).phrase.bars).toBe(3);
    const result = edit(source, { kind: 'insert', start: '5/4', end: '3/2', pitch: 64 });
    expect(result).toBe('note("<c4 [~ e4 ~@2] f4>")');
    expect(events(editable(result).phrase)).toEqual([
      [60, '0', '1'],
      [64, '5/4', '3/2'],
      [65, '2', '3'],
    ]);
  });

  test('replaces one chord member and preserves overlapping duplicates', () => {
    const source = 'note("[c4,c4] ~")';
    const { phrase } = editable(source);
    const id = phrase.notes[1].id;
    expect(phrase.insertable).toBe(false);
    expect(planRollEdit(source, span(source, source), { kind: 'delete', id })).toBeNull();
    const result = edit(source, { kind: 'pitch', id, pitch: 64 });
    expect(result).toBe('note("[c4,e4] ~")');
    expect(events(editable(result).phrase)).toEqual([
      [60, '0', '1/2'],
      [64, '0', '1/2'],
    ]);
  });

  test('a flat accidental spelling keeps the B root for a natural retune', () => {
    const source = 'note("df4")';
    const note = editable(source).phrase.notes[0];
    expect(edit(source, { kind: 'pitch', id: note.id, pitch: 71 })).toBe('note("b4")');
  });

  test('retains direct MIDI and pitch spelling styles', () => {
    const numeric = 'note(64)';
    const numericId = editable(numeric).phrase.notes[0].id;
    expect(edit(numeric, { kind: 'pitch', id: numericId, pitch: 67 })).toBe('note(67)');
    const spelled = 'note("Eb4")';
    const spelledId = editable(spelled).phrase.notes[0].id;
    expect(edit(spelled, { kind: 'pitch', id: spelledId, pitch: 70 })).toBe('note("Bb4")');
  });
});
