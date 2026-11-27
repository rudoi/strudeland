import { describe, expect, test } from 'bun:test';
import { MIC_SETTINGS_URL } from '../src/lib/recording/memo-ui';
import type { RecordingUiState } from '../src/lib/recording/types';
import {
  statusNotices,
  type StatusNotice,
  type StatusNoticesInput,
} from '../src/lib/status-notices';

const quiet: StatusNoticesInput = {
  runtime: { evalError: null, outputError: null, tempoOverride: false, engineWarnings: [] },
  session: { error: null, metaError: null, notice: null },
  prefs: { error: null },
  control: { status: 'off', path: null, detail: null },
  recording: null,
  player: null,
  platform: 'MacIntel',
};

interface Over {
  runtime?: Partial<StatusNoticesInput['runtime']>;
  session?: Partial<StatusNoticesInput['session']>;
  prefs?: Partial<StatusNoticesInput['prefs']>;
  control?: Partial<StatusNoticesInput['control']>;
  recording?: StatusNoticesInput['recording'];
  player?: StatusNoticesInput['player'];
  platform?: StatusNoticesInput['platform'];
}

/** `quiet` with overrides; the four store snapshots merge one level deep. */
const input = (over: Over = {}): StatusNoticesInput => ({
  ...quiet,
  ...over,
  runtime: { ...quiet.runtime, ...over.runtime },
  session: { ...quiet.session, ...over.session },
  prefs: { ...quiet.prefs, ...over.prefs },
  control: { ...quiet.control, ...over.control },
});

/** A recorder state with no devices, so there is no bluetooth notice. */
const rec = (p: Partial<RecordingUiState> = {}): RecordingUiState => ({
  phase: 'armed',
  countInBeat: null,
  takeStartedAtMs: null,
  devices: [],
  input: null,
  error: null,
  permission: 'authorized',
  warning: null,
  stem: null,
  lastStem: null,
  ...p,
});

const ids = (ns: StatusNotice[]) => ns.map((n) => n.id);

describe('statusNotices', () => {
  test('nothing to say', () => {
    expect(statusNotices(quiet)).toEqual([]);
    expect(statusNotices(input({ control: { status: 'listening', path: '/p' } }))).toEqual([]);
    expect(statusNotices(input({ control: { status: 'off' } }))).toEqual([]);
  });

  const rows: [string, Over, StatusNotice[]][] = [
    [
      'eval error',
      { runtime: { evalError: 'boom' } },
      [{ id: 'eval-error', tone: 'danger', text: 'error: boom' }],
    ],
    [
      'output error',
      { runtime: { outputError: 'device gone' } },
      [{ id: 'output-error', tone: 'danger', text: 'audio error: device gone' }],
    ],
    [
      'session error',
      { session: { error: 'the project changed' } },
      [{ id: 'session-error', tone: 'danger', text: 'the project changed' }],
    ],
    [
      'meta error',
      { session: { metaError: 'bpm must be a number' } },
      [{ id: 'meta-error', tone: 'danger', text: 'meta: bpm must be a number' }],
    ],
    [
      'prefs error',
      { prefs: { error: 'disk full' } },
      [{ id: 'prefs-error', tone: 'danger', text: 'prefs: disk full' }],
    ],
    [
      'tempo override',
      { runtime: { tempoOverride: true } },
      [{ id: 'tempo-override', tone: 'warning', text: 'setcps in the code overrides meta.bpm' }],
    ],
    [
      'engine warning without keys',
      { runtime: { engineWarnings: [{ signature: 's=gm_piano', keys: [], firstCycle: 3 }] } },
      [{ id: 'engine:s=gm_piano', tone: 'warning', text: 'rust engine skipped s=gm_piano' }],
    ],
    [
      'engine warning with keys',
      {
        runtime: {
          engineWarnings: [{ signature: 's=gm_piano', keys: ['vowel', 'phaser'], firstCycle: 3 }],
        },
      },
      [
        {
          id: 'engine:s=gm_piano',
          tone: 'warning',
          text: 'rust engine skipped s=gm_piano',
          title: 'keys: vowel, phaser',
        },
      ],
    ],
    [
      'Claude edit',
      { session: { notice: { kind: 'claude-edit', path: 'songs/a.js', title: 'Alpha' } } },
      [{ id: 'claude-edit', tone: 'warning', text: 'Claude edited Alpha' }],
    ],
    [
      'control in use',
      { control: { status: 'in-use', path: '/tmp/strudeland.sock', detail: null } },
      [
        {
          id: 'control-in-use',
          tone: 'warning',
          text: 'control socket in use',
          title: '/tmp/strudeland.sock',
        },
      ],
    ],
    [
      'control in use without a path',
      { control: { status: 'in-use', path: null, detail: null } },
      [{ id: 'control-in-use', tone: 'warning', text: 'control socket in use', title: '' }],
    ],
    [
      'control disabled',
      { control: { status: 'disabled', path: null, detail: 'bind failed' } },
      [{ id: 'control-disabled', tone: 'warning', text: 'control disabled: bind failed' }],
    ],
    [
      'control disabled without a detail',
      { control: { status: 'disabled', path: null, detail: null } },
      [{ id: 'control-disabled', tone: 'warning', text: 'control disabled: ' }],
    ],
    [
      'recording limit reached',
      { recording: rec({ error: { kind: 'limitReached', message: 'x' } }) },
      [
        {
          id: 'limit-reached',
          tone: 'warning',
          text: 'recording stopped at the length limit; the memo was saved',
          action: { kind: 'dismiss' },
        },
      ],
    ],
    [
      'memo playback error',
      { player: { status: 'error', stem: 'take-01', kind: 'audio', message: 'could not play' } },
      [
        {
          id: 'play-audio',
          tone: 'danger',
          text: 'could not play',
          action: { kind: 'dismiss-playback' },
        },
      ],
    ],
  ];
  for (const [name, over, expected] of rows) {
    test(`one source: ${name}`, () => {
      expect(statusNotices(input(over))).toEqual(expected);
    });
  }

  test('only the in-use notice keeps an empty title', () => {
    const [engine] = statusNotices(
      input({
        runtime: { engineWarnings: [{ signature: 's=gm_piano', keys: [], firstCycle: 3 }] },
      }),
    );
    expect('title' in engine!).toBe(false);
    expect(engine).toStrictEqual({
      id: 'engine:s=gm_piano',
      tone: 'warning',
      text: 'rust engine skipped s=gm_piano',
    });
    const [disabled] = statusNotices(
      input({ control: { status: 'disabled', path: null, detail: null } }),
    );
    expect('title' in disabled!).toBe(false);
    expect(disabled).toStrictEqual({
      id: 'control-disabled',
      tone: 'warning',
      text: 'control disabled: ',
    });
    const [inUse] = statusNotices(
      input({ control: { status: 'in-use', path: null, detail: null } }),
    );
    expect('title' in inUse!).toBe(true);
    expect(inUse!.title).toBe('');
    expect(inUse).toStrictEqual({
      id: 'control-in-use',
      tone: 'warning',
      text: 'control socket in use',
      title: '',
    });
  });

  test('the platform passes through to the recording notices', () => {
    const recording = rec({
      error: { kind: 'permissionDenied', message: 'Microphone access was denied' },
    });
    expect(statusNotices(input({ recording, platform: 'MacIntel' }))).toEqual([
      {
        id: 'permission',
        tone: 'danger',
        text: 'Microphone access was denied',
        action: { kind: 'link', label: 'open settings', href: MIC_SETTINGS_URL },
      },
    ]);
    const linux = statusNotices(input({ recording, platform: 'Linux x86_64' }));
    expect(linux).toEqual([
      {
        id: 'permission',
        tone: 'danger',
        text: 'Microphone access was denied · check the input device in sound settings',
      },
    ]);
    expect('action' in linux[0]!).toBe(false);
  });

  const pairs: [string, Over, string[]][] = [
    [
      'an eval error hides the output error',
      { runtime: { evalError: 'boom', outputError: 'device gone' } },
      ['eval-error'],
    ],
    [
      'an empty eval error shows the output error',
      { runtime: { evalError: '', outputError: 'device gone' } },
      ['output-error'],
    ],
    ['an empty session error shows nothing', { session: { error: '' } }, []],
    ['control off', { control: { status: 'off', path: '/p', detail: 'd' } }, []],
    ['control listening', { control: { status: 'listening', path: '/p', detail: 'd' } }, []],
    [
      'control in use',
      { control: { status: 'in-use', path: '/p', detail: 'd' } },
      ['control-in-use'],
    ],
    [
      'control disabled',
      { control: { status: 'disabled', path: '/p', detail: 'd' } },
      ['control-disabled'],
    ],
  ];
  for (const [name, over, expected] of pairs) {
    test(`either/or: ${name}`, () => {
      expect(ids(statusNotices(input(over)))).toEqual(expected);
    });
  }

  test('the order, everything at once', () => {
    const all: Over = {
      runtime: {
        evalError: 'e',
        outputError: 'o',
        tempoOverride: true,
        engineWarnings: [
          { signature: 'a', keys: ['k'], firstCycle: 0 },
          { signature: 'b', keys: [], firstCycle: 0 },
        ],
      },
      session: {
        error: 's',
        metaError: 'm',
        notice: { kind: 'claude-edit', path: 'songs/a.js', title: 'Alpha' },
      },
      prefs: { error: 'p' },
      control: { status: 'in-use', path: '/p', detail: null },
      recording: rec({
        error: { kind: 'limitReached', message: 'x' },
        warning: 'input level is low',
      }),
      player: { status: 'error', stem: 'take-01', kind: 'tooLong', message: 'too long' },
    };
    const first = statusNotices(input(all));
    expect(ids(first)).toEqual([
      'eval-error',
      'session-error',
      'meta-error',
      'prefs-error',
      'tempo-override',
      'engine:a',
      'engine:b',
      'claude-edit',
      'control-in-use',
      'limit-reached',
      'recording-warning',
      'play-too-long',
    ]);
    const second = statusNotices(
      input({
        ...all,
        runtime: { ...all.runtime, evalError: null },
        control: { status: 'disabled', path: '/p', detail: 'd' },
      }),
    );
    expect(ids(second)).toEqual([
      'output-error',
      'session-error',
      'meta-error',
      'prefs-error',
      'tempo-override',
      'engine:a',
      'engine:b',
      'claude-edit',
      'control-disabled',
      'limit-reached',
      'recording-warning',
      'play-too-long',
    ]);
    // The disabled control notice has no tooltip, as today.
    for (const [ns, titled] of [
      [first, ['engine:a', 'control-in-use']],
      [second, ['engine:a']],
    ] as const) {
      expect(new Set(ids(ns)).size).toBe(ns.length);
      expect(ids(ns.filter((n) => 'title' in n))).toEqual([...titled]);
      expect(ids(ns.filter((n) => 'action' in n))).toEqual(['limit-reached', 'play-too-long']);
      expect(ns.filter((n) => 'title' in n && 'action' in n)).toEqual([]);
    }
  });

  test('ids are stable per source', () => {
    const id = (over: Over) => ids(statusNotices(input(over)));
    expect(id({ runtime: { evalError: 'a' } })).toEqual(['eval-error']);
    expect(id({ runtime: { evalError: 'b' } })).toEqual(['eval-error']);
    expect(id({ session: { metaError: 'a' } })).toEqual(['meta-error']);
    expect(id({ session: { metaError: 'b' } })).toEqual(['meta-error']);
    expect(id({ control: { status: 'disabled', detail: 'a' } })).toEqual(['control-disabled']);
    expect(id({ control: { status: 'disabled', detail: 'b' } })).toEqual(['control-disabled']);
    expect(
      id({ runtime: { engineWarnings: [{ signature: 'x', keys: [], firstCycle: 0 }] } }),
    ).toEqual(['engine:x']);
    expect(
      id({ runtime: { engineWarnings: [{ signature: 'x', keys: ['k'], firstCycle: 0 }] } }),
    ).toEqual(['engine:x']);
  });
});
