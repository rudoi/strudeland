// The committed headless host bundle is current: every input the bundler read still has the hash
// recorded in host.inputs.json (fast, no bundler). `HOST_BUILD_CHECK=1` also rebuilds the bundle
// and compares it byte for byte.
import { afterEach, describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  buildBundle,
  combinedSha256,
  forbiddenInputs,
  hashInputs,
  HOST_INPUTS,
  HOST_JS,
  hostInputs,
  inputsProblems,
  inputsText,
  REPO_ROOT,
} from '../scripts/host/build';
import { sha256 } from '../scripts/sha256';

const committed = readFileSync(join(REPO_ROOT, HOST_INPUTS), 'utf8');

test('host.inputs.json matches the files on disk (run `bun run host:build` if not)', () => {
  expect(inputsProblems(committed, REPO_ROOT)).toEqual([]);
});

test('the committed bundle defines the entry and lists its own sources', () => {
  const js = readFileSync(join(REPO_ROOT, HOST_JS), 'utf8');
  expect(js).toContain('strudelandHost');
  expect(js.endsWith('\n')).toBe(true);
  const files = Object.keys(JSON.parse(committed).files);
  expect(files).toContain('src/host/main.ts');
  expect(files).toContain('src/lib/render/job-plan.ts');
  expect(files.some((f) => f.startsWith('vendor/strudel/packages/core/'))).toBe(true);
  expect(forbiddenInputs(files)).toEqual([]);
});

describe('inputsProblems', () => {
  let dir: string | null = null;
  afterEach(() => {
    if (dir) rmSync(dir, { recursive: true, force: true });
    dir = null;
  });
  const fixture = () => {
    dir = mkdtempSync(join(tmpdir(), 'host-inputs-'));
    mkdirSync(join(dir, 'src/lib'), { recursive: true });
    writeFileSync(join(dir, 'a.ts'), 'a');
    writeFileSync(join(dir, 'src/lib/b.ts'), 'b');
    const inputs = hostInputs({ 'src/lib/b.ts': sha256('b'), 'a.ts': sha256('a') });
    return { root: dir, inputs, text: inputsText(inputs) };
  };

  test('a current file has no problems', () => {
    const { root, text } = fixture();
    expect(inputsProblems(text, root)).toEqual([]);
  });

  test('a changed file is named', () => {
    const { root, text } = fixture();
    writeFileSync(join(root, 'src/lib/b.ts'), 'b // x');
    expect(inputsProblems(text, root)).toEqual(['changed src/lib/b.ts']);
  });

  test('a missing file is named', () => {
    const { root, text } = fixture();
    rmSync(join(root, 'a.ts'));
    expect(inputsProblems(text, root)).toEqual(['missing a.ts']);
  });

  test('unsorted keys, a wrong combined and the text format are each reported', () => {
    const { root, inputs } = fixture();
    const files = { 'src/lib/b.ts': inputs.files['src/lib/b.ts']!, 'a.ts': inputs.files['a.ts']! };
    const unsorted = JSON.stringify({ files, combined: combinedSha256(files) }, null, 2) + '\n';
    expect(inputsProblems(unsorted, root)).toEqual([
      `${HOST_INPUTS} keys are not sorted by byte order`,
    ]);
    const badCombined = inputsText({ ...inputs, combined: sha256('') });
    expect(inputsProblems(badCombined, root)).toEqual([
      `${HOST_INPUTS} combined is not the hash of its files`,
    ]);
    expect(inputsProblems(inputsText(inputs).trimEnd(), root)).toEqual([
      `${HOST_INPUTS} is not two-space JSON with a trailing newline`,
    ]);
    expect(inputsProblems(JSON.stringify(inputs), root)).toEqual([
      `${HOST_INPUTS} is not two-space JSON with a trailing newline`,
    ]);
  });

  test('malformed files', () => {
    const { root } = fixture();
    expect(inputsProblems('{', root)[0]).toStartWith(`${HOST_INPUTS} is not JSON:`);
    const shape = `${HOST_INPUTS} is not { files: { path: sha256 }, combined: sha256 }`;
    for (const text of [
      'null',
      '[]',
      '{}',
      '{"files":[],"combined":""}',
      '{"files":{"a":1},"combined":""}',
    ]) {
      expect({ text, problems: inputsProblems(text, root) }).toEqual({ text, problems: [shape] });
    }
    const empty = inputsText(hostInputs({}));
    expect(inputsProblems(empty, root)).toEqual([`${HOST_INPUTS} lists no files`]);
  });
});

describe('build helpers', () => {
  test('hostInputs sorts keys by byte order and hashes path/sha lines', () => {
    const inputs = hostInputs({ b: '2', B: '1', a: '3' });
    expect(Object.keys(inputs.files)).toEqual(['B', 'a', 'b']);
    expect(inputs.combined).toBe(sha256('B\n1\na\n3\nb\n2\n'));
    expect(combinedSha256({})).toBe(sha256(''));
  });

  test('forbiddenInputs names every forbidden prefix and nothing else', () => {
    const bad = [
      'node_modules/superdough/index.mjs',
      'vendor/strudel/packages/superdough/superdough.mjs',
      'vendor/strudel/packages/webaudio/index.mjs',
      'vendor/strudel/packages/codemirror/widget.mjs',
      'node_modules/@codemirror/view/dist/index.js',
      'node_modules/svelte/src/store/index-client.js',
      'src/lib/audio/offline.ts',
    ];
    const good = [
      'vendor/strudel/packages/core/index.mjs',
      'src/lib/audiofile.ts',
      'node_modules/svelte-check/x.js',
    ];
    expect(forbiddenInputs([...good, ...bad])).toEqual(bad);
  });

  test('hashInputs resolves symlinks to repo-relative real paths and skips non-files', () => {
    const files = hashInputs(['node_modules/@strudel/core/index.mjs', 'not/a/file.ts'], REPO_ROOT);
    const path = 'vendor/strudel/packages/core/index.mjs';
    expect(files).toEqual({ [path]: sha256(readFileSync(join(REPO_ROOT, path))) });
  });
});

test.skipIf(process.env.HOST_BUILD_CHECK !== '1')(
  'HOST_BUILD_CHECK=1: a fresh build equals the committed bundle and inputs',
  async () => {
    const built = await buildBundle(REPO_ROOT);
    expect(inputsText(built.inputs)).toBe(committed);
    expect(built.js === readFileSync(join(REPO_ROOT, HOST_JS), 'utf8')).toBe(true);
  },
  120_000,
);
