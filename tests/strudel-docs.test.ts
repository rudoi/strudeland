import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';
import {
  extractDocs,
  extractVendored,
  htmlToText,
  renderDocs,
  type DocsFile,
} from '../scripts/extract-strudel-docs';

const root = new URL('../', import.meta.url);
const read = (rel: string) => readFileSync(new URL(rel, root), 'utf8');
const upstream = JSON.parse(read('vendor/strudel/UPSTREAM.json')) as {
  repo: string;
  commit: string;
};
const committedText = read('crates/strudeland-cli/strudel-docs.json');
const committed = JSON.parse(committedText) as DocsFile;

test('the committed docs file matches a fresh extraction', () => {
  const fresh = renderDocs(extractDocs(JSON.parse(read('vendor/strudel/doc.json')), upstream));
  if (fresh !== committedText) {
    throw new Error(
      'crates/strudeland-cli/strudel-docs.json is out of date; run `bun run docs:extract` and commit it',
    );
  }
  expect(renderDocs(extractVendored(root.pathname))).toBe(committedText);
});

test('the docs file is schema 2 for the vendored commit', () => {
  expect(committed.schema).toBe(2);
  expect(committed.source).toEqual({ repo: upstream.repo, commit: upstream.commit });
  expect(Object.keys(committed)).toEqual(['schema', 'source', 'entries']);
});

test('the docs file has the expected entries and no junk', () => {
  const byName = new Map(committed.entries.map((e) => [e.name, e]));
  expect(byName.get('fast')?.synonyms).toContain('density');
  expect(byName.has('note')).toBe(true);
  // Only the vendored packages are documented: no MIDI or OSC output.
  expect(byName.has('midi')).toBe(false);
  expect(byName.has('osc')).toBe(false);
  expect(committed.entries.every((e) => !e.name.includes('#'))).toBe(true);
  expect(byName.size).toBe(committed.entries.length);
  expect(committedText).not.toContain('/Users/');
  expect(committedText).not.toContain('/private/');
  expect(committed.entries.every((e) => !/<\/?(p|code|br|li|ul)>/.test(e.description))).toBe(true);
});

test('htmlToText', () => {
  expect(htmlToText('use <code>fast(2)</code> now')).toBe('use `fast(2)` now');
  expect(htmlToText('<p>one</p><p>two</p>')).toBe('one\n\ntwo');
  expect(htmlToText('a &quot;b&quot; &amp; &lt;c&gt; &#39;d&#39; e&nbsp;f &#65;')).toBe(
    'a "b" & <c> \'d\' e f A',
  );
  expect(htmlToText('items<ul><li>a</li><li>b</li></ul>')).toBe('items\n\n- a\n\n- b');
  expect(htmlToText('x<br>y<br/>z')).toBe('x\ny\nz');
  expect(htmlToText('<p>a <em>very <strong>nested</strong></em> tag</p>')).toBe(
    'a very nested tag',
  );
  expect(htmlToText('a\n\n\n\n\nb')).toBe('a\n\nb');
  expect(htmlToText('  \n hi \n ')).toBe('hi');
  expect(htmlToText('')).toBe('');
});

const SOURCE = { repo: 'https://example.org/strudel', commit: 'a'.repeat(40) };

test('extractDocs filters, deduplicates and sorts', () => {
  const docs = [
    { name: 'zed', description: '<p>last</p>' },
    { name: 'beta', synonyms: ['b'], tags: [{ title: 'superdirtonly' }], examples: ['x'] },
    { name: 'alpha', description: 'first', params: [{ name: 'p', type: { names: ['number'] } }] },
    { name: 'alpha', description: 'duplicate' },
    { name: 'method', memberof: 'Pattern' },
    { name: 'voice#note', memberof: 'DoughVoice' },
    { name: 'hidden', access: 'private' },
    { name: 'pkg', kind: 'package' },
    { description: 'no name' },
    { name: 'esc', description: 'a `tick` and \\ slash "q"' },
  ];
  const out = extractDocs({ docs }, SOURCE);
  expect(out.schema).toBe(2);
  expect(out.source).toEqual(SOURCE);
  expect(out.entries.map((e) => e.name)).toEqual(['alpha', 'beta', 'esc', 'method', 'zed']);
  const alpha = out.entries[0];
  expect(alpha.description).toBe('first');
  expect(alpha.params).toEqual([{ name: 'p', types: ['number'], description: '' }]);
  expect(out.entries[1]).toMatchObject({ synonyms: ['b'], superdirt: true, examples: ['x'] });
  expect(out.entries[2].description).toBe('a `tick` and \\ slash "q"');
  expect(out.entries[4].description).toBe('last');
});

test('edge: an entry without a name, a private one and a duplicate name are skipped', () => {
  const out = extractDocs(
    {
      docs: [{ description: 'x' }, { name: 'a', access: 'private' }, { name: 'b' }, { name: 'b' }],
    },
    SOURCE,
  );
  expect(out.entries.map((e) => e.name)).toEqual(['b']);
  expect(extractDocs({ docs: [] }, SOURCE).entries).toEqual([]);
});

test('extractDocs copies only repo and commit into the source', () => {
  const out = extractDocs({ docs: [] }, { ...SOURCE, date: 'x', paths: [] } as any);
  expect(out.source).toEqual(SOURCE);
});

test('extractDocs rejects a doc.json without a docs array', () => {
  expect(() => extractDocs({} as any, SOURCE)).toThrow('no docs array');
  expect(() => extractDocs({ docs: {} } as any, SOURCE)).toThrow('no docs array');
  expect(() => extractDocs(null as any, SOURCE)).toThrow('no docs array');
});
