import { describe, expect, test } from 'bun:test';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { decodeWav } from '../src/lib/render/wav';
import { GATE_NAMES, type GateName } from '../src/lib/render/gates/types';
import {
  MAX_WINDOW_BARS,
  samplesMap,
  validateCorpus,
  type Corpus,
  type CorpusIo,
} from '../src/lib/render/corpus';
import { parseMeta } from '../src/lib/project/meta';
import { buildStructure } from '../src/lib/strudel/structure';
import { hat, kick, makeSamples, SAMPLE_RATE, tone } from '../scripts/render/make-samples';

const DIR = new URL('./render/corpus/', import.meta.url).pathname;
const MANIFEST = new URL('./render/corpus.json', import.meta.url).pathname;

const fsIo: CorpusIo = {
  readSong: (f) => (existsSync(DIR + f) ? readFileSync(DIR + f, 'utf8') : null),
  sampleExists: (f) => existsSync(`${DIR}samples/${f}`),
  listSongs: () => readdirSync(DIR).filter((f) => f.endsWith('.js')),
};

const realCorpus = (): Corpus => {
  const { corpus, errors } = validateCorpus(JSON.parse(readFileSync(MANIFEST, 'utf8')), fsIo);
  expect(errors).toEqual([]);
  return corpus!;
};

// ---- fakes

const SONG = `const meta = { title: 'a', bpm: 120, meter: '4/4' };\n$: note("c3")\n`;
const SECTIONED = `const meta = {
  sections: [{ name: 'intro', bars: 2 }, { name: 'body', bars: 4, loop: true }, { name: 'tail', bars: 2 }],
  loops: 2,
};
$: song(meta, {})
`;
const NO_LOOP = `const meta = { sections: [{ name: 'a', bars: 4 }] };\n$: note("c3")\n`;
const SAMPLED = `await samples({ kick: 'kick.wav' }, '/samples/');\n$: s("kick")\n`;

function fakeIo(
  songs: Record<string, string> = { 'a.js': SONG },
  samples: string[] = ['kick.wav'],
): CorpusIo {
  return {
    readSong: (f) => songs[f] ?? null,
    sampleExists: (f) => samples.includes(f),
    listSongs: () => Object.keys(songs),
  };
}

const fixture = (over: Record<string, unknown> = {}) => ({
  name: 'a',
  file: 'a.js',
  kind: 'targeted',
  windowBars: [0, 4],
  layout: 'linear',
  gates: ['twin', 'mix'],
  stochastic: false,
  usesSamples: false,
  rust: 'supported',
  tags: [],
  ...over,
});
const manifest = (fixtures: unknown[] = [fixture()], over: Record<string, unknown> = {}) => ({
  schema: 1,
  samples: ['kick.wav'],
  fixtures,
  ...over,
});

function errorsOf(raw: unknown, io: CorpusIo = fakeIo()) {
  const { corpus, errors } = validateCorpus(raw, io);
  if (errors.length > 0) expect(corpus).toBeNull();
  return errors;
}
const messages = (raw: unknown, io?: CorpusIo) => errorsOf(raw, io).map((e) => e.message);

// ---- the real corpus

describe('the committed corpus', () => {
  test('corpus.json validates against tests/render/corpus/', () => {
    const corpus = realCorpus();
    expect(corpus.fixtures.every((f) => f.kind === 'targeted')).toBe(true);
  });

  test('host-shims carries the host tag and every shim', () => {
    const fixture = realCorpus().fixtures.find((f) => f.name === 'host-shims');
    expect(fixture?.tags).toContain('host');
    const source = readFileSync(DIR + 'host-shims.js', 'utf8');
    for (const shim of [
      'scope',
      '_scope',
      '_pianoroll',
      'slider',
      'markcss',
      'pianoroll',
      'perlin',
      'samples(',
      'aliasBank(',
    ]) {
      expect(source).toContain(shim);
    }
  });

  test('the rust-supported fixtures are exactly the in-scope ones', () => {
    const supported = realCorpus()
      .fixtures.filter((f) => f.rust === 'supported')
      .map((f) => f.name);
    expect(supported.sort()).toEqual(
      [
        'bass-line',
        'stems-5',
        'stems-shared',
        'silence-entry-perc',
        'silence-entry-pad',
        'odd-meter',
        'late-feedback',
        'crowded-bass',
        'asymmetric-delay',
        'filtered-opening',
        'clipping',
        'pad-seam',
        'reverb-long',
        'reverb-change',
        'sine-basic',
        'host-shims',
        'voice-osc',
        'voice-mod',
        'voice-noise',
        'voice-filter',
        'voice-level',
        'fx-delay',
        'fx-room',
        'worklets',
        'song-linear',
        'song-clip',
        'duck',
        'voice-worklets',
        'fx-native',
        'fx-mod',
        'fx-bus',
        'fx-chain',
      ].sort(),
    );
  });

  test('every rust-unsupported fixture says why in its notes', () => {
    for (const f of realCorpus().fixtures.filter((x) => x.rust === 'unsupported')) {
      expect(f.notes ?? '').toMatch(/rust: /);
    }
  });

  test('no committed fixture is stochastic', () => {
    expect(realCorpus().fixtures.every((f) => !f.stochastic)).toBe(true);
  });

  test.each([
    ['late-feedback', '9/8', 1.5],
    ['crowded-bass', '6/8', 1.2],
    ['asymmetric-delay', '11/8', 330 / 145],
    ['filtered-opening', '4/4', 12],
  ])('%s plans its synthetic meter at the expected bar duration', (name, meter, barSeconds) => {
    const { meta, error } = parseMeta(readFileSync(`${DIR}${name}.js`, 'utf8'));
    expect(error).toBeNull();
    expect(meta.meter).toBe(meter);
    expect(1 / buildStructure(meta).cps).toBeCloseTo(barSeconds, 8);
  });

  test('late-feedback keeps a nonzero-start multi-chunk window through bar 160', () => {
    const fixture = realCorpus().fixtures.find((f) => f.name === 'late-feedback');
    expect(fixture?.windowBars).toEqual([136, 160]);
    expect(fixture?.gates).toContain('seam');
    expect(fixture?.expectChunks).toBeGreaterThanOrEqual(2);
  });

  test('targeted fixtures carry the fixture header and no export or setcps', () => {
    for (const f of realCorpus().fixtures.filter((x) => x.kind === 'targeted')) {
      const code = readFileSync(DIR + f.file, 'utf8');
      expect(code.startsWith('// Corpus fixture: ')).toBe(true);
      expect(code).not.toContain('export ');
      expect(code).not.toMatch(/setcp[sm]/);
    }
  });

  test('song-linear and song-clip windows cover the whole song', () => {
    const corpus = realCorpus();
    const s = buildStructure(parseMeta(readFileSync(`${DIR}song-linear.js`, 'utf8')).meta);
    const linear = corpus.fixtures.find((f) => f.name === 'song-linear')!;
    const clip = corpus.fixtures.find((f) => f.name === 'song-clip')!;
    expect(linear.layout).toBe('linear');
    expect(linear.windowBars).toEqual([0, s.totalBars]);
    expect(clip.layout).toBe('clip');
    expect(clip.file).toBe('song-linear.js');
    expect(clip.windowBars).toEqual([0, s.clipBars]);
    expect(s.clipBars).toBe(12);
  });

  test('odd-meter is 7/8 at cps 0.6524', () => {
    const s = buildStructure(parseMeta(readFileSync(`${DIR}odd-meter.js`, 'utf8')).meta);
    expect(s.cps).toBeCloseTo(0.6524, 4);
  });

  test('noise-seed sets meta.seed 7', () => {
    expect(parseMeta(readFileSync(`${DIR}noise-seed.js`, 'utf8')).meta.seed).toBe(7);
  });

  test('the synth and effect fixtures are short, deterministic twin+mix fixtures', () => {
    const names = [
      'voice-osc',
      'voice-mod',
      'voice-noise',
      'voice-filter',
      'voice-level',
      'fx-delay',
      'fx-room',
    ];
    for (const name of names) {
      const f = realCorpus().fixtures.find((x) => x.name === name)!;
      expect(f, name).toBeDefined();
      expect(f.file).toBe(`${name}.js`);
      expect(f.kind).toBe('targeted');
      expect(f.windowBars[1] - f.windowBars[0]).toBeLessThanOrEqual(8);
      expect(f.gates).toEqual(['twin', 'mix']);
      expect(f.stochastic).toBe(false);
      expect(f.usesSamples).toBe(false);
      expect(f.rust).toBe('supported');
    }
    // voice-noise draws its noise from a non-default seed; fx-delay's delaysync runs at a
    // non-default cps
    expect(parseMeta(readFileSync(`${DIR}voice-noise.js`, 'utf8')).meta.seed).toBe(11);
    const delay = buildStructure(parseMeta(readFileSync(`${DIR}fx-delay.js`, 'utf8')).meta);
    expect(delay.cps).toBeCloseTo(100 / 240, 12);
  });

  test('the effect fixtures: linear, deterministic, supported, with their windows, tempi, gates, chunks and tags', () => {
    const want: Record<
      string,
      { bars: [number, number]; cps: number; gates: GateName[]; chunks?: number; tags: string[] }
    > = {
      'voice-worklets': {
        bars: [0, 8],
        cps: 0.5,
        gates: ['twin', 'mix'],
        tags: ['engine', 'worklet'],
      },
      'fx-native': { bars: [0, 8], cps: 0.5, gates: ['twin', 'mix'], tags: ['engine', 'effects'] },
      'fx-mod': {
        bars: [0, 8],
        cps: 100 / 240,
        gates: ['twin', 'mix'],
        tags: ['engine', 'modulation'],
      },
      'fx-bus': {
        bars: [0, 12],
        cps: 0.5,
        gates: ['seam', 'twin', 'mix'],
        chunks: 2,
        tags: ['engine', 'bus', 'seam'],
      },
      'fx-chain': {
        bars: [0, 12],
        cps: 0.5,
        gates: ['seam', 'twin', 'mix'],
        chunks: 2,
        tags: ['engine', 'chain', 'seam'],
      },
    };
    const corpus = realCorpus();
    for (const [name, w] of Object.entries(want)) {
      const f = corpus.fixtures.find((x) => x.name === name)!;
      expect(f.file, name).toBe(`${name}.js`);
      expect(f.kind, name).toBe('targeted');
      expect(f.layout, name).toBe('linear');
      expect(f.windowBars, name).toEqual(w.bars);
      expect(f.gates, name).toEqual(w.gates);
      expect(f.expectChunks, name).toBe(w.chunks);
      expect(f.tags, name).toEqual(w.tags);
      expect(f.stochastic, name).toBe(false);
      expect(f.usesSamples, name).toBe(false);
      expect(f.rust, name).toBe('supported');
      expect(f.expect, name).toEqual({ unverifiable: [], clip: false });
      const { meta, error } = parseMeta(readFileSync(DIR + f.file, 'utf8'));
      expect(error, name).toBeNull();
      expect(Object.keys(meta).sort(), name).toEqual(['bpm', 'meter', 'title']);
      expect(meta.title, name).toBe(name);
      expect(meta.meter, name).toBe('4/4');
      expect(buildStructure(meta).cps, name).toBeCloseTo(w.cps, 12);
    }
  });

  test('samplesMap maps file stems to file names', () => {
    expect(samplesMap(realCorpus())).toEqual({
      kick: 'kick.wav',
      hat: 'hat.wav',
      tone: 'tone.wav',
    });
  });
});

// ---- samples

describe('makeSamples', () => {
  const SIZES: Record<string, [frames: number, bytes: number]> = {
    'kick.wav': [4410, 8864],
    'hat.wav': [2205, 4454],
    'tone.wav': [8820, 17684],
  };

  test('equals the committed bytes', () => {
    const made = makeSamples();
    expect(Object.keys(made)).toEqual(['kick.wav', 'hat.wav', 'tone.wav']);
    for (const [file, bytes] of Object.entries(made)) {
      expect(Buffer.from(bytes).equals(readFileSync(`${DIR}samples/${file}`))).toBe(true);
    }
  });

  test('is deterministic', () => {
    const a = makeSamples();
    const b = makeSamples();
    for (const f of Object.keys(a)) expect(Buffer.from(a[f]!).equals(b[f]!)).toBe(true);
  });

  test('sizes, rate and format match the spec', () => {
    for (const [file, bytes] of Object.entries(makeSamples())) {
      const [frames, size] = SIZES[file]!;
      expect(bytes.length).toBe(size);
      const wav = decodeWav(bytes);
      expect(wav.sampleRate).toBe(SAMPLE_RATE);
      expect(wav.channels.length).toBe(1);
      expect(wav.channels[0]!.length).toBe(frames);
    }
  });

  test('signals are audible, bounded and decay', () => {
    for (const x of [kick(), hat(), tone()]) {
      const max = Math.max(...Array.from(x, Math.abs));
      expect(max).toBeGreaterThan(0.1);
      expect(max).toBeLessThanOrEqual(1.1);
      expect(Math.abs(x[x.length - 1]!)).toBeLessThan(0.15 * max);
    }
    expect(kick()[0]).toBe(0); // sine from phase 0
    expect(tone()[0]).toBe(0); // linear attack from 0
  });
});

// ---- validation rules

describe('validateCorpus', () => {
  test('a minimal manifest validates', () => {
    const { corpus, errors } = validateCorpus(manifest(), fakeIo());
    expect(errors).toEqual([]);
    expect(corpus?.fixtures[0]?.name).toBe('a');
  });

  test('constants', () => {
    expect(GATE_NAMES).toEqual(['seam', 'twin', 'stems', 'bass', 'mix']);
    expect(MAX_WINDOW_BARS).toBe(24);
  });

  test('not an object', () => {
    expect(errorsOf(null)).toEqual([{ path: 'schema', message: 'schema must be 1' }]);
    expect(messages([])).toEqual(['schema must be 1']);
  });

  test('schema must be 1', () => {
    expect(messages(manifest(undefined, { schema: 2 }))).toEqual(['schema must be 1']);
  });

  test('unknown keys at every level', () => {
    expect(errorsOf(manifest(undefined, { extra: 1 }))).toEqual([
      { path: '', message: "unknown key 'extra'" },
    ]);
    expect(errorsOf(manifest([fixture({ x: 1 })]))).toEqual([
      { path: 'fixtures[0]', message: "unknown key 'x'" },
    ]);
    expect(errorsOf(manifest([fixture({ expect: { y: true } })]))).toEqual([
      { path: 'fixtures[0].expect', message: "unknown key 'y'" },
    ]);
  });

  test('fixtures must be a non-empty array', () => {
    expect(messages(manifest([]), fakeIo({}))).toEqual(['fixtures must be a non-empty array']);
    expect(messages(manifest(undefined, { fixtures: 'x' }), fakeIo({}))).toEqual([
      'fixtures must be a non-empty array',
    ]);
  });

  test('a fixture must be an object', () => {
    expect(messages(manifest([fixture(), 3]))).toEqual(['fixture must be an object']);
  });

  test('name pattern and uniqueness', () => {
    expect(errorsOf(manifest([fixture({ name: 'Bad_name' })]))).toEqual([
      { path: 'fixtures[0].name', message: 'name must match /^[a-z0-9][a-z0-9-]*$/' },
    ]);
    expect(errorsOf(manifest([fixture(), fixture()]))).toEqual([
      { path: 'fixtures[1].name', message: "duplicate fixture name 'a'" },
    ]);
  });

  test('file must be a .js name', () => {
    expect(messages(manifest([fixture({ file: 'a.mjs' })]))).toContain(
      'file must be a .js file name',
    );
  });

  test('song file not found', () => {
    expect(messages(manifest([fixture({ file: 'b.js' })]))).toEqual([
      "song file 'b.js' not found",
      "song 'a.js' is not in corpus.json",
    ]);
  });

  test('a meta error is reported with a meta: prefix', () => {
    const io = fakeIo({ 'a.js': `const meta = { bpm: 5 };\n$: note("c")\n` });
    expect(errorsOf(manifest(), io)).toEqual([
      { path: 'fixtures[0].file', message: 'meta: meta.bpm must be a number from 20 to 400' },
    ]);
  });

  test('several fixtures may share a file', () => {
    const io = fakeIo({ 'a.js': SECTIONED });
    const ok = manifest([
      fixture({ windowBars: [0, 8] }),
      fixture({ name: 'b', layout: 'clip', windowBars: [0, 12] }),
    ]);
    expect(messages(ok, io)).toEqual([]);
  });

  test('windowBars shape', () => {
    const msg = 'windowBars must be [from, to] integers with 0 <= from < to';
    for (const w of [[0], [0, 1.5], [-1, 2], [4, 4], [5, 2], 'x', [0, '4']]) {
      expect(errorsOf(manifest([fixture({ windowBars: w })]))).toEqual([
        { path: 'fixtures[0].windowBars', message: msg },
      ]);
    }
  });

  test('window longer than 24 bars', () => {
    expect(messages(manifest([fixture({ windowBars: [0, 25] })]))).toEqual([
      'window longer than 24 bars',
    ]);
    expect(messages(manifest([fixture({ windowBars: [10, 34] })]))).toEqual([]);
  });

  test('window ends after the song (linear and clip)', () => {
    const io = fakeIo({ 'a.js': SECTIONED });
    expect(messages(manifest([fixture({ windowBars: [0, 9] })]), io)).toEqual([
      'window ends after the song (8 bars)',
    ]);
    expect(messages(manifest([fixture({ layout: 'clip', windowBars: [0, 13] })]), io)).toEqual([
      'window ends after the song (12 bars)',
    ]);
  });

  test("layout 'clip' needs a loop section", () => {
    const msg = "layout 'clip' needs a loop section";
    expect(messages(manifest([fixture({ layout: 'clip' })]))).toEqual([msg]);
    const io = fakeIo({ 'a.js': NO_LOOP });
    expect(messages(manifest([fixture({ layout: 'clip' })]), io)).toEqual([msg]);
  });

  test('layout must be linear or clip', () => {
    expect(messages(manifest([fixture({ layout: 'loop' })]))).toEqual([
      "layout must be 'linear' or 'clip'",
    ]);
  });

  test('gates: non-empty, known, unique', () => {
    expect(messages(manifest([fixture({ gates: [] })]))).toEqual([
      'gates must be a non-empty array',
    ]);
    expect(errorsOf(manifest([fixture({ gates: ['twin', 'loud'] })]))).toEqual([
      { path: 'fixtures[0].gates[1]', message: "unknown gate 'loud'" },
    ]);
    expect(messages(manifest([fixture({ gates: ['mix', 'mix'] })]))).toEqual([
      "duplicate gate 'mix'",
    ]);
  });

  test('seam needs a window over 8 bars and expectChunks >= 2', () => {
    const msg = 'seam needs a window over 8 bars and expectChunks >= 2';
    const seam = (o: Record<string, unknown>) =>
      messages(manifest([fixture({ gates: ['seam', 'mix'], ...o })]));
    expect(seam({ windowBars: [0, 9], expectChunks: 2 })).toEqual([]);
    expect(seam({ windowBars: [0, 8], expectChunks: 2 })).toEqual([msg]);
    expect(seam({ windowBars: [0, 16] })).toEqual([msg]);
    expect(seam({ windowBars: [0, 16], expectChunks: 1 })).toEqual([msg]);
    expect(seam({ windowBars: [0, 16], expectChunks: 2.5 })).toEqual([msg]);
  });

  test('expectChunks without seam', () => {
    expect(messages(manifest([fixture({ expectChunks: 2 })]))).toEqual([
      'expectChunks needs the seam gate',
    ]);
  });

  test('stochastic and usesSamples must be booleans', () => {
    expect(messages(manifest([fixture({ stochastic: 'no' })]))).toEqual([
      'stochastic must be a boolean',
    ]);
    expect(messages(manifest([fixture({ usesSamples: 1 })]))).toEqual([
      'usesSamples must be a boolean',
    ]);
  });

  test('usesSamples must match the song', () => {
    const msg = 'usesSamples does not match the song';
    expect(messages(manifest([fixture({ usesSamples: true })]))).toEqual([msg]);
    const io = fakeIo({ 'a.js': SAMPLED });
    expect(messages(manifest([fixture()]), io)).toEqual([msg]);
    expect(messages(manifest([fixture({ usesSamples: true })]), io)).toEqual([]);
  });

  test('usesSamples needs a non-empty samples list', () => {
    const io = fakeIo({ 'a.js': SAMPLED }, []);
    expect(messages(manifest([fixture({ usesSamples: true })], { samples: [] }), io)).toEqual([
      'usesSamples needs a non-empty samples list',
    ]);
  });

  test('samples: array, present, unique', () => {
    expect(messages(manifest(undefined, { samples: 'kick.wav' }))).toEqual([
      'samples must be an array of file names',
    ]);
    expect(errorsOf(manifest(undefined, { samples: ['kick.wav', 'snare.wav'] }))).toEqual([
      { path: 'samples[1]', message: "sample 'snare.wav' not found" },
    ]);
    expect(messages(manifest(undefined, { samples: ['kick.wav', 'kick.wav'] }))).toEqual([
      "duplicate sample 'kick.wav'",
    ]);
  });

  test('rust is required and must be supported or unsupported', () => {
    const { rust: _, ...noRust } = fixture();
    expect(errorsOf(manifest([noRust]))).toEqual([
      { path: 'fixtures[0].rust', message: "rust must be 'supported' or 'unsupported'" },
    ]);
    for (const bad of ['yes', true, null, 'Supported']) {
      expect(messages(manifest([fixture({ rust: bad })]))).toEqual([
        "rust must be 'supported' or 'unsupported'",
      ]);
    }
    expect(messages(manifest([fixture({ rust: 'unsupported' })]))).toEqual([]);
  });

  test('tags, kind and notes', () => {
    expect(messages(manifest([fixture({ tags: [1] })]))).toEqual([
      'tags must be an array of strings',
    ]);
    expect(messages(manifest([fixture({ kind: 'other' })]))).toEqual([
      "kind must be 'external' or 'targeted'",
    ]);
    const { corpus, errors } = validateCorpus(manifest([fixture({ kind: 'external' })]), fakeIo());
    expect(errors).toEqual([]);
    expect(corpus?.fixtures[0]?.kind).toBe('external');
    expect(messages(manifest([fixture({ notes: 3 })]))).toEqual(['notes must be a string']);
  });

  test('expect: object, unverifiable within gates, clip boolean', () => {
    expect(messages(manifest([fixture({ expect: [] })]))).toEqual(['expect must be an object']);
    expect(messages(manifest([fixture({ expect: { unverifiable: 'stems' } })]))).toEqual([
      'expect.unverifiable must be an array',
    ]);
    expect(errorsOf(manifest([fixture({ expect: { unverifiable: ['stems'] } })]))).toEqual([
      {
        path: 'fixtures[0].expect.unverifiable[0]',
        message: "expected-unverifiable gate 'stems' is not in gates",
      },
    ]);
    expect(messages(manifest([fixture({ expect: { clip: 'yes' } })]))).toEqual([
      'expect.clip must be a boolean',
    ]);
    const ok = fixture({
      gates: ['stems', 'mix'],
      expect: { unverifiable: ['stems'], clip: true },
    });
    expect(messages(manifest([ok]))).toEqual([]);
  });

  test('every song file must be used', () => {
    const io = fakeIo({ 'a.js': SONG, 'b.js': SONG });
    expect(errorsOf(manifest(), io)).toEqual([
      { path: 'fixtures', message: "song 'b.js' is not in corpus.json" },
    ]);
  });

  test('collects every error at once', () => {
    const bad = manifest([fixture({ name: 'X', gates: ['nope'], windowBars: [3, 1] })], {
      schema: 3,
    });
    expect(messages(bad).length).toBe(4);
  });
});
