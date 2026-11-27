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
  sectionItemHelp,
  timelineView,
  type SectionItemPlay,
  type TimelineViewInput,
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

describe('sectionItemHelp', () => {
  const TEXT = { stale: 'stale', started: 'started', taking: 'taking', stopped: 'stopped' };
  const IDLE: SectionItemPlay = {
    canPlayFrom: true,
    stopTarget: null,
    started: false,
    taking: false,
  };
  const TAKE_BACKING: SectionItemPlay = {
    canPlayFrom: false,
    stopTarget: 'take',
    started: true,
    taking: true,
  };
  const MEMO_BACKING: SectionItemPlay = {
    canPlayFrom: false,
    stopTarget: 'memo',
    started: true,
    taking: false,
  };
  const rows: [string, boolean, SectionItemPlay, string][] = [
    ['an unresolved choice is stale, even while busy', false, TAKE_BACKING, 'stale'],
    ['an unresolved choice is stale while stopped', false, IDLE, 'stale'],
    ['a take’s backing waits for the take', true, TAKE_BACKING, 'Stop the take first.'],
    ['a memo’s backing waits for the memo', true, MEMO_BACKING, 'Stop the memo first.'],
    [
      'a starting take (not started) waits for the take',
      true,
      { canPlayFrom: false, stopTarget: 'take', started: false, taking: true },
      'Stop the take first.',
    ],
    [
      'busy without a take or memo target reads as the memo',
      true,
      { ...IDLE, canPlayFrom: false, stopTarget: 'song', started: true },
      'Stop the memo first.',
    ],
    ['the song plays', true, { ...IDLE, stopTarget: 'song', started: true }, 'started'],
    [
      'started wins over taking',
      true,
      { canPlayFrom: true, stopTarget: 'take', started: true, taking: true },
      'started',
    ],
    [
      'a metronome-only take arms for the next play',
      true,
      { canPlayFrom: true, stopTarget: 'take', started: false, taking: true },
      'taking',
    ],
    ['stopped', true, IDLE, 'stopped'],
  ];
  for (const [why, resolves, play, expected] of rows) {
    test(why, () => expect(sectionItemHelp(resolves, play, TEXT)).toBe(expected));
  }
});

describe('timelineView', () => {
  const SESSION: TimelineViewInput['session'] = {
    active: 'songs/a.js',
    structure: song,
    metaError: null,
  };
  const RUNTIME: TimelineViewInput['runtime'] = { started: true, structure: song };
  const PLAYBACK: TimelineViewInput['playback'] = {
    playing: { path: 'songs/a.js', title: 'a' },
    transport: { view: { kind: 'song' }, loop: null, armed: null },
  };
  const POSITION: TimelineViewInput['position'] = {
    cycle: 3,
    located: { section: 1, localCycle: 1, songBar: 3 },
  };
  const input = (over: Partial<TimelineViewInput> = {}): TimelineViewInput => ({
    session: SESSION,
    runtime: RUNTIME,
    playback: PLAYBACK,
    position: POSITION,
    ...over,
  });
  const OTHER: TimelineViewInput['playback'] = {
    ...PLAYBACK,
    playing: { path: 'songs/b.js', title: 'b' },
  };
  const STOPPED: Partial<TimelineViewInput> = {
    runtime: { ...RUNTIME, started: false },
    playback: { ...PLAYBACK, playing: null },
  };
  const transport = (over: Partial<TimelineViewInput['playback']['transport']>) => ({
    ...PLAYBACK,
    transport: { ...PLAYBACK.transport, ...over },
  });

  test('the active song plays', () => {
    const view = timelineView(input());
    expect(view.playingActive).toBe(true);
    expect(view.playingOther).toBe(false);
    expect(view.head).toEqual({ section: 1, fraction: 0.25, localCycle: 1 });
    expect(view.pct).toBeNull();
    expect(view.ending).toBe(true);
    expect(view.endingEnabled).toBe(true);
    expect(view.endingHelp).toBe('Finish the loop pass, then play the ending once.');
  });

  test('another song plays: no playhead and no ending', () => {
    const view = timelineView(input({ playback: OTHER }));
    expect(view.playingOther).toBe(true);
    expect(view.playingActive).toBe(false);
    expect(view.head).toBeNull();
    expect(view.pct).toBeNull();
    expect(view.endingEnabled).toBe(false);
    expect(view.endingHelp).toBe('The ending belongs to the song currently playing.');
  });

  test('a sectionless song has the ruler playhead only while it is the one playing', () => {
    const plain = buildStructure({ bpm: 100 });
    const over = {
      session: { ...SESSION, structure: plain },
      runtime: { ...RUNTIME, structure: plain },
    };
    expect(timelineView(input({ ...over, playback: OTHER })).pct).toBeNull();
    expect(timelineView(input(over)).pct).toBe(18.75);
  });

  test('a sectionless header with an evaluated song that has sections has no ruler playhead', () => {
    const plain = buildStructure({ bpm: 100 });
    expect(timelineView(input({ session: { ...SESSION, structure: plain } })).pct).toBeNull();
  });

  test('an evaluated structure that differs from the header places the head by name', () => {
    const evaluated = buildStructure({
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'verse', bars: 2 },
        { name: 'body', bars: 4, loop: true },
        { name: 'tail', bars: 2 },
      ],
    });
    const view = timelineView(
      input({
        runtime: { ...RUNTIME, structure: evaluated },
        position: { cycle: 5, located: { section: 2, localCycle: 1, songBar: 5 } },
      }),
    );
    expect(view.head).toEqual({ section: 1, fraction: 0.25, localCycle: 1 });
    expect(view.playable).toBe(evaluated);
    const stopped = timelineView(
      input({ ...STOPPED, runtime: { started: false, structure: evaluated } }),
    );
    expect(stopped.playable).toBe(song);
  });

  test('armed while stopped: the armed name, and the stale position shows nothing', () => {
    const view = timelineView(
      input({ ...STOPPED, playback: { ...transport({ armed: 'body' }), playing: null } }),
    );
    expect(view.armedName).toBe('body');
    expect(view.head).toBeNull();
    expect(view.pct).toBeNull();
    expect(view.endingEnabled).toBe(false);
    expect(view.endingHelp).toBe('Start song playback to reach its defined loop.');
  });

  test('the armed name is gated on the editable structure', () => {
    const armed = { ...STOPPED, playback: { ...transport({ armed: 'body' }), playing: null } };
    const metaError = { ...SESSION, metaError: 'meta.bpm must be a number (1:20)' };
    expect(timelineView(input({ ...armed, session: metaError })).armedName).toBeNull();
    const none = { ...SESSION, structure: null };
    expect(timelineView(input({ ...armed, session: none })).armedName).toBeNull();
  });

  test('duplicate section names: the head is in the first match', () => {
    const header = {
      ...song,
      sections: [
        { name: 'intro', start: 0, bars: 2, loop: false },
        { name: 'body', start: 2, bars: 4, loop: true },
        { name: 'body', start: 6, bars: 2, loop: false },
        { name: 'tail', start: 8, bars: 2, loop: false },
      ],
      totalBars: 10,
    };
    const view = timelineView(input({ session: { ...SESSION, structure: header } }));
    expect(view.head).toEqual({ section: 1, fraction: 0.25, localCycle: 1 });
  });

  test('a section the header lacks has no head', () => {
    const evaluated = buildStructure({
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'bridge', bars: 4 },
      ],
    });
    const view = timelineView(input({ runtime: { ...RUNTIME, structure: evaluated } }));
    expect(view.playingActive).toBe(true);
    expect(view.head).toBeNull();
  });

  test('no located position while the active song plays', () => {
    const view = timelineView(input({ position: { cycle: 3, located: null } }));
    expect(view.head).toBeNull();
    expect(view.endingEnabled).toBe(false);
    expect(view.endingHelp).toBe('Available while the song plays through its defined loop.');
  });

  test('the ending follows what plays: the evaluated song while started, else the header', () => {
    const noTail = buildStructure({
      bpm: 120,
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'body', bars: 4, loop: true },
      ],
    });
    expect(timelineView(input({ runtime: { ...RUNTIME, structure: noTail } })).ending).toBe(false);
    const stopped = { ...STOPPED, runtime: { started: false, structure: noTail } };
    expect(timelineView(input(stopped)).ending).toBe(true);
  });

  test('audition names', () => {
    const loop = { section: 'body', names: ['body', 'tail'] };
    expect(timelineView(input({ playback: transport({ loop }) })).auditionNames).toEqual([
      'body',
      'tail',
    ]);
    expect(timelineView(input()).auditionNames).toEqual([]);
  });

  describe('ending help', () => {
    const rows: [string, Partial<TimelineViewInput>, string][] = [
      ['enabled', {}, 'Finish the loop pass, then play the ending once.'],
      [
        'another song plays',
        { playback: OTHER },
        'The ending belongs to the song currently playing.',
      ],
      ['stopped, nothing playing', STOPPED, 'Start song playback to reach its defined loop.'],
      [
        'the release is queued or playing',
        { playback: transport({ view: { kind: 'release' } }) },
        'The ending is already queued or playing.',
      ],
      [
        'not on a loop pass (in the tail)',
        { position: { cycle: 6, located: { section: 2, localCycle: 0, songBar: 6 } } },
        'Available while the song plays through its defined loop.',
      ],
      [
        'unreachable, kept for the order: another song wins over stopped',
        { runtime: { ...RUNTIME, started: false }, playback: OTHER },
        'The ending belongs to the song currently playing.',
      ],
    ];
    for (const [why, over, expected] of rows) {
      test(why, () => expect(timelineView(input(over)).endingHelp).toBe(expected));
    }
  });

  describe('stop looping help', () => {
    const loop = transport({ view: { kind: 'loop', section: 1 } });
    const rows: [string, Partial<TimelineViewInput>, string][] = [
      ['no section loop', {}, 'There’s no temporary section loop to clear.'],
      [
        'a section loop while started',
        { playback: loop },
        'Return to the song’s normal playback at the next available bar line.',
      ],
      [
        'a section loop while stopped',
        { runtime: { ...RUNTIME, started: false }, playback: { ...loop, playing: null } },
        'Clear the temporary section loop for the next play.',
      ],
    ];
    for (const [why, over, expected] of rows) {
      test(why, () => expect(timelineView(input(over)).stopLoopingHelp).toBe(expected));
    }
  });
});
