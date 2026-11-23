import { describe, expect, test } from 'bun:test';
import {
  barsProblem,
  bpmFromCps,
  bpmProblem,
  seedProblem,
  engineProblem,
  buildStructure,
  cpsFor,
  hasLoopSection,
  DEFAULT_BPM,
  DEFAULT_METER,
  loopsProblem,
  meterProblem,
  meterString,
  parseMeter,
  sectionsProblem,
  StructureError,
  withLoops,
} from '../src/lib/strudel/structure';

const BPM_MSG = 'meta.bpm must be a number from 20 to 400';
const METER_MSG =
  "meta.meter must be a string 'N/D' with N from 1 to 32 and D one of 1, 2, 4, 8, 16";
const LOOPS_MSG = 'meta.loops must be an integer from 1 to 1000';
const NO_LOOP_MSG = 'meta.loops needs a section with loop: true or a meta.loop range';

describe('tempo', () => {
  test('cpsFor', () => {
    expect(cpsFor(120, { num: 4, den: 4 })).toBe(0.5);
    expect(cpsFor(120, { num: 6, den: 8 })).toBeCloseTo(2 / 3, 12);
    expect(cpsFor(120, { num: 11, den: 8 })).toBeCloseTo(120 / 60 / 5.5, 12);
    expect(cpsFor(120, { num: 1, den: 16 })).toBe(8);
    expect(cpsFor(400, { num: 1, den: 16 })).toBeCloseTo(26.666666666666668, 12);
    expect(cpsFor(60, { num: 3, den: 4 })).toBeCloseTo(1 / 3, 12);
  });

  test('bpmFromCps round-trips', () => {
    for (const meter of [
      { num: 4, den: 4 },
      { num: 6, den: 8 },
      { num: 11, den: 8 },
      { num: 1, den: 16 },
      { num: 32, den: 1 },
    ]) {
      for (const bpm of [20, 92.5, 120, 400]) {
        expect(bpmFromCps(cpsFor(bpm, meter), meter)).toBeCloseTo(bpm, 9);
      }
    }
    expect(bpmFromCps(0.5, DEFAULT_METER)).toBe(120);
  });

  test('meterString and parseMeter', () => {
    expect(meterString({ num: 6, den: 8 })).toBe('6/8');
    expect(parseMeter('6/8')).toEqual({ num: 6, den: 8 });
    expect(parseMeter('32/1')).toEqual({ num: 32, den: 1 });
    expect(parseMeter('1/16')).toEqual({ num: 1, den: 16 });
    for (const bad of ['', '4', '4/', '/4', '0/4', '33/4', '4/3', '4/32', '4/0', ' 4/4', '4/4 ']) {
      expect(parseMeter(bad)).toBeNull();
    }
    expect(parseMeter('-4/4')).toBeNull();
    expect(parseMeter('4.0/4')).toBeNull();
    expect(parseMeter('4 / 4')).toBeNull();
  });
});

describe('rules', () => {
  test('bpm', () => {
    for (const ok of [20, 20.5, 120, 400]) expect(bpmProblem(ok)).toBeNull();
    for (const bad of [19.99, 401, 0, -120, NaN, Infinity, '120', null, undefined]) {
      expect(bpmProblem(bad)).toBe(BPM_MSG);
    }
  });

  test('seed', () => {
    for (const ok of [0, -3, 7, 1e9, -2147483648, 2147483647]) expect(seedProblem(ok)).toBeNull();
    for (const bad of [
      1.5,
      -0.5,
      2147483648,
      -2147483649,
      1e15,
      NaN,
      Infinity,
      -Infinity,
      '1',
      null,
      undefined,
      true,
    ]) {
      expect(seedProblem(bad)).toBe(
        'meta.seed must be an integer between -2147483648 and 2147483647',
      );
    }
  });

  test('engine', () => {
    expect(engineProblem('superdough')).toBeNull();
    expect(engineProblem('rust')).toBeNull();
    for (const bad of ['Rust', 'x', '', 3, null, undefined, true, ['rust']]) {
      expect(engineProblem(bad)).toBe("meta.engine must be 'superdough' or 'rust'");
    }
  });

  test('buildStructure ignores the engine', () => {
    expect(buildStructure({ engine: 'rust' })).toEqual(buildStructure({}));
  });

  test('buildStructure ignores the seed', () => {
    expect(buildStructure({ seed: 5 })).toEqual(buildStructure({}));
  });

  test('meter', () => {
    expect(meterProblem('4/4')).toBeNull();
    expect(meterProblem('11/8')).toBeNull();
    for (const bad of ['4/3', '0/4', 4, null, { num: 4, den: 4 }]) {
      expect(meterProblem(bad)).toBe(METER_MSG);
    }
  });

  test('bars', () => {
    for (const ok of [1, 16, 10000]) expect(barsProblem(ok)).toBeNull();
    for (const bad of [0, -1, 1.5, 10001, '4', null, NaN]) {
      expect(barsProblem(bad)).toBe('bars must be an integer from 1 to 10000');
    }
    expect(barsProblem(0, 'meta.sections[2].bars')).toBe(
      'meta.sections[2].bars must be an integer from 1 to 10000',
    );
  });

  test('loops', () => {
    expect(loopsProblem(1, true)).toBeNull();
    expect(loopsProblem(1000, true)).toBeNull();
    for (const bad of [0, 1001, 2.5, '2', null]) expect(loopsProblem(bad, true)).toBe(LOOPS_MSG);
    expect(loopsProblem(2, false)).toBe(NO_LOOP_MSG);
    expect(loopsProblem(0, false)).toBe(LOOPS_MSG);
  });

  test('sections: valid', () => {
    expect(sectionsProblem([{ name: 'a', bars: 1 }])).toBeNull();
    expect(
      sectionsProblem([
        { name: 'intro', bars: 4, loop: false },
        { name: '_body2', bars: 16, loop: true },
        { name: 'Tail', bars: 2 },
      ]),
    ).toBeNull();
    const many = Array.from({ length: 64 }, (_, i) => ({ name: `s${i}`, bars: 1 }));
    expect(sectionsProblem(many)).toBeNull();
  });

  test('sections: shape', () => {
    const msg = 'meta.sections must be a non-empty array of sections';
    for (const bad of [[], {}, 'intro', null, undefined, 4]) expect(sectionsProblem(bad)).toBe(msg);
    const many = Array.from({ length: 65 }, (_, i) => ({ name: `s${i}`, bars: 1 }));
    expect(sectionsProblem(many)).toBe('meta.sections must have at most 64 sections');
    for (const bad of [null, 'a', 4, [{ name: 'a', bars: 1 }]]) {
      expect(sectionsProblem([{ name: 'a', bars: 1 }, bad])).toBe(
        "meta.sections[1] must be an object like { name: 'intro', bars: 4 }",
      );
    }
  });

  test('sections: names', () => {
    const msg = (i: number) =>
      `meta.sections[${i}].name must be an identifier (letters, digits and _, not starting with a digit)`;
    for (const bad of ['', '1a', 'a-b', 'a b', 'é', 5, null]) {
      expect(sectionsProblem([{ name: bad, bars: 1 }])).toBe(msg(0));
    }
    expect(sectionsProblem([{ bars: 1 }])).toBe(msg(0));
    expect(
      sectionsProblem([
        { name: 'a', bars: 1 },
        { name: 'b', bars: 1 },
        { name: 'a', bars: 1 },
      ]),
    ).toBe("meta.sections[2].name 'a' is already used");
  });

  test('sections: bars, loop and unknown keys', () => {
    expect(sectionsProblem([{ name: 'a', bars: 1 }, { name: 'b' }])).toBe(
      'meta.sections[1].bars must be an integer from 1 to 10000',
    );
    expect(sectionsProblem([{ name: 'a', bars: 10001 }])).toBe(
      'meta.sections[0].bars must be an integer from 1 to 10000',
    );
    expect(sectionsProblem([{ name: 'a', bars: 1, loop: 'yes' }])).toBe(
      'meta.sections[0].loop must be true or false',
    );
    expect(sectionsProblem([{ name: 'a', bars: 1, loop: null }])).toBe(
      'meta.sections[0].loop must be true or false',
    );
    expect(
      sectionsProblem([
        { name: 'a', bars: 1, loop: true },
        { name: 'b', bars: 1, loop: false },
        { name: 'c', bars: 1, loop: true },
      ]),
    ).toBe('meta.sections[2].loop: only one section can loop');
    expect(sectionsProblem([{ name: 'a', bars: 1, fade: 2 }])).toBe(
      "meta.sections[0] has an unknown key 'fade'",
    );
  });
});

describe('buildStructure', () => {
  test('defaults for an empty meta', () => {
    expect(buildStructure({})).toEqual({
      bpm: DEFAULT_BPM,
      meter: DEFAULT_METER,
      beatsPerBar: 4,
      cps: 0.5,
      sections: [],
      loopIndex: null,
      loops: 1,
      totalBars: 0,
      introEnd: 0,
      endOfSong: 0,
      clipBars: 0,
      clipSeconds: 0,
    });
  });

  test('undefined values count as absent; unknown keys are ignored', () => {
    const s = buildStructure({ bpm: undefined, meter: undefined, title: 5, duration: 'x' });
    expect(s.bpm).toBe(120);
    expect(s.meter).toEqual({ num: 4, den: 4 });
  });

  test('the default meter is not shared', () => {
    const s = buildStructure({});
    s.meter.num = 3;
    expect(DEFAULT_METER).toEqual({ num: 4, den: 4 });
  });

  test('tempo and meter without sections', () => {
    const s = buildStructure({ bpm: 90, meter: '6/8' });
    expect(s.bpm).toBe(90);
    expect(s.meter).toEqual({ num: 6, den: 8 });
    expect(s.beatsPerBar).toBe(3);
    expect(s.cps).toBe(0.5);
    expect(s.sections).toEqual([]);
  });

  test('intro, loop and tail', () => {
    const s = buildStructure({
      bpm: 120,
      sections: [
        { name: 'intro', bars: 4 },
        { name: 'build', bars: 2 },
        { name: 'body', bars: 16, loop: true },
        { name: 'tail', bars: 2 },
      ],
      loops: 3,
    });
    expect(s.sections).toEqual([
      { name: 'intro', start: 0, bars: 4, loop: false },
      { name: 'build', start: 4, bars: 2, loop: false },
      { name: 'body', start: 6, bars: 16, loop: true },
      { name: 'tail', start: 22, bars: 2, loop: false },
    ]);
    expect(s.loopIndex).toBe(2);
    expect(s.loops).toBe(3);
    expect(s.totalBars).toBe(24);
    expect(s.introEnd).toBe(6);
    expect(s.endOfSong).toBeNull();
    expect(s.clipBars).toBe(6 + 3 * 16 + 2);
    expect(s.clipSeconds).toBe(56 / 0.5);
  });

  test('loops defaults to 1', () => {
    const s = buildStructure({
      sections: [
        { name: 'a', bars: 2 },
        { name: 'b', bars: 4, loop: true },
        { name: 'c', bars: 1 },
      ],
    });
    expect(s.loops).toBe(1);
    expect(s.clipBars).toBe(7);
  });

  test('no intro (loop first) and no tail (loop last)', () => {
    const noIntro = buildStructure({
      sections: [
        { name: 'body', bars: 8, loop: true },
        { name: 'end', bars: 2 },
      ],
      loops: 2,
    });
    expect(noIntro.loopIndex).toBe(0);
    expect(noIntro.introEnd).toBe(0);
    expect(noIntro.clipBars).toBe(18);

    const noTail = buildStructure({
      sections: [
        { name: 'in', bars: 2 },
        { name: 'body', bars: 8, loop: true },
      ],
      loops: 4,
    });
    expect(noTail.loopIndex).toBe(1);
    expect(noTail.introEnd).toBe(2);
    expect(noTail.totalBars).toBe(10);
    expect(noTail.clipBars).toBe(34);
  });

  test('no loop section plays straight through', () => {
    const s = buildStructure({
      bpm: 60,
      meter: '3/4',
      sections: [
        { name: 'a', bars: 3 },
        { name: 'b', bars: 5, loop: false },
      ],
    });
    expect(s.loopIndex).toBeNull();
    expect(s.introEnd).toBe(8);
    expect(s.endOfSong).toBe(8);
    expect(s.clipBars).toBe(8);
    expect(s.cps).toBeCloseTo(1 / 3, 12);
    expect(s.clipSeconds).toBeCloseTo(24, 9);
  });

  test('throws StructureError with the first problem', () => {
    const cases: [Record<string, unknown>, string][] = [
      [{ bpm: 10 }, BPM_MSG],
      [{ bpm: null }, BPM_MSG],
      [{ meter: '4/3' }, METER_MSG],
      [{ sections: [] }, 'meta.sections must be a non-empty array of sections'],
      [
        { sections: [{ name: 'a', bars: 0 }] },
        'meta.sections[0].bars must be an integer from 1 to 10000',
      ],
      [{ loops: 2 }, NO_LOOP_MSG],
      [{ sections: [{ name: 'a', bars: 1 }], loops: 2 }, NO_LOOP_MSG],
      [{ sections: [{ name: 'a', bars: 1, loop: true }], loops: 0 }, LOOPS_MSG],
      [{ bpm: 1, meter: 'x' }, BPM_MSG],
    ];
    for (const [meta, msg] of cases) {
      let err: unknown;
      try {
        buildStructure(meta);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(StructureError);
      expect(err).toBeInstanceOf(Error);
      expect((err as Error).message).toBe(msg);
    }
  });
});

describe('hasLoopSection', () => {
  test('an array with a { loop: true } entry, checked or not', () => {
    expect(hasLoopSection([{ name: 'a', bars: 1 }, { loop: true }])).toBe(true);
    expect(hasLoopSection([{ name: 'a', bars: 1, loop: false }])).toBe(false);
    expect(hasLoopSection([5, null, [true], { loop: 'yes' }])).toBe(false);
    expect(hasLoopSection({ loop: true })).toBe(false);
    expect(hasLoopSection(undefined)).toBe(false);
  });
});

describe('withLoops', () => {
  const meta = {
    bpm: 90,
    meter: '3/4',
    loops: 2,
    sections: [
      { name: 'intro', bars: 4 },
      { name: 'main', bars: 8, loop: true },
      { name: 'outro', bars: 3 },
    ],
  };

  test('equals buildStructure with the loop count replaced', () => {
    for (const loops of [1, 2, 3, 1000]) {
      expect(withLoops(buildStructure(meta), loops)).toEqual(buildStructure({ ...meta, loops }));
    }
    expect(withLoops(buildStructure(meta), 5).clipBars).toBe(4 + 5 * 8 + 3);
  });

  test('throws the meta.loops problem', () => {
    const s = buildStructure(meta);
    for (const loops of [0, 1001, 1.5, NaN]) {
      expect(() => withLoops(s, loops)).toThrow(new StructureError(LOOPS_MSG));
    }
    const noLoop = buildStructure({ sections: [{ name: 'a', bars: 2 }] });
    expect(() => withLoops(noLoop, 2)).toThrow(new StructureError(NO_LOOP_MSG));
    expect(() => withLoops(buildStructure({}), 2)).toThrow(new StructureError(NO_LOOP_MSG));
  });
});
