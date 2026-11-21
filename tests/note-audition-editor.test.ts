import { closeBrackets, insertBracket } from '@codemirror/autocomplete';
import { syntaxTreeAvailable } from '@codemirror/language';
import { javascript } from '@codemirror/lang-javascript';
import { EditorSelection, EditorState } from '@codemirror/state';
import { expect, test } from 'bun:test';
import { inspectNoteAudition } from '../src/lib/editor/note-audition';

function editor(marked: string) {
  const cursor = marked.indexOf('|');
  expect(cursor).toBeGreaterThanOrEqual(0);
  return EditorState.create({
    doc: marked.slice(0, cursor) + marked.slice(cursor + 1),
    selection: { anchor: cursor },
    extensions: [javascript(), closeBrackets(), EditorState.allowMultipleSelections.of(true)],
  });
}

const type = (state: EditorState, text: string, userEvent = 'input.type') =>
  state.update(state.replaceSelection(text), { userEvent });
const edit = (marked: string, text: string) => inspectNoteAudition(type(editor(marked), text));

test('a typed octave resolves the local template instrument and literal controls', () => {
  expect(
    edit('$: s("sine*4").note("c|").penv(24).pdecay(0.05).decay(0.25).sustain(0).gain(0.9)', '2'),
  ).toMatchObject({
    kind: 'note',
    pitch: 36,
    sound: 'sine',
    controls: { decay: 0.25, sustain: 0, gain: 0.9 },
  });
  expect(
    edit(
      'song(meta, { intro: note("<[c3,eb|,g3] [ab2,c3,eb3]>").s("supersaw").attack(.1).release(.8).room(.5).gain(.25) })',
      '3',
    ),
  ).toMatchObject({
    pitch: 51,
    sound: 'supersaw',
    controls: { attack: 0.1, release: 0.8, gain: 0.25 },
  });
  expect(
    edit('stack(note("c3").s("square"), sound(\'saw\').note("f#|"), note("d3").s("pulse"))', '4'),
  ).toMatchObject({
    pitch: 66,
    sound: 'saw',
  });
});

test('uses the complete edited atom including unchanged suffixes, negative octaves and multiline chords', () => {
  expect(edit('note("g|").s("sine").gain(0.5).orbit(1).delay(0.2)', '3')).toMatchObject({
    pitch: 55,
    sound: 'sine',
    controls: { gain: 0.5 },
  });
  const mid = 'note("c3 |4 e4")';
  expect(edit(mid, 'G#')).toEqual({
    kind: 'note',
    from: mid.indexOf('|'),
    to: mid.indexOf('|') + 3,
    pitch: 68,
    sound: 'triangle',
    controls: {},
  });
  expect(edit('note(`\n [g-|,  e3]\n`)', '1')).toMatchObject({ pitch: 7 });
  const afterModifier = 's("[SINE]*4").note("c4!2   g|")';
  expect(edit(afterModifier, '3')).toMatchObject({
    from: afterModifier.indexOf('|') - 1,
    pitch: 55,
    sound: 'sine',
  });
  expect(edit('note("g|.5")', '3')).toBeNull();
  expect(edit('note("g|foo")', '3')).toBeNull();
  expect(edit('note("d|")', 'b')).toBeNull();
  expect(edit('note("12|")', '3')).toBeNull();
  expect(edit('note("g|")', '10')).toBeNull();
  expect(edit('note("c-|")', '2')).toBeNull();
});

test('handles unfinished JavaScript and an unclosed note string', () => {
  expect(edit('const unclosed = [\n s("pulse").note("b|', '3')).toMatchObject({
    pitch: 59,
    sound: 'pulse',
  });
  expect(edit('note("c|").s("sine");\nconst unfinished = (', '3')).toMatchObject({
    pitch: 48,
    sound: 'sine',
  });
});

test('resolves complete local voices in a long song without finishing the document parse', () => {
  const rest = '\n$: note("a4").s("pulse")'.repeat(2000);
  for (const [voice, sound] of [
    ['$: note("g|").s("sine")', 'sine'],
    ['$: note("g|")\n  .sound("saw")', 'saw'],
    ['$: note("g|")', 'triangle'],
  ]) {
    const tr = type(editor(voice + rest), '3');
    expect(inspectNoteAudition(tr)).toMatchObject({ pitch: 55, sound });
    expect(syntaxTreeAvailable(tr.state)).toBe(false);
  }
  const comments = '\n// more of the same voice follows'.repeat(1000);
  expect(edit('$: note("g|")' + comments + '\n.s("pulse")' + rest, '3')).toBeNull();
  expect(edit('$: note("g|");' + comments + rest, '3')).toMatchObject({ sound: 'triangle' });
});

test('accepts trailing commas on the sole argument of note and sound calls', () => {
  expect(edit('sound("triangle*4",).note("g|", /* end */)', '3')).toMatchObject({
    pitch: 55,
    sound: 'triangle',
  });
  expect(edit('note("g|",).s("pulse", /* end */)', '3')).toMatchObject({
    pitch: 55,
    sound: 'pulse',
  });
  expect(edit('note("g|",,).s("sine")', '3')).toBeNull();
  expect(edit('note("g|", "c4",).s("sine")', '3')).toBeNull();
});

test('operator arguments and non-note strings cannot become candidates', () => {
  for (const source of [
    'note("c4:g|")',
    'note("c4*g|")',
    'note("c4(g|,8)")',
    'note("{c4}%g|")',
    'note("c3").s("g|")',
    'n("g|")',
    "note('g|')",
    'note(random("g|"))',
    'note("g|" + variable)',
    'note("g|", 1)',
    'note(`g| ${other}`)',
    'note("\\x63 g|")',
    '// note("g|")',
    'const example = `note("g|")`',
  ])
    expect(edit(source, '3'), source).toBeNull();
});

test('honors nested mini directives, including comments inside the voice chain', () => {
  expect(edit('// mini-off\n/* mini-off */\n// mini-on\nnote("c|")', '3')).toBeNull();
  expect(
    edit('// mini-off\n/* mini-on */\nconst text = "// mini-off"; note("c|")', '3'),
  ).toMatchObject({ pitch: 48 });
  expect(edit('note("c|") /* mini-off */ .s("sine*4")', '3')).toBeNull();
});

test('ambiguous voices, transforms, overwritten pitches, and unsupported sounds stay silent', () => {
  for (const source of [
    's(choose("sine","square")).note("g|")',
    'note("g|").sound("<sine square>")',
    'note("g|").s("sine ! 4")',
    'note("g|").s("bd")',
    'note("g|").s(\'sine*4\')',
    'note("g|").s("square").s("sine")',
    'note("g|").freq(440)',
    'note("g|").add(12)',
    'note("g|").note("d3")',
    'note("a3").note("g|")',
    '(note("g|")).transpose(2)',
    'voice.note("g|")',
    'note("g|").apply(other)',
    'note("g|")["sound"]("sine")',
    'note("g|").s',
    'note("g|").gain(() => 1)',
    'stack(note("g|"), note("c3")).s("pulse")',
    'customVoice(note("g|"))',
  ])
    expect(edit(source, '3'), source).toBeNull();
});

test('keeps the edited stack voice through enclosing effects and applies literal outer controls', () => {
  const stack = 'stack(note("g|").s("sine").gain(.3), note("d4").s("square"))';
  expect(edit(stack + '.orbit(1)', '3')).toMatchObject({ sound: 'sine', controls: { gain: 0.3 } });
  expect(edit(stack + '.gain(.2).orbit(1)', '3')).toMatchObject({
    sound: 'sine',
    controls: { gain: 0.2 },
  });
  for (const suffix of ['.s("pulse")', '.note("d4")', '.add(12)', '.gain(dynamic)']) {
    expect(edit(stack + suffix, '3')).toBeNull();
  }
  expect(edit('note("c4").gain(note("g|").s("sine"))', '3')).toBeNull();
  expect(
    edit('note /* pitch */ ("g|"). /* selected sound */ s /* argument */ ("sine")', '3'),
  ).toMatchObject({ sound: 'sine' });
});

test('local definitions and function parameters can shadow bare Strudel constructors', () => {
  for (const source of [
    'const s = () => sound("square"); s("sine").note("g|")',
    'function note() { return other }; note("g|")',
    'function f({s}) { return s("sine").note("g|") }',
    'const f = sound => sound("sine").note("g|")',
    'const stack = custom; stack(note("g|"))',
  ])
    expect(edit(source, '3'), source).toBeNull();
  expect(edit('function other(s) { return s(1) }; s("sine").note("g|")', '3')).toMatchObject({
    sound: 'sine',
  });
});

test('accepts a single typed selection replacement and rejects all other change sources', () => {
  const state = editor('note("g|3")');
  const start = state.selection.main.head - 1;
  const selected = state.update({ selection: EditorSelection.range(start, start + 2) }).state;
  expect(inspectNoteAudition(type(selected, 'db4'))).toMatchObject({ pitch: 61 });
  for (const userEvent of [
    'input.paste',
    'input.complete',
    'input.drop',
    'undo',
    'redo',
    'input',
    'delete.selection',
  ]) {
    expect(inspectNoteAudition(type(selected, 'db4', userEvent)), userEvent).toBeNull();
  }
  expect(inspectNoteAudition(selected.update(selected.replaceSelection('db4')))).toBeNull();
  expect(inspectNoteAudition(type(selected, ''))).toBeNull();
  expect(
    inspectNoteAudition(selected.update({ selection: { anchor: start }, userEvent: 'select' })),
  ).toBeNull();
  const multi = state.update({
    selection: EditorSelection.create([EditorSelection.cursor(0), EditorSelection.cursor(start)]),
  }).state;
  expect(inspectNoteAudition(type(multi, 'a3'))).toBeNull();
});

test('confirms an old span on a delimiter, including CodeMirror overtyping its auto-inserted quote', () => {
  const initial = editor('note(|).s("sine")');
  const autoPair = insertBracket(initial, '"')!;
  expect(inspectNoteAudition(autoPair)).toBeNull();
  const note = type(type(autoPair.state, 'g').state, '3');
  const candidate = inspectNoteAudition(note)!;
  expect(candidate).toMatchObject({ kind: 'note', pitch: 55, sound: 'sine' });
  for (const tr of [type(note.state, ' '), insertBracket(note.state, '"')!]) {
    expect(inspectNoteAudition(tr)).toEqual({ ...candidate, kind: 'confirm' });
  }
  expect(edit('note("g|3")', ' ')).toBeNull();
  expect(edit('note("g|")', ' ')).toBeNull();
});
