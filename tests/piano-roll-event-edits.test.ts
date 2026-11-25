import { expect, test } from 'bun:test';
import { Fraction, Hap, Pattern, State, TimeSpan, note, pure, stack } from '@strudel/core';
import { transpiler } from '@strudel/transpiler';
import {
  editPatternEvents,
  identifyEvent,
  registerEventEdits,
  eventPitch,
  type EventEditEntry,
} from '../src/lib/strudel/event-edits';
import { planEventEdit, preparePhraseEvents } from '../src/lib/piano-roll/event-source';
import { songEditComment } from '../src/lib/strudel/event-document';
import { buildStructure } from '../src/lib/strudel/structure';
import { INITIAL_SEGMENTS, LINEAR_SEGMENTS } from '../src/lib/strudel/transport';
import { song } from '../src/lib/strudel/song';
import { formatSong } from '../src/lib/editor/format-song';
import type { RollNote, RollPatch } from '../src/lib/piano-roll/types';

function query(p: any, from: string | number, to: string | number, controls: object = {}) {
  return p.query(new State(new TimeSpan(from, to), controls));
}
const apply = (s: string, edits: readonly RollPatch[] | null) => {
  expect(edits).not.toBeNull();
  for (const e of [...edits!].reverse()) s = s.slice(0, e.from) + e.insert + s.slice(e.to);
  return s;
};
const summary = (haps: any[]) =>
  haps.map((h) => [
    h.whole?.begin.toFraction(),
    h.whole?.end.toFraction(),
    h.part.begin.toFraction(),
    h.part.end.toFraction(),
    eventPitch(h.value, true),
  ]);
function event(p: any, h: any, group: any[], patch: Partial<EventEditEntry>, stochastic = false) {
  const selector = identifyEvent(h, group, true, stochastic);
  expect(selector).toBeDefined();
  return editPatternEvents(p, { entries: [{ ...selector!, id: 'roll1', ...patch }] });
}

test('move an overlapping duplicate, preserving unrelated controls, painter count, callbacks and partial windows', () => {
  const callback = () => {};
  const source = new TimeSpan('3/4', '5/4');
  const voice = (gain: number) =>
    new Pattern((s: any) => {
      s.controls.painters?.push('voice');
      const part = source.intersection(s.span);
      return part
        ? [
            new Hap(
              source,
              part,
              { note: 60, s: 'sine', gain, duration: 1 / 2, clip: 1 / 2 },
              { onSchedule: callback },
            ),
          ]
        : [];
    });
  const p = stack(voice(1), voice(0.5), voice(0.5)),
    all = query(p, 0, 2);
  const edited = event(p, all[2], all, { to: '5/4', key: 62 });
  const painters: unknown[] = [];
  expect(summary(query(edited, 0, 1, { painters }))).toEqual([
    ['3/4', '5/4', '3/4', '1', 60],
    ['3/4', '5/4', '3/4', '1', 60],
  ]);
  expect(painters.length).toBe(3);
  expect(query(edited, 1, '5/4')).toHaveLength(2);
  const [first] = query(edited, '5/4', '3/2'),
    [last] = query(edited, '3/2', '7/4');
  expect(first.hasOnset()).toBe(true);
  expect(last.hasOnset()).toBe(false);
  expect(first.value).toEqual({ note: 62, s: 'sine', gain: 0.5, duration: 0.5, clip: 0.5 });
  expect(Number(first.duration)).toBe(0.25);
  expect(first.context.onSchedule).toBe(callback);
  expect(query(edited, '7/4', 2)).toEqual([]);
});

test('capture one stochastic lane and preserve frequency and octave through pitch and time edits', () => {
  const value = { freq: 440, octave: -1, s: 'sine', gain: 0.25 };
  let actual = value;
  const p = new Pattern((s: any) => pure(actual).query(s));
  const all = query(p, 0, 1);
  const edited = event(p, all[0], all, { to: '1/2', key: 69 }, true);
  actual = { ...value, freq: 880 };
  const result = query(edited, '1/2', '3/4');
  expect(result).toHaveLength(1);
  expect(result[0].value).toMatchObject({ octave: -1, s: 'sine', gain: 0.25 });
  expect(result[0].value.freq).toBeCloseTo(880);
  expect(eventPitch(result[0].value, true)).toBeCloseTo(69);
});

test('a captured continuous sample becomes one finite note across partitioned queries', () => {
  const p = new Pattern((s: any) => [
    new Hap(undefined, s.span, { note: 60 + Number(s.span.midpoint()) }),
  ]);
  const all = query(p, 0, 1);
  const edited = event(p, all[0], all, { to: '2', key: 66 });
  expect(query(edited, '1/4', '1/2')).toEqual([]);
  const values = query(edited, '2', '5/2');
  expect(values.some((h: any) => h.whole && h.value.note === 66 && h.hasOnset())).toBe(true);
  expect(
    query(edited, '5/2', 3)
      .filter((h: any) => h.whole)
      .every((h: any) => !h.hasOnset()),
  ).toBe(true);
});

test('Expression wrappers are parseable, retain shorthand semantics and update one entry on a second gesture', () => {
  registerEventEdits();
  const input = 'const intro = note(60); ({intro})';
  const from = input.lastIndexOf('intro'),
    to = from + 5;
  const [n] = preparePhraseEvents([{ id: '1', start: '0', end: '1', pitch: 60 }]);
  const first = apply(
    input,
    planEventEdit(input, { kind: 'expression', expression: { from, to } }, n, {
      kind: 'pitch',
      id: n.id,
      pitch: 63,
    }),
  );
  expect(first).toContain('intro: (intro)._pianoEdits');
  const a = first.indexOf('(intro)'),
    b = first.lastIndexOf('})') + 1;
  const next = apply(
    first,
    planEventEdit(first, { kind: 'expression', expression: { from: a, to: b - 1 } }, n, {
      kind: 'move',
      id: n.id,
      start: '2',
    }),
  );
  expect(next.match(/id: 'roll1'/g)?.length).toBe(1);
  const p = new Function('note', transpiler(next, {}).output)(note).intro;
  expect(query(p, 0, 1)).toEqual([]);
  expect(query(p, 2, 3).some((h: any) => h.value.note === 63)).toBe(true);
});

test('a saved final-output edit survives formatting and updates the same entry from inspected identity', () => {
  const code = 'lead: note(60)\nlead2: note(63)';
  const p = stack(note(60), note(63));
  const all = query(p, 0, 1);
  const n: RollNote = {
    id: 'a',
    start: '0',
    end: '1',
    pitch: 63,
    event: identifyEvent(all[1], all, false, false),
  };
  const first = apply(
    code,
    planEventEdit(code, { kind: 'song' }, n, { kind: 'pitch', id: 'a', pitch: 65 }),
  );
  const formatted = apply(first, formatSong(first));
  const parsed = songEditComment(formatted)!;
  const edited = editPatternEvents(p, parsed.spec);
  const changed = query(edited, 0, 1);
  expect(changed.map((h: any) => h.value.note)).toEqual([60, 65]);
  const updated = { ...n, pitch: 65, event: identifyEvent(changed[1], changed, false, false) };
  const next = apply(
    first,
    planEventEdit(first, { kind: 'song' }, updated, { kind: 'move', id: 'a', start: '3/2' }),
  );
  expect(songEditComment(next)?.spec.entries).toHaveLength(1);
  expect(songEditComment(next)?.spec.entries[0]).toMatchObject({ pitch: 63, key: 65, to: '3/2' });
  expect(transpiler(next, {}).output).not.toContain('@piano-edits');
});

test('written song edits move a note across sections on every loop visit and clip layout', () => {
  const meta = {
    bpm: 120,
    meter: '4/4',
    sections: [
      { name: 'intro', bars: 1 },
      { name: 'loop', bars: 1, loop: true },
      { name: 'tail', bars: 1 },
    ],
  };
  const structure = buildStructure(meta),
    p = song(meta, { intro: note(60), loop: note(64), tail: note(67) });
  const all = query(p, 0, 3, { _segments: LINEAR_SEGMENTS });
  const entry = { ...identifyEvent(all[0], all, false, false)!, id: 'roll1', to: '3/2', key: 72 };
  const edited = editPatternEvents(
    p,
    { entries: [entry] },
    { structure, segments: () => INITIAL_SEGMENTS, cps: () => 0.5 },
  );
  expect(query(edited, 0, 1)).toEqual([]);
  expect(
    query(edited, '3/2', 2)
      .filter((h: any) => h.value.note === 72)
      .map((h: any) => h.whole.begin.toFraction()),
  ).toEqual(['3/2']);
  expect(
    query(edited, '5/2', 3)
      .filter((h: any) => h.value.note === 72)
      .map((h: any) => h.whole.begin.toFraction()),
  ).toEqual(['5/2']);
  expect(
    query(edited, 2, 3, { _segments: LINEAR_SEGMENTS })
      .filter((h: any) => h.value.note === 72)
      .every((h: any) => !h.hasOnset()),
  ).toBe(true);
});

test('known local phrase periods repeat source-backed edits across the period boundary', () => {
  const n = preparePhraseEvents([{ id: '1', start: '0', end: '1', pitch: 60 }], 1)[0];
  const p = editPatternEvents(note(60), {
    entries: [{ ...n.event!, id: 'roll1', key: 61, to: '3/2' }],
  });
  for (const from of [0, 1, 5]) {
    const result = query(p, from, from + 1);
    expect(result.map((h: any) => h.value.note)).toEqual([61, 61]);
    expect(result.filter((h: any) => h.hasOnset()).map((h: any) => Number(h.whole.begin))).toEqual([
      from + 0.5,
    ]);
  }
});

test('a first edit preserves an implicit statement boundary before an unlabelled pattern', () => {
  registerEventEdits();
  const code = 'const lead = note(60)\nlead';
  const from = code.lastIndexOf('lead'),
    n = preparePhraseEvents([{ id: '0', start: '0', end: '1', pitch: 60 }])[0];
  const saved = apply(
    code,
    planEventEdit(code, { kind: 'expression', expression: { from, to: code.length } }, n, {
      kind: 'pitch',
      id: '0',
      pitch: 64,
    }),
  );
  const p = new Function('note', transpiler(saved, {}).output)(note);
  expect(query(p, 0, 1)[0].value.note).toBe(64);
});
