import { describe, expect, test } from 'bun:test';
import { parsePatches, patchPathProblem } from '../scripts/strudel-patches';

const entry = (
  id: string,
  { upstream = 'Upstream: candidate', files = ['packages/core/repl.mjs'], body = 'Why.' } = {},
) => `## ${id}: Title of ${id}\n${upstream}\n${body}\n\`\`\`files\n${files.join('\n')}\n\`\`\`\n`;

const fileWith = (...entries: string[]) =>
  `# Local patches\n\nIntro text.\n\n${entries.join('\n')}`;

describe('parsePatches: happy', () => {
  test('two entries, one with two files', () => {
    const text = fileWith(
      entry('hap-tap'),
      entry('audio-rng', {
        files: ['packages/superdough/rng.mjs', 'packages/superdough/noise.mjs'],
      }),
    );
    expect(parsePatches(text)).toEqual([
      {
        id: 'hap-tap',
        title: 'Title of hap-tap',
        upstream: 'candidate',
        files: ['packages/core/repl.mjs'],
      },
      {
        id: 'audio-rng',
        title: 'Title of audio-rng',
        upstream: 'candidate',
        files: ['packages/superdough/rng.mjs', 'packages/superdough/noise.mjs'],
      },
    ]);
  });

  test('all three Upstream forms', () => {
    const text = fileWith(
      entry('a', { upstream: 'Upstream: candidate' }),
      entry('b', { upstream: 'Upstream: strudeland-only' }),
      entry('c', { upstream: 'Upstream: submitted https://codeberg.org/uzu/strudel/pulls/1' }),
    );
    expect(parsePatches(text).map((e) => e.upstream)).toEqual([
      'candidate',
      'strudeland-only',
      'submitted https://codeberg.org/uzu/strudel/pulls/1',
    ]);
  });

  test('text before the first heading is ignored', () => {
    const text = '# Title\n\nSome intro with ```files\nnot/a/patch\n``` and `## ` mentions.\n';
    expect(parsePatches(text)).toEqual([]);
    expect(parsePatches('')).toEqual([]);
  });

  test('a multi-line description and a non-files fenced block are allowed', () => {
    const text = fileWith(
      entry('a', { body: 'Line one.\nLine two.\n```js\n## not a heading\n```\nLine three.' }),
    );
    const [e] = parsePatches(text);
    expect(e.id).toBe('a');
    expect(e.files).toEqual(['packages/core/repl.mjs']);
  });

  test('CRLF line endings parse', () => {
    expect(parsePatches(fileWith(entry('a')).replace(/\n/g, '\r\n'))[0].files).toEqual([
      'packages/core/repl.mjs',
    ]);
  });

  test('edge: the same path in two entries is allowed', () => {
    const text = fileWith(entry('a'), entry('b'));
    expect(parsePatches(text).map((e) => e.files)).toEqual([
      ['packages/core/repl.mjs'],
      ['packages/core/repl.mjs'],
    ]);
  });
});

describe('parsePatches: errors', () => {
  const throws = (text: string, message: string) =>
    expect(() => parsePatches(text)).toThrow(message);

  test('duplicate id', () => throws(fileWith(entry('a'), entry('a')), 'entry "a": duplicate id'));

  test('bad id', () => {
    throws(fileWith(entry('Hap')), 'entry "Hap": id must match');
    throws(fileWith(entry('-a')), 'id must match');
    throws(fileWith(entry('a_b')), 'id must match');
  });

  test('heading without an id and title', () =>
    throws('## just a heading\nUpstream: candidate\n', 'heading is not'));

  test('empty title', () =>
    throws('## a: \nUpstream: candidate\n```files\nx\n```\n', 'entry "a": empty title'));

  test('missing Upstream line', () => {
    throws(
      fileWith(entry('a', { upstream: 'Some text' })),
      'entry "a": the line after the heading',
    );
    throws('## a: t', 'entry "a": the line after the heading');
  });

  test('unknown Upstream value', () => {
    throws(fileWith(entry('a', { upstream: 'Upstream: maybe' })), 'entry "a": unknown Upstream');
    throws(fileWith(entry('a', { upstream: 'Upstream: submitted' })), 'unknown Upstream');
    throws(fileWith(entry('a', { upstream: 'Upstream: candidate ' })), 'unknown Upstream');
  });

  test('missing files block', () =>
    throws('## a: t\nUpstream: candidate\nWhy.\n', 'entry "a": no files block'));

  test('two files blocks', () =>
    throws(
      fileWith(entry('a', { body: 'Why.\n```files\npackages/core/x.mjs\n```' })),
      'entry "a": more than one files block',
    ));

  test('empty files block', () =>
    throws('## a: t\nUpstream: candidate\n```files\n```\n', 'the files block is empty'));

  test('unclosed fence', () =>
    throws('## a: t\nUpstream: candidate\n```files\npackages/core/x.mjs\n', 'unclosed'));

  test('blank line inside the block', () =>
    throws(
      fileWith(entry('a', { files: ['packages/a.mjs', '', 'packages/b.mjs'] })),
      'blank line',
    ));

  test('comment inside the block', () => {
    throws(fileWith(entry('a', { files: ['# note', 'packages/a.mjs'] })), 'comment');
    throws(fileWith(entry('a', { files: ['// note'] })), 'comment');
  });

  test('bad paths', () => {
    throws(fileWith(entry('a', { files: ['packages/../x.mjs'] })), 'a .. segment');
    throws(fileWith(entry('a', { files: ['/packages/x.mjs'] })), 'leading /');
    throws(fileWith(entry('a', { files: ['./packages/x.mjs'] })), 'leading ./');
    throws(fileWith(entry('a', { files: ['packages\\x.mjs'] })), 'backslash');
    throws(fileWith(entry('a', { files: [' packages/x.mjs'] })), 'whitespace');
  });

  test('the same path twice in one entry', () =>
    throws(
      fileWith(entry('a', { files: ['packages/x.mjs', 'packages/x.mjs'] })),
      'entry "a": path listed twice: packages/x.mjs',
    ));
});

describe('patchPathProblem', () => {
  test('accepts normal paths', () => {
    for (const p of ['LICENSE', 'packages/core/repl.mjs', 'packages/a.b/c..d.mjs'])
      expect(patchPathProblem(p)).toBeNull();
  });
  test('rejects unnormalized paths', () => {
    for (const p of ['packages//x', 'packages/./x', 'packages/x/', '..'])
      expect(patchPathProblem(p)).not.toBeNull();
  });
});
