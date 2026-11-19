import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import { transpiler } from '@strudel/transpiler';
import {
  beginEvaluate,
  registeredStructure,
  setSegmentSource,
  song,
} from '../src/lib/strudel/song';
import { buildStructure, type SongStructure } from '../src/lib/strudel/structure';
import { INITIAL_SEGMENTS, withCountIn, type Segment } from '../src/lib/strudel/transport';
import { fakeRuntime } from './helpers/fake-runtime';
import { loopView, RELEASE, seg, SONG } from './helpers/song-fixtures';

await core.evalScope(core, mini, { song });
const { Pattern, reify, silence } = core;
/** What the transpiler makes of a double-quoted string. */
const m = mini.mini;

// intro 2 bars (cycles 0-1), body 4 looping (2-5), tail 2 (6-7).
const META = {
  title: 'x',
  bpm: 120,
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
};
const PARTS = { intro: m('<i0 i1 i2>'), body: m('<b0 b1 b2 b3 b4>'), tail: m('<t0 t1 t2>') };

type Onset = [begin: number, end: number, value: unknown];

/** Onset haps of `pat` over [b, e), as [whole.begin, whole.end, value], sorted. */
function onsets(pat: any, b: number, e: number): Onset[] {
  return toOnsets(pat.queryArc(b, e));
}
function toOnsets(haps: any[]): Onset[] {
  return haps
    .filter((h) => h.hasOnset())
    .map((h): Onset => [h.whole.begin.valueOf(), h.whole.end.valueOf(), h.value])
    .sort((x, y) => x[0] - y[0] || String(x[2]).localeCompare(String(y[2])));
}
const values = (pat: any, b: number, e: number) => onsets(pat, b, e).map((o) => o[2]);

beforeEach(() => beginEvaluate());
afterEach(() => setSegmentSource(() => INITIAL_SEGMENTS));

describe('song view', () => {
  test('intro once, then the loop restarts at local cycle 0 on every pass', () => {
    const pat = song(META, PARTS);
    expect(values(pat, 0, 14)).toEqual([
      'i0',
      'i1',
      'b0',
      'b1',
      'b2',
      'b3',
      'b0',
      'b1',
      'b2',
      'b3',
      'b0',
      'b1',
      'b2',
      'b3',
    ]);
    expect(onsets(pat, 0, 3).map((o) => o[0])).toEqual([0, 1, 2]);
  });

  test('each section starts at its own cycle 0', () => {
    const same = m('<a b c d e f>');
    const pat = song(META, { intro: same, body: same, tail: same });
    expect(values(pat, 0, 8)).toEqual(['a', 'b', 'a', 'b', 'c', 'd', 'a', 'b']);
  });

  test('mini-notation parts', () => {
    const pat = song(META, { intro: m('a b'), body: m('c [d e]'), tail: m('f') });
    expect(onsets(pat, 0, 1)).toEqual([
      [0, 0.5, 'a'],
      [0.5, 1, 'b'],
    ]);
    expect(onsets(pat, 2, 3)).toEqual([
      [2, 2.5, 'c'],
      [2.5, 2.75, 'd'],
      [2.75, 3, 'e'],
    ]);
  });

  test('plain values go through reify like any Strudel argument', () => {
    const meta = {
      sections: [
        { name: 'a', bars: 1 },
        { name: 'b', bars: 1 },
        { name: 'c', bars: 1 },
      ],
    };
    const pat = song(meta, { a: 'plain', b: 3, c: { s: 'bd' } });
    expect(onsets(pat, 0, 3)).toEqual([
      [0, 1, 'plain'],
      [1, 2, 3],
      [2, 3, { s: 'bd' }],
    ]);
  });

  test('in evaluated code, double-quoted strings are mini-notation', async () => {
    const code = `const meta = { sections: [{ name: 'a', bars: 1 }, { name: 'b', bars: 2, loop: true }] };
song(meta, { a: "x y", b: "<p q>" })`;
    const { pattern } = await core.evaluate(code, transpiler);
    expect(pattern).toBeInstanceOf(Pattern);
    expect(values(pattern, 0, 5)).toEqual(['x', 'y', 'p', 'q', 'p', 'q']);
    expect(registeredStructure()?.totalBars).toBe(3);
  });

  test('pattern parts and silence', () => {
    const pat = song(META, { intro: silence, body: reify('x').fast(2), tail: silence });
    expect(onsets(pat, 0, 2)).toEqual([]);
    expect(onsets(pat, 2, 3)).toEqual([
      [2, 2.5, 'x'],
      [2.5, 3, 'x'],
    ]);
  });

  test('no loop section plays through once, then silence', () => {
    const meta = {
      sections: [
        { name: 'a', bars: 1 },
        { name: 'b', bars: 2 },
      ],
    };
    const pat = song(meta, { a: m('<a0 a1>'), b: m('<b0 b1 b2>') });
    expect(values(pat, 0, 10)).toEqual(['a0', 'b0', 'b1']);
    expect(pat.queryArc(3, 100)).toEqual([]);
  });

  test('negative cycles are silent', () => {
    const pat = song(META, PARTS);
    expect(pat.queryArc(-2, 0)).toEqual([]);
    expect(pat.queryArc(-2, -1)).toEqual([]);
    expect(toOnsets(pat.queryArc(-1, 1))).toEqual(onsets(pat, 0, 1));
  });

  test('a zero-width query answers from the section at that point', () => {
    const pat = song(META, PARTS);
    const haps = pat.queryArc(2.5, 2.5);
    expect(haps.map((h: any) => h.value)).toEqual(['b0']);
    expect(haps[0].part.begin.valueOf()).toBe(2.5);
    expect(haps[0].whole.begin.valueOf()).toBe(2);
  });

  test('returns a Pattern of the one @strudel/core copy', () => {
    const pat = song(META, PARTS);
    expect(pat).toBeInstanceOf(Pattern);
    expect(pat.fast(2)).toBeInstanceOf(Pattern);
    expect(values(pat.fast(2), 0, 2)).toEqual(['i0', 'i1', 'b0', 'b1']);
  });

  test('haps keep their values and context', () => {
    const body = core.s(m('bd sd')).gain(0.5);
    const pat = song(META, { intro: silence, body, tail: silence });
    const direct = body.queryArc(0, 1);
    const mapped = pat.queryArc(2, 3);
    expect(mapped.map((h: any) => h.value)).toEqual([
      { s: 'bd', gain: 0.5 },
      { s: 'sd', gain: 0.5 },
    ]);
    expect(mapped.map((h: any) => h.context)).toEqual(direct.map((h: any) => h.context));
  });
});

describe('clipping', () => {
  test('a whole crossing a section boundary is clipped to the occurrence, onset kept', () => {
    const pat = song(META, { intro: reify('long').slow(3), body: 'b', tail: 't' });
    expect(onsets(pat, 0, 3)).toEqual([
      [0, 2, 'long'],
      [2, 3, 'b'],
    ]);
    // The part of the long note after the boundary never plays.
    expect(pat.queryArc(2, 2.5).map((h: any) => h.value)).toEqual(['b']);
  });

  test('a hap straddling two queries keeps one onset', () => {
    const pat = song(META, { intro: reify('long').slow(2), body: 'b', tail: 't' });
    const first = pat.queryArc(0, 0.7);
    const second = pat.queryArc(0.7, 1.4);
    expect(first.filter((h: any) => h.hasOnset())).toHaveLength(1);
    expect(second.filter((h: any) => h.hasOnset())).toHaveLength(0);
    expect(second[0].whole.begin.valueOf()).toBe(0);
    expect(second[0].whole.end.valueOf()).toBe(2);
  });

  test('many small queries give the same onsets as one big query', () => {
    const parts = {
      intro: reify('x').slow(3),
      body: m('<[a b c]@2 d> [e f]/3'),
      tail: m('g . h i'),
    };
    const segments = [seg(0, 0, SONG), seg(9, 0, loopView(0)), seg(13, 6, SONG)];
    setSegmentSource(() => segments);
    const pat = song(META, parts);
    const big = onsets(pat, 0, 16);
    expect(big.length).toBeGreaterThan(20);
    const small: any[] = [];
    for (let i = 0; i < 64; i++) small.push(...pat.queryArc(i / 4, (i + 1) / 4));
    expect(toOnsets(small)).toEqual(big);
    // Uneven windows (not aligned to bars) too.
    const uneven: any[] = [];
    for (let t = 0; t < 16; t += 0.3) uneven.push(...pat.queryArc(t, Math.min(16, t + 0.3)));
    expect(toOnsets(uneven)).toEqual(big);
  });
});

describe('segment source', () => {
  test('a loop view loops its section', () => {
    setSegmentSource(() => [seg(0, 0, SONG), seg(3, 0, loopView(0))]);
    const pat = song(META, PARTS);
    expect(values(pat, 0, 7)).toEqual(['i0', 'i1', 'b0', 'i0', 'i1', 'i0', 'i1']);
  });

  test('a release plays the tail once and ends', () => {
    setSegmentSource(() => [seg(0, 2, loopView(1)), seg(4, 6, RELEASE)]);
    const pat = song(META, PARTS);
    expect(values(pat, 0, 12)).toEqual(['b0', 'b1', 'b2', 'b3', 't0', 't1']);
  });

  test('start from a section', () => {
    setSegmentSource(() => [seg(0, 6, SONG)]);
    expect(values(song(META, PARTS), 0, 5)).toEqual(['t0', 't1']);
  });

  test('the source is read at every query', () => {
    let segments: readonly Segment[] = INITIAL_SEGMENTS;
    setSegmentSource(() => segments);
    const pat = song(META, PARTS);
    expect(values(pat, 0, 1)).toEqual(['i0']);
    segments = [seg(0, 6, SONG)];
    expect(values(pat, 0, 1)).toEqual(['t0']);
  });

  test('a change mid-occurrence that keeps the occurrence does not retrigger', () => {
    // From cycle 4 the body loops; at 4 it is at local cycle 2 of the occurrence that began at 2.
    setSegmentSource(() => [seg(0, 0, SONG), seg(4, 4, loopView(1))]);
    const pat = song(META, { intro: 'i', body: reify('long').slow(4), tail: 't' });
    const haps = pat.queryArc(4, 5);
    expect(haps).toHaveLength(1);
    expect(haps[0].hasOnset()).toBe(false);
    expect([haps[0].whole.begin.valueOf(), haps[0].whole.end.valueOf()]).toEqual([2, 6]);
  });

  test('a restart mid-occurrence starts a new one', () => {
    setSegmentSource(() => [seg(0, 0, SONG), seg(4, 2, loopView(1))]);
    const pat = song(META, PARTS);
    expect(onsets(pat, 2, 7).map((o) => [o[0], o[2]])).toEqual([
      [2, 'b0'],
      [3, 'b1'],
      [4, 'b0'],
      [5, 'b1'],
      [6, 'b2'],
    ]);
  });

  test('segments off whole cycles are rejected instead of looping forever', () => {
    setSegmentSource(() => [seg(0, 0, SONG), seg(3.5, 0, SONG)]);
    const pat = song(META, PARTS);
    expect(() => pat.query(new core.State(new core.TimeSpan(3.7, 4)))).toThrow(
      'song: transport segments must start at whole cycles',
    );
  });
});

describe('count-in', () => {
  test('silent in [0, 1), then bar 0 at cycle 1', () => {
    setSegmentSource(() => withCountIn(INITIAL_SEGMENTS));
    const pat = song(META, PARTS);
    expect(pat.queryArc(0, 1)).toEqual([]);
    expect(pat.queryArc(0.25, 0.75)).toEqual([]);
    expect(onsets(pat, 0, 2)).toEqual([[1, 2, 'i0']]);
    expect(onsets(pat, 0, 4)).toEqual([
      [1, 2, 'i0'],
      [2, 3, 'i1'],
      [3, 4, 'b0'],
    ]);
  });

  test('the haps after the count-in are those of the song shifted by one cycle', () => {
    const parts = { intro: m('a [b c]'), body: m('<d e> f'), tail: m('g') };
    setSegmentSource(() => INITIAL_SEGMENTS);
    const plain = onsets(song(META, parts), 0, 9).map(([b, e, v]) => [b + 1, e + 1, v]);
    beginEvaluate();
    setSegmentSource(() => withCountIn(INITIAL_SEGMENTS));
    expect(onsets(song(META, parts), 0, 10)).toEqual(plain as Onset[]);
  });

  test('an armed section after the count-in starts at cycle 1', () => {
    setSegmentSource(() => withCountIn([seg(0, 6, SONG)]));
    expect(onsets(song(META, PARTS), 0, 5)).toEqual([
      [1, 2, 't0'],
      [2, 3, 't1'],
    ]);
  });

  test('a long note in bar 0 is not heard during the count-in', () => {
    setSegmentSource(() => withCountIn(INITIAL_SEGMENTS));
    const pat = song(META, { intro: reify('long').slow(2), body: 'b', tail: 't' });
    expect(pat.queryArc(0, 1)).toEqual([]);
    const haps = pat.queryArc(1, 2);
    expect(haps).toHaveLength(1);
    expect([haps[0].whole.begin.valueOf(), haps[0].whole.end.valueOf()]).toEqual([1, 3]);
  });
});

describe('errors', () => {
  const thrown = (f: () => unknown): string => {
    try {
      f();
    } catch (e) {
      return (e as Error).message;
    }
    throw new Error('did not throw');
  };

  test('meta without sections', () => {
    expect(thrown(() => song({ bpm: 120 }, {}))).toBe(
      "song: meta.sections is required, e.g. [{ name: 'intro', bars: 4 }]",
    );
  });

  test('invalid meta', () => {
    expect(thrown(() => song({ sections: [] }, {}))).toBe(
      'song: meta.sections must be a non-empty array of sections',
    );
    expect(thrown(() => song({ ...META, bpm: 1000 }, PARTS))).toBe(
      'song: meta.bpm must be a number from 20 to 400',
    );
    expect(thrown(() => song({ ...META, loops: 0 }, PARTS))).toBe(
      'song: meta.loops must be an integer from 1 to 1000',
    );
    expect(thrown(() => song(undefined as any, PARTS))).toBe(
      'song: the first argument must be the meta object',
    );
    expect(thrown(() => song([] as any, PARTS))).toBe(
      'song: the first argument must be the meta object',
    );
  });

  test('missing part', () => {
    expect(thrown(() => song(META, { intro: 'a', body: 'b' }))).toBe(
      "song: no part for section 'tail'",
    );
    expect(thrown(() => song(META, { intro: 'a', body: 'b', tail: undefined }))).toBe(
      "song: no part for section 'tail'",
    );
  });

  test('extra part', () => {
    expect(thrown(() => song(META, { ...PARTS, outro: 'x' }))).toBe(
      "song: part 'outro' is not a section in meta.sections",
    );
  });

  test('parts must be an object', () => {
    for (const bad of [undefined, null, 'a', ['a']]) {
      expect(thrown(() => song(META, bad as any))).toBe(
        'song: the second argument must be an object of parts, e.g. { intro: pat }',
      );
    }
  });

  test('a failed call registers nothing', () => {
    expect(() => song(META, {})).toThrow();
    expect(registeredStructure()).toBeNull();
  });
});

describe('registry', () => {
  test('song() registers its structure; beginEvaluate clears it', () => {
    expect(registeredStructure()).toBeNull();
    song(META, PARTS);
    expect(registeredStructure()).toEqual(buildStructure(META));
    beginEvaluate();
    expect(registeredStructure()).toBeNull();
  });

  test('the same structure twice in one evaluate is fine', () => {
    song(META, PARTS);
    song({ ...META, title: 'other title' }, { intro: 'a', body: 'b', tail: 'c' });
    expect(registeredStructure()).toEqual(buildStructure(META));
  });

  test('two different structures in one evaluate throw', () => {
    song(META, PARTS);
    expect(() => song({ ...META, bpm: 90 }, PARTS)).toThrow('song: one structure per song');
    expect(registeredStructure()).toEqual(buildStructure(META));
  });

  test('a different structure is fine after beginEvaluate', () => {
    song(META, PARTS);
    beginEvaluate();
    song({ ...META, bpm: 90 }, PARTS);
    expect(registeredStructure()?.bpm).toBe(90);
  });

  test('the pattern keeps its own structure after the registry changes', () => {
    const pat = song(META, PARTS);
    beginEvaluate();
    song({ sections: [{ name: 'only', bars: 1 }] }, { only: 'o' });
    expect(values(pat, 0, 3)).toEqual(['i0', 'i1', 'b0']);
  });
});

describe('the demo song template', () => {
  test('evaluates through the runtime to a structure and queries without error', async () => {
    const templateRs = readFileSync(
      new URL('../crates/strudeland-project/src/template.rs', import.meta.url),
      'utf8',
    );
    const code = /pub const MAIN_SONG: &str = r#"([\s\S]*?)"#;/.exec(templateRs)![1]!;
    // The editor's slider widget is not part of the runtime; a constant stands in for it.
    await core.evalScope({ sliderWithID: (_id: string, v: number) => core.reify(v) });
    // The runtime's `repl()` stacks the `$:` lines into one pattern.
    let pattern: any;
    const { rt } = fakeRuntime({ hooks: { afterEval: (a) => void (pattern = a.pattern) } });
    expect(await rt.evaluate(code)).toBe(true);
    rt.stop();
    let s: SongStructure | null = null;
    rt.store.subscribe((v) => (s = v.structure))();
    expect(s!.sections.map((x) => x.name)).toEqual(['intro', 'body', 'tail']);
    expect(s!.totalBars).toBe(8);
    expect(s!.bpm).toBe(120);
    // Two plain lines (kick, hat) and the song: every bar of the song and a second pass of the loop.
    const sounds = new Set(pattern.queryArc(0, 10).map((h: any) => h.value.s));
    expect([...sounds].sort()).toEqual(['sawtooth', 'sine', 'supersaw', 'white']);
    for (const [b, e] of [
      [0, 2],
      [2, 6],
      [6, 8],
    ] as const) {
      expect(pattern.queryArc(b, e).some((h: any) => h.value.s === 'supersaw')).toBe(true);
    }
  });
});
