import { describe, expect, test } from 'bun:test';
import {
  blockPlayhead,
  editableStructure,
  FALLBACK_BARS,
  fieldValue,
  hasEnding,
  litBars,
  minorTicks,
  mixedMeterRuler,
  parseInput,
  playheadPct,
  rulerBars,
  rulerMarks,
  rulerStride,
} from '../src/lib/project/timeline';
import { buildStructure, type Meter } from '../src/lib/strudel/structure';
import { catOf } from '../src/lib/theme/tones';

test('mixed meter places denominator-note beats and labels at written bar positions', () => {
  const s = buildStructure({
    sections: [{ name: 'phrase', bars: 4 }],
    meterChanges: [
      { bar: 2, meter: '3/8' },
      { bar: 3, meter: '7/8' },
    ],
  });
  expect(mixedMeterRuler(s)).toEqual({
    ticks: [
      { from: 0, to: 1, count: 4 },
      { from: 1, to: 2, count: 3 },
      { from: 2, to: 4, count: 14 },
    ],
    labels: [
      { bar: 1, meter: { num: 3, den: 8 } },
      { bar: 2, meter: { num: 7, den: 8 } },
    ],
  });
});

const song = buildStructure({
  bpm: 120,
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
});

describe('ruler', () => {
  test('stride is the smallest power of two with at most 32 labels', () => {
    expect(rulerStride(1)).toBe(1);
    expect(rulerStride(32)).toBe(1);
    expect(rulerStride(33)).toBe(2);
    expect(rulerStride(64)).toBe(2);
    expect(rulerStride(65)).toBe(4);
    expect(rulerStride(10000)).toBe(512);
  });
  test('marks are multiples of the stride and never exceed 32', () => {
    expect(rulerMarks(4)).toEqual([0, 1, 2, 3]);
    expect(rulerMarks(0)).toEqual([]);
    for (const total of [1, 31, 32, 33, 100, 999, 10000]) {
      const marks = rulerMarks(total);
      expect(marks.length).toBeLessThanOrEqual(32);
      expect(marks.every((m) => m % rulerStride(total) === 0 && m < total)).toBe(true);
    }
    expect(rulerMarks(40)).toEqual(Array.from({ length: 20 }, (_, i) => i * 2));
  });
  test('rulerBars falls back to 16 with no sections', () => {
    expect(rulerBars(song)).toBe(8);
    expect(rulerBars(buildStructure({ bpm: 100 }))).toBe(FALLBACK_BARS);
    expect(rulerBars(null)).toBe(FALLBACK_BARS);
  });
});

describe('hasEnding', () => {
  test('needs a loop section followed by another section', () => {
    expect(hasEnding(song)).toBe(true);
    expect(hasEnding(null)).toBe(false);
    expect(hasEnding(buildStructure({}))).toBe(false);
    const noTail = buildStructure({
      sections: [
        { name: 'a', bars: 2 },
        { name: 'b', bars: 2, loop: true },
      ],
    });
    expect(hasEnding(noTail)).toBe(false);
    const noLoop = buildStructure({
      sections: [
        { name: 'a', bars: 2 },
        { name: 'b', bars: 2 },
      ],
    });
    expect(hasEnding(noLoop)).toBe(false);
  });
});

describe('blockPlayhead', () => {
  test('the located section and how far into its block, by the static bars', () => {
    expect(blockPlayhead(song, { section: 1, localCycle: 1, songBar: 3 })).toEqual({
      section: 1,
      fraction: 0.25,
      localCycle: 1,
    });
    expect(blockPlayhead(song, { section: 0, localCycle: 1.5, songBar: 1.5 })).toEqual({
      section: 0,
      fraction: 0.75,
      localCycle: 1.5,
    });
  });
  test('clamped when the block is shorter than what plays (an edited header)', () => {
    const shorter = buildStructure({ sections: [{ name: 'intro', bars: 1 }] });
    expect(blockPlayhead(shorter, { section: 0, localCycle: 1.5, songBar: 1.5 })).toEqual({
      section: 0,
      fraction: 1,
      localCycle: 1.5,
    });
  });
  test('localCycle stays raw when fraction is clamped', () => {
    const r = blockPlayhead(song, { section: 0, localCycle: -0.5, songBar: 0 });
    expect(r).toEqual({ section: 0, fraction: 0, localCycle: -0.5 });
    expect(blockPlayhead(song, { section: 2, localCycle: 7, songBar: 13 })).toEqual({
      section: 2,
      fraction: 1,
      localCycle: 7,
    });
  });
  test('none past the end, or for a section the static structure lacks', () => {
    expect(blockPlayhead(song, null)).toBeNull();
    expect(blockPlayhead(null, { section: 0, localCycle: 0, songBar: 0 })).toBeNull();
    expect(blockPlayhead(song, { section: 3, localCycle: 0, songBar: 8 })).toBeNull();
  });
});

describe('litBars', () => {
  test('the bars reached so far, the current one included', () => {
    expect(litBars(4, 0)).toBe(1);
    expect(litBars(4, 0.999)).toBe(1);
    expect(litBars(4, 1)).toBe(2);
    expect(litBars(4, 2.5)).toBe(3);
    expect(litBars(4, 4 - 1e-9)).toBe(4);
  });
  test('clamped to the section', () => {
    expect(litBars(4, 4)).toBe(4);
    expect(litBars(4, 100)).toBe(4);
    expect(litBars(4, -1)).toBe(1);
    expect(litBars(4, -0.001)).toBe(1);
  });
  test('none for a section without bars', () => {
    expect(litBars(0, 0)).toBe(0);
    expect(litBars(0, 3)).toBe(0);
    expect(litBars(-2, 1)).toBe(0);
    expect(litBars(NaN, 1)).toBe(0);
  });
});

describe('minorTicks', () => {
  const m = (s: string): Meter => {
    const [num, den] = s.split('/').map(Number);
    return { num, den };
  };
  test('one per beat for a whole number of beats from 2 to 8', () => {
    expect(minorTicks(8, m('4/4'))).toBe(32);
    expect(minorTicks(8, m('6/8'))).toBe(24);
    expect(minorTicks(2, m('2/4'))).toBe(4);
    expect(minorTicks(1, m('8/4'))).toBe(8);
    expect(minorTicks(32, m('3/4'))).toBe(96);
  });
  test('none outside 2..8 beats or for a fractional beat count', () => {
    expect(minorTicks(8, m('7/8'))).toBe(0);
    expect(minorTicks(8, m('1/4'))).toBe(0);
    expect(minorTicks(8, m('2/8'))).toBe(0);
    expect(minorTicks(8, m('9/4'))).toBe(0);
    expect(minorTicks(8, m('5/8'))).toBe(0);
  });
  test('none when the ruler skips bars, or with no bars', () => {
    expect(minorTicks(33, m('4/4'))).toBe(0);
    expect(minorTicks(0, m('4/4'))).toBe(0);
    expect(minorTicks(-1, m('4/4'))).toBe(0);
  });
});

describe('section categories', () => {
  test('catOf cycles 1, 2, 3, 4, 1', () => {
    expect([0, 1, 2, 3, 4].map(catOf)).toEqual([1, 2, 3, 4, 1]);
  });
});

describe('playheadPct (the ruler, for a song without sections)', () => {
  test('cycle modulo 16', () => {
    const plain = buildStructure({});
    expect(playheadPct(plain, { cycle: 4 })).toBe(25);
    expect(playheadPct(null, { cycle: 20 })).toBe(25);
    expect(playheadPct(null, { cycle: -1 })).toBe(0);
  });
  test('none for an evaluated song with sections: its playhead is in the blocks', () => {
    expect(playheadPct(song, { cycle: 9 })).toBeNull();
  });
});

describe('editableStructure', () => {
  test('the active structure, or none while the header has a meta error', () => {
    expect(editableStructure({ structure: song, metaError: null })).toBe(song);
    expect(editableStructure({ structure: null, metaError: null })).toBeNull();
    const defaults = buildStructure({});
    expect(editableStructure({ structure: defaults, metaError: 'meta.bpm … (1:20)' })).toBeNull();
  });
});

describe('fieldValue', () => {
  test('what each input shows for the structure', () => {
    const s = buildStructure({
      ...{ bpm: 96, meter: '6/8', loops: 3 },
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'body', bars: 4, loop: true },
      ],
    });
    expect(fieldValue(s, 'bpm')).toBe('96');
    expect(fieldValue(s, 'meter')).toBe('6/8');
    expect(fieldValue(s, 'loops')).toBe('3');
    expect(fieldValue(s, 'bars', 1)).toBe('4');
    expect(fieldValue(s, 'bars', 5)).toBe('');
  });
});

describe('parseInput', () => {
  test('bpm', () => {
    expect(parseInput('bpm', ' 90 ')).toEqual({ ok: true, path: 'bpm', value: 90 });
    for (const bad of ['', 'x', '19', '401']) {
      const r = parseInput('bpm', bad);
      expect(r.ok).toBe(false);
    }
  });
  test('meter', () => {
    expect(parseInput('meter', '6/8')).toEqual({ ok: true, path: 'meter', value: '6/8' });
    expect(parseInput('meter', '4/3').ok).toBe(false);
    expect(parseInput('meter', '').ok).toBe(false);
  });
  test('loops needs a loop section and an integer', () => {
    expect(parseInput('loops', '2', { hasLoopSection: true })).toEqual({
      ok: true,
      path: 'loops',
      value: 2,
    });
    expect(parseInput('loops', '2').ok).toBe(false);
    expect(parseInput('loops', '1.5', { hasLoopSection: true }).ok).toBe(false);
    expect(parseInput('loops', '0', { hasLoopSection: true }).ok).toBe(false);
  });
  test('bars maps to the section path', () => {
    expect(parseInput('bars', '8', { section: 2 })).toEqual({
      ok: true,
      path: 'sections[2].bars',
      value: 8,
    });
    const bad = parseInput('bars', '0', { section: 2 });
    expect(bad).toEqual({ ok: false, error: 'bars must be an integer from 1 to 10000' });
    expect(parseInput('bars', '2.5').ok).toBe(false);
  });
});
