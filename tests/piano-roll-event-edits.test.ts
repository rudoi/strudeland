import { expect, test } from 'bun:test';
import {
  Fraction,
  Hap,
  Pattern,
  State,
  TimeSpan,
  note,
  pure,
  register,
  sequence,
  stack,
} from '@strudel/core';
import { transpiler } from '@strudel/transpiler';
import {
  editPatternEvents,
  inspectEventEdits,
  identifyEvent,
  verifyEventOrigin,
  registerEventEdits,
  eventPitch,
  type EventEditEntry,
} from '../src/lib/strudel/event-edits';
import {
  eventMoveSource,
  planEventEdit,
  preparePhraseEvents,
} from '../src/lib/piano-roll/event-source';
import { songEditComment } from '../src/lib/strudel/event-document';
import { buildStructure } from '../src/lib/strudel/structure';
import { INITIAL_SEGMENTS, LINEAR_SEGMENTS } from '../src/lib/strudel/transport';
import { beginEvaluate, song, withSongPlacement } from '../src/lib/strudel/song';
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
      end: '3',
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
    planEventEdit(first, { kind: 'song' }, updated, {
      kind: 'move',
      id: 'a',
      start: '3/2',
      end: '5/2',
    }),
  );
  expect(songEditComment(next)?.spec.entries).toHaveLength(1);
  expect(songEditComment(next)?.spec.entries[0]).toMatchObject({ pitch: 63, key: 65, to: '3/2' });
  expect(songEditComment(next)?.spec.entries[0]).not.toHaveProperty('until');
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
  beginEvaluate();
  const structure = buildStructure(meta),
    p = song(meta, { intro: note(60), loop: note(64), tail: note(67) });
  const all = withSongPlacement(() => query(p, 0, 3, { _segments: LINEAR_SEGMENTS }));
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

test('a mixed-meter move saves both edges and replays its gate in partitioned queries', () => {
  const source = new TimeSpan('1/4', '1/2');
  const p = new Pattern((s: any) => {
    const part = source.intersection(s.span);
    return part ? [new Hap(source, part, { note: 64, duration: 0.25, clip: 0.5, pan: 0.75 })] : [];
  });
  const all = query(p, 0, 1);
  const n: RollNote = {
    id: 'n',
    start: '1/4',
    end: '1/2',
    pitch: 64,
    event: identifyEvent(all[0], all, false, false),
  };
  const saved = apply(
    'lead',
    planEventEdit('lead', { kind: 'song' }, n, {
      kind: 'move',
      id: n.id,
      start: '10/7',
      end: '12/7',
    }),
  );
  const spec = songEditComment(saved)!.spec;
  expect(spec.entries[0]).toMatchObject({
    from: ['1/4', '1/2'],
    pitch: 64,
    to: '10/7',
    until: '12/7',
  });
  const moved: RollNote = {
    ...n,
    start: '10/7',
    end: '12/7',
    event: { ...n.event!, entry: spec.entries[0].id },
  };
  expect(eventMoveSource(saved, { kind: 'song' }, moved)).toMatchObject({
    start: '1/4',
    end: '1/2',
  });
  const result = editPatternEvents(p, spec);
  expect(query(result, 0, 1)).toEqual([]);
  const [whole] = query(result, 1, 2);
  const parts = [...query(result, '10/7', '11/7'), ...query(result, '11/7', '12/7')];
  expect(summary(parts)).toEqual([
    ['10/7', '12/7', '10/7', '11/7', 64],
    ['10/7', '12/7', '11/7', '12/7', 64],
  ]);
  expect(parts.map((h) => h.hasOnset())).toEqual([true, false]);
  expect(whole.duration.toFraction()).toBe('1/7');
  expect(whole.value).toMatchObject({ pan: 0.75, clip: 0.5 });
  expect(parts.map((h) => h.value)).toEqual([whole.value, whole.value]);
});

test('moving a later repeat keeps both edges in the original entry phase', () => {
  registerEventEdits();
  const code =
    "(note(60))._pianoEdits({ entries: [{ id: 'r', from: ['0','1'], pitch: 60, period: 2, key: 65, to: '10/7', until: '12/7' }] })";
  const scope = { kind: 'expression', expression: { from: 0, to: code.length } } as const;
  const p = new Function('note', transpiler(code, {}).output)(note);
  const all = query(p, 5, 6);
  const later = all.find((h: any) => h.value.note === 65);
  const n: RollNote = {
    id: 'n',
    start: later.whole.begin.toFraction(),
    end: later.whole.end.toFraction(),
    pitch: 65,
    event: identifyEvent(later, all, true, false),
  };
  expect(n).toMatchObject({ start: '38/7', end: '40/7', event: { entry: 'r' } });
  expect(eventMoveSource(code, scope, n)).toMatchObject({ start: '4', end: '5', period: 2 });
  expect(
    eventMoveSource(code, scope, {
      ...n,
      event: { ...n.event!, entry: 'unrelated' },
    })?.period,
  ).toBeUndefined();
  const move = { kind: 'move', id: n.id, start: '11/2', end: '13/2' } as const;
  const saved = apply(code, planEventEdit(code, scope, n, move));
  expect(saved).toContain("to: '3/2', until: '5/2'");
  const updated = new Function('note', transpiler(saved, {}).output)(note);
  expect(
    query(updated, 0, 7)
      .filter((h: any) => h.value.note === 65 && h.hasOnset())
      .map((h: any) => [h.whole.begin.toFraction(), h.whole.end.toFraction()]),
  ).toEqual([
    ['3/2', '5/2'],
    ['7/2', '9/2'],
    ['11/2', '13/2'],
  ]);
  expect(planEventEdit(code, scope, { ...n, start: '39/7' }, move)).toBeNull();
  expect(planEventEdit(code, scope, n, { ...move, end: '300' })).toBeNull();
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

test('continuous cuts leave the same intervals in a full query and in sliced queries', () => {
  const value = { note: 60 };
  const p = new Pattern((state: any) => [new Hap(undefined, state.span, value)]);
  const cuts: [string, string][] = [
    ['3/4', '1'],
    ['1/4', '1/2'],
    ['5/16', '3/8'],
  ];
  const entries = cuts.map((from, i) => ({
    id: 'cut' + i,
    from,
    pitch: 60,
    continuous: true,
    capture: value,
    remove: true,
  }));
  const edited = editPatternEvents(p, { entries });
  const expected = [
    ['0', '1/4'],
    ['1/2', '3/4'],
  ];
  const spans = (haps: any[]) =>
    haps.map((h) => [h.part.begin.toFraction(), h.part.end.toFraction()]);
  expect(spans(query(edited, 0, 1))).toEqual(expected);
  const edges = ['0', '1/4', '1/2', '3/4', '1'];
  expect(edges.slice(1).flatMap((end, i) => spans(query(edited, edges[i], end)))).toEqual(expected);
  expect(query(edited, '1/4', '1/4')).toEqual([]);
  expect(query(edited, '1/2', '1/2')).toHaveLength(1);
});

test('final song edits distinguish an independent voice from an identical section voice on each loop visit', () => {
  beginEvaluate();
  const m = { sections: [{ name: 'loop', bars: 1, loop: true }] };
  const p = stack(song(m, { loop: note(70) }), note(70));
  const haps = withSongPlacement(() => query(p, 0, 1, { _segments: LINEAR_SEGMENTS }));
  const structure = buildStructure(m),
    layout = { structure, segments: () => INITIAL_SEGMENTS, cps: () => 0.5 };
  for (const [index, repeat] of [
    [0, true],
    [1, false],
  ] as const) {
    const identity = identifyEvent(haps[index], haps, false, false)!;
    expect(identity.placement).toBe(repeat ? 'song' : 'absolute');
    const edited = editPatternEvents(
      p,
      { entries: [{ ...identity, id: 'roll1', key: 72 }] },
      layout,
    );
    expect(
      query(edited, 0, 1)
        .map((h: any) => h.value.note)
        .sort(),
    ).toEqual([70, 72]);
    expect(
      query(edited, 1, 2)
        .map((h: any) => h.value.note)
        .sort(),
    ).toEqual(repeat ? [70, 72] : [70, 70]);
  }
});

test('planner checks the limit across existing and new entries before writing source', () => {
  const entries = Array.from({ length: 2048 }, (_, i) => ({
    id: 'e' + i,
    from: ['0', '1'],
    pitch: 60,
    key: 63,
  }));
  const code = 'note(60)\n/* @piano-edits ' + JSON.stringify({ entries }) + ' */';
  const note = preparePhraseEvents([{ id: 'n', start: '0', end: '1', pitch: 60 }])[0];
  expect(
    planEventEdit(code, { kind: 'song' }, note, { kind: 'pitch', id: 'n', pitch: 61 }),
  ).not.toBeNull();
  expect(
    planEventEdit(
      code,
      { kind: 'song' },
      { ...note, event: { from: ['1', '2'], pitch: 60 } },
      { kind: 'pitch', id: 'n', pitch: 61 },
    ),
  ).toBeNull();
});

test('primitive malformed pitches cannot cross the host boundary as non-finite MIDI numbers', () => {
  expect(eventPitch('c-', true)).toBeUndefined();
  expect(eventPitch(Infinity, true)).toBeUndefined();
  expect(eventPitch({ freq: [], note: 60, octave: [] }, false)).toBe(60);
  expect(eventPitch({ freq: Symbol('freq'), note: 65 }, false)).toBe(65);
});

test('masked note edits retain disjoint parts across whole and split requests and distinguish stacked copies', () => {
  const masked = note(60).mask(sequence(1, 0, 1, 0));
  for (const p of [masked, stack(masked, masked)]) {
    const fragments = query(p, 0, 1);
    const target = fragments[fragments.length - 1];
    const observed = identifyEvent(target, fragments, true, false)!;
    const verified = verifyEventOrigin(target, observed, query(p, 0, 0))!;
    expect(verified).toBeDefined();
    expect(verified.count ?? 1).toBe(fragments.length / 2);
    const retuned = editPatternEvents(p, { entries: [{ ...verified, id: 'r', key: 65 }] });
    const moved = editPatternEvents(p, { entries: [{ ...verified, id: 'r', key: 65, to: '5/4' }] });
    const targetParts = (haps: any[]) =>
      haps
        .filter((h) => h.value.note === 65)
        .map((h) => [
          h.whole.begin.toFraction(),
          h.part.begin.toFraction(),
          h.part.end.toFraction(),
          h.hasOnset(),
        ]);
    const expected = [
      ['0', '0', '1/4', true],
      ['0', '1/2', '3/4', false],
    ];
    expect(targetParts(query(retuned, 0, 1))).toEqual(expected);
    expect(
      targetParts([
        ...query(retuned, 0, '1/4'),
        ...query(retuned, '1/4', '1/2'),
        ...query(retuned, '1/2', 1),
      ]),
    ).toEqual(expected);
    expect(query(retuned, 0, 1).filter((h: any) => h.value.note === 60).length).toBe(
      fragments.length - 2,
    );
    expect(targetParts(query(moved, 1, 3))).toEqual([
      ['5/4', '5/4', '3/2', true],
      ['5/4', '7/4', '2', false],
    ]);
    const scaled = editPatternEvents(p, {
      entries: [{ ...verified, id: 'r', key: 65, to: '10/7', until: '12/7' }],
    });
    const scaledParts = [
      ['10/7', '10/7', '3/2', true],
      ['10/7', '11/7', '23/14', false],
    ];
    expect(targetParts(query(scaled, 1, 2))).toEqual(scaledParts);
    expect(targetParts([...query(scaled, 1, '11/7'), ...query(scaled, '11/7', 2)])).toEqual(
      scaledParts,
    );
    expect(query(moved, 0, 1).filter((h: any) => h.value.note === 60).length).toBe(
      fragments.length - 2,
    );
  }
});

test('a final time transform edits its absolute occurrence without adding onsets on loop wrap', () => {
  beginEvaluate();
  const m = { sections: [{ name: 'loop', bars: 1, loop: true }] };
  const p = song(m, { loop: note(60) }).slow(2);
  const linear = withSongPlacement(() => query(p, 0, 2, { _segments: LINEAR_SEGMENTS }));
  const identity = identifyEvent(linear[0], linear, false, false)!;
  expect(identity.placement).toBe('absolute');
  const edited = editPatternEvents(
    p,
    { entries: [{ ...identity, id: 'r', key: 65 }] },
    { structure: buildStructure(m), segments: () => INITIAL_SEGMENTS, cps: () => 0.5 },
  );
  const onsets = (hs: any[]) =>
    hs.filter((h) => h.hasOnset()).map((h) => [Number(h.whole.begin), h.value.note]);
  expect(onsets(query(edited, 0, 4))).toEqual([
    [0, 65],
    [2, 60],
  ]);
  expect(onsets([0, 1, 2, 3].flatMap((i) => query(edited, i, i + 1)))).toEqual([
    [0, 65],
    [2, 60],
  ]);
});

test('song registration cannot silently replace the operation used by saved edits', () => {
  registerEventEdits();
  expect(() => register('pianoEdits', (_spec: unknown, p: any) => p)).toThrow();
  const p = note(60)._pianoEdits({
    entries: [{ id: 'first', from: ['0', '1'], pitch: 60, key: 62 }],
  });
  expect(query(p, 0, 1)[0].value.note).toBe(62);
});

test('retuning a note to base MIDI zero uses a sounding pitch with and without an octave offset', () => {
  for (const octave of [0, 1]) {
    const p = pure({ note: 60, octave });
    const all = query(p, 0, 1);
    const patched = event(p, all[0], all, { key: 12 * octave });
    const value = query(patched, 0, 1)[0].value;
    expect(value.note).toBe('c-1');
    expect(eventPitch(value, false)).toBe(12 * octave);
  }
});

test('string-valued pitch transforms retain string inputs and the written spelling convention', () => {
  const p = pure('db4');
  const haps = query(p, 0, 1);
  const flat = event(p, haps[0], haps, { key: 63 });
  expect(query(flat, 0, 1)[0].value).toBe('eb4');
  const line = event(p, haps[0], haps, { key: 64 });
  expect(query(line.fmap((x: string) => x.toUpperCase()).note(), 0, 1)[0].value.note).toBe('E4');
  const tagged = note('C4');
  const observed = query(tagged, 0, 1);
  expect(query(event(tagged, observed[0], observed, { key: 66 }), 0, 1)[0].value.note).toBe('F#4');
});

test('an explicitly captured retune replays its missing onset after a seed change', () => {
  const captured = { note: 60, s: 'sine', gain: 0.4 };
  const p = new Pattern((s: any) => (s.controls.randSeed === 3 ? pure(captured).query(s) : []));
  const first = query(p, 0, 1, { randSeed: 3 }),
    original = identifyEvent(first[0], first, true, true)!;
  const edited = editPatternEvents(p, { entries: [{ ...original, id: 'captured', key: 64 }] });
  expect(query(edited, 0, 1, { randSeed: 3 })).toHaveLength(1);
  const full = inspectEventEdits(() => query(edited, 0, 1, { randSeed: 4 }));
  expect(full.staleEdits).toEqual([]);
  expect(full.value[0].value).toEqual({ ...captured, note: 64 });
  expect(query(edited, '1/2', 1, { randSeed: 4 })[0].hasOnset()).toBe(false);
  const { capture: _, ...selector } = original;
  const live = editPatternEvents(p, { entries: [{ ...selector, id: 'live', key: 64 }] });
  const stale = inspectEventEdits(() => query(live, 0, 1, { randSeed: 4 }));
  expect(stale).toEqual({ value: [], staleEdits: ['live'] });
});

test('collect stale entries only for visible queries and isolate nested inspections', () => {
  const inner = editPatternEvents(note(60), {
    entries: [{ id: 'old', from: ['0', '1'], pitch: 67, key: 65 }],
  });
  const moved = editPatternEvents(inner, {
    entries: [{ id: 'move', from: ['0', '1'], pitch: 60, to: '2' }],
  });
  const visible = inspectEventEdits(() => query(moved, 2, 3));
  expect(visible.staleEdits).toEqual([]);
  const original = inspectEventEdits(() => {
    query(inner, 0, 1);
    query(inner, 0, 1);
    return inspectEventEdits(() => query(inner, 2, 3)).staleEdits;
  });
  expect(original).toEqual({ value: [], staleEdits: ['old'] });
  expect(query(inner, 0, 1)[0].value.note).toBe(60);
});
