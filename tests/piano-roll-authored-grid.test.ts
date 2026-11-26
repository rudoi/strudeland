import { expect, test } from 'bun:test';
import { note, noteToMidi, silence, stack, stepcat } from '@strudel/core';
import { planRollEdit, resolveRollTarget } from '../src/lib/piano-roll/source';
import { decodeNoteGrid, noteGrid } from '../src/lib/strudel/note-grid';
import type { RollEdit } from '../src/lib/piano-roll/types';

// Different pitches, names and row spacing from the local song, with a 27-slot opening.
const rows = [
  'F4 ' + Array(15).fill('_').join('  '),
  Array(11).fill('_').join(' ') + '  D4:2 _ _ _ _',
  '~ _ ~ E4:0.30   _ _ _ _ _ _ _ _ _ _ _ _',
];
const fixture =
  `// 🎶 rows\nconst score = [\n${rows.map((row) => `  '${row}',`).join('\n')}\n];` +
  String.raw`
const flattened = score.flatMap((line, n) => {
  const parts = line.trim().split(/\s+/);
  if (parts.length !== 16) throw new Error('incorrect width');
  return parts;
});
const events = [];
let preceding;
for (let i = 0; i < flattened.length;) {
  const token = flattened[i];
  let size = 1;
  while (flattened[i + size] === '_') size++;
  if (token === '~' || token === '_') {
    events.push([size, silence]);
  } else {
    const [pitch, slide] = token.split(':');
    const interval = slide && preceding ? noteToMidi(pitch) - noteToMidi(preceding) : 0.55;
    events.push([size, note(pitch).penv(interval).pattack(slide ? Number(slide) : 0.09)]);
    preceding = pitch;
  }
  i += size;
}
const flute = stepcat(...events).slow(score.length);
`;

function target(code: string, location: string) {
  const pos = code.indexOf(location);
  expect(pos).toBeGreaterThanOrEqual(0);
  return resolveRollTarget(code, { from: pos, to: pos });
}

function edited(code: string, location: string, edit: RollEdit): string {
  const owner = target(code, location)!;
  const patches = planRollEdit(code, owner.expression, edit);
  expect(patches).toBeTruthy();
  for (const patch of [...patches!].reverse()) {
    code = code.slice(0, patch.from) + patch.insert + code.slice(patch.to);
  }
  return code;
}

function legacy(code: string) {
  return new Function('stepcat', 'note', 'silence', 'noteToMidi', code + '\nreturn flute;')(
    stepcat,
    note,
    silence,
    noteToMidi,
  );
}

type Sound = { note: string; penv?: number; pattack?: number; s?: string };
type TestHap = {
  whole: { begin: { toFraction(): string }; end: { toFraction(): string } };
  value: Sound;
  hasOnset(): boolean;
};
function inspect(pattern: { queryArc(a: number, b: number): TestHap[] }, a = 0, b = 3) {
  return pattern
    .queryArc(a, b)
    .filter((h) => h.hasOnset())
    .map((h) => ({
      start: h.whole.begin.toFraction(),
      end: h.whole.end.toFraction(),
      value: h.value,
    }));
}

test('a row caret owns the entire legacy pattern and projects the same intervals as the runtime', () => {
  const selected = target(fixture, rows[1])!;
  expect(selected.gridRow).toBe(1);
  expect(selected.phrase?.bars).toBe(3);
  expect(fixture.slice(selected.expression.from, selected.expression.to)).toBe(
    'stepcat(...events).slow(score.length)',
  );
  const sounds = inspect(legacy(fixture));
  expect(sounds).toEqual(inspect(noteGrid(rows)));
  expect(sounds).toEqual([
    { start: '0', end: '27/16', value: { note: 'F4', penv: 0.55, pattack: 0.09 } },
    { start: '27/16', end: '2', value: { note: 'D4', penv: -3, pattack: 2 } },
    { start: '35/16', end: '3', value: { note: 'E4', penv: 2, pattack: 0.3 } },
  ]);
  expect(
    selected.phrase?.notes.map((n) => [
      n.start,
      n.end,
      fixture.slice(n.origin!.from, n.origin!.to),
    ]),
  ).toEqual(sounds.map((n) => [n.start, n.end, n.value.note]));
  const at = fixture.indexOf('D4:2');
  expect(resolveRollTarget(fixture, { from: at, to: at + 4 })?.expression).toEqual(
    selected.expression,
  );
});

test('retuning updates the two layered pitches and next audible glide, across rests', () => {
  const after = edited(fixture, 'D4:2', { kind: 'pitch', id: 'grid-27', pitch: 64 });
  expect(after).toBe(fixture.replace('D4:2', 'E4:2'));
  const layered = (source: string) => {
    const line = legacy(source);
    return stack(line.s('sine'), line.withValue((v: Sound) => ({ note: v.note })).s('white'));
  };
  const before = inspect(layered(fixture));
  const expected = before.map((event) => {
    const value = { ...event.value };
    if (event.start === '27/16') {
      value.note = 'E4';
      if (value.s === 'sine') value.penv = -1;
    }
    if (event.start === '35/16' && value.s === 'sine') value.penv = 0;
    return { ...event, value };
  });
  expect(inspect(layered(after))).toEqual(expected);
  expect(inspect(layered(after), 1, 2)).toEqual(expected.filter((e) => e.start === '27/16'));
});

test('consumer proof checks operators, binding flow, widths, plain strings and shadowing', () => {
  for (const changed of [
    fixture.replace('!== 16', '!== 8'),
    fixture.replace('score.length);', '2);'),
    fixture.replace('0.09', '0.08'),
    fixture.replace('preceding = pitch;', 'preceding = token;'),
    fixture.replace('flattened[i + size]', 'flattened[i + 1]'),
    fixture.replace("token === '~'", 'token === "~"'),
    fixture + '\nscore.reverse();',
    'const noteToMidi = () => 0;\n' + fixture,
    fixture + "\nregister('pattack',()=>silence);",
    fixture.replace('D4:2', 'D4:Infinity'),
    fixture.replace('F4 ', 'F4 ~ '),
    "const score = ['F4 _ _ ~']; score.reverse();",
  ])
    expect(target(changed, 'F4')?.phrase).toBeUndefined();
  for (const changed of [
    "const rows=['F4 _']; rows.reverse(); noteGrid(rows);",
    "const noteGrid = () => null; noteGrid(['F4 _']);",
    'noteGrid(["F4 _"]);',
    "noteGrid(['F4 _','G4']);",
  ])
    expect(target(changed, 'F4')?.phrase).toBeUndefined();
});

test('grid source moves and resizes across rows, retains trivia, and releases holds into silence', () => {
  const code = "const line=noteGrid([ 'c4:0.30   _  ~ ~', // row 1\n  '~ ~  ~ _', ]);";
  const gesture = { kind: 'move', id: 'grid-0', start: '3/4', end: '5/4' } as const;
  const owner = target(code, 'c4:')!;
  const patches = planRollEdit(code, owner.expression, gesture)!;
  expect(patches.find(({ reveal }) => reveal)?.insert).toBe('c4:0.30');
  const moved = edited(code, 'c4:', gesture);
  expect(moved).toBe("const line=noteGrid([ '~   _  ~ c4:0.30', // row 1\n  '_ ~  ~ _', ]);");
  const resized = edited(moved, 'c4:', { kind: 'resize', id: 'grid-3', end: '3/2' });
  expect(resized).toBe(moved.replace("'_ ~  ~ _'", "'_ _  ~ _'"));
  const deleted = edited(resized, 'c4:', { kind: 'delete', id: 'grid-3' });
  expect(target(deleted, '~   _')?.phrase?.notes).toEqual([]);
  const inserted = edited(deleted, '~   _', {
    kind: 'insert',
    start: '5/4',
    end: '7/4',
    pitch: 62,
  });
  expect(target(inserted, 'd4')?.phrase?.notes.map((n) => [n.start, n.end])).toEqual([
    ['5/4', '7/4'],
  ]);
  const cuts = "noteGrid(['~ _ _ _'])";
  expect(edited(cuts, '~ _', { kind: 'insert', start: '1/4', end: '1/2', pitch: 62 })).toBe(
    "noteGrid(['~ d4 ~ _'])",
  );
});

test('moves both edges to new slots across meters, retains the suffix and recomputes adjacent glides', () => {
  const score = ['E4:0.30 _ _ ~ G4:1.25 _ ~ ~', '~ ~ ~ ~ ~ ~ ~ ~', '~ ~ ~ ~ ~ ~ ~ ~'];
  const code = `const line = noteGrid([${score.map((row) => `'${row}'`).join(', ')}]);`;
  // Three slots in 4/4 and four slots in 6/8 both last one and a half quarter notes.
  const changed = edited(code, 'E4:0.30', {
    kind: 'move',
    id: 'grid-0',
    start: '7/4',
    end: '9/4',
  });
  const newScore = ['~ _ _ ~ G4:1.25 _ ~ ~', '~ ~ ~ ~ ~ ~ E4:0.30 _', '_ _ ~ ~ ~ ~ ~ ~'];
  expect(changed).toBe(`const line = noteGrid([${newScore.map((row) => `'${row}'`).join(', ')}]);`);
  expect(inspect(noteGrid(newScore))).toEqual([
    { start: '1/2', end: '3/4', value: { note: 'G4', penv: 0.55, pattack: 1.25 } },
    { start: '7/4', end: '9/4', value: { note: 'E4', penv: -3, pattack: 0.3 } },
  ]);
  const owner = target(changed, 'E4:0.30')!;
  const backward = { kind: 'move', id: 'grid-14', start: '0', end: '3/8' } as const;
  const patches = planRollEdit(changed, owner.expression, backward)!;
  expect(patches[0]).toMatchObject({ insert: 'E4:0.30', reveal: true });
  const restored = edited(changed, 'E4:0.30', backward);
  expect(target(restored, 'E4:0.30')?.phrase?.notes).toMatchObject([
    { pitch: 64, start: '0', end: '3/8', glideBend: 0.55 },
    { pitch: 67, start: '1/2', end: '3/4', glideBend: 3 },
  ]);
});

test('src preserves exact suffix, accidental style and carry-in, routes collisions and off-slot moves to fallback', () => {
  const code = "const rows = [ 'Bb4:0.20   _', '_ A4:0' ]; const a=noteGrid(rows);";
  expect(edited(code, '_ A4', { kind: 'pitch', id: 'grid-0', pitch: 61 })).toBe(
    code.replace('Bb4', 'Db4'),
  );
  const owner = target(code, '_ A4')!;
  expect(owner.gridRow).toBe(1);
  expect(owner.phrase?.notes[0].end).toBe('3/2');
  for (const [start, end] of [
    ['1/2', '2'],
    ['1/3', '1'],
    ['1/2', '4/3'],
    ['1/2', '5/2'],
    ['1/2', '1/0'],
    ['1/2', '1/2'],
  ]) {
    expect(
      planRollEdit(code, owner.expression, { kind: 'move', id: 'grid-0', start, end }),
    ).toBeNull();
  }
  for (const broken of ['c4:NaN', 'c4:2:3', 'c4:-0.2', '128', '_abc']) {
    expect(() => decodeNoteGrid(['~ ' + broken])).toThrow('row 1, slot 2');
  }
  expect(decodeNoteGrid(['_ _', 'd4:0 ~', '_ c4:0']).runs.map((r) => r.tone)).toEqual([
    undefined,
    { spelling: 'd4', midi: 62, bend: 0.55, attack: 0 },
    undefined,
    { spelling: 'c4', midi: 60, bend: -2, attack: 0 },
  ]);
});

test('a saved edit around a producer remains the inspection expression at its original rows', () => {
  const wrap = '._pianoEdits({ entries: [] })';
  for (const [code, token] of [
    [fixture.replace('slow(score.length);', 'slow(score.length)' + wrap + ';'), 'F4'],
    ["const a=(noteGrid(['c4 _']))" + wrap + ';', 'c4'],
  ]) {
    const owner = target(code, token)!;
    expect(code.slice(owner.expression.from, owner.expression.to)).toEndWith(wrap);
    expect(owner.phrase).toBeUndefined();
    expect(
      planRollEdit(code, owner.expression, { kind: 'pitch', id: 'grid-0', pitch: 65 }),
    ).toBeNull();
  }
});
