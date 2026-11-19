// Extracts the function docs of the vendored Strudel (vendor/strudel/doc.json, jsdoc output) into a
// committed JSON file that the CLI embeds. Run with `bun run docs:extract` after a sync; the sync
// script also runs it.
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { jsonText } from './json-text';

export type DocParam = { name: string; types: string[]; description: string };
export type DocEntry = {
  name: string;
  synonyms: string[];
  description: string;
  params: DocParam[];
  examples: string[];
  superdirt: boolean;
};
export type DocsSource = { repo: string; commit: string };
export type DocsFile = {
  schema: 2;
  source: DocsSource;
  entries: DocEntry[];
};

const ENTITIES: Record<string, string> = {
  quot: '"',
  amp: '&',
  lt: '<',
  gt: '>',
  nbsp: ' ',
};

function decodeEntities(s: string): string {
  return s.replace(/&(?:#(\d+)|#x([0-9a-fA-F]+)|([a-zA-Z]+));/g, (m, dec, hex, name) => {
    if (dec !== undefined) return String.fromCodePoint(Number(dec));
    if (hex !== undefined) return String.fromCodePoint(parseInt(hex, 16));
    return ENTITIES[name] ?? m;
  });
}

/** JSDoc HTML to plain text: `<code>` becomes backticks, paragraphs and list items become lines. */
export function htmlToText(html: string): string {
  let s = html;
  s = s.replace(/<code[^>]*>([\s\S]*?)<\/code>/gi, '`$1`');
  s = s.replace(/<li[^>]*>/gi, '\n- ');
  s = s.replace(/<\/li>/gi, '\n');
  s = s.replace(/<\/p>/gi, '\n\n');
  s = s.replace(/<br\s*\/?>/gi, '\n');
  s = s.replace(/<\/(?:ul|ol|div|pre|h[1-6]|table|tr)>/gi, '\n');
  s = s.replace(/<(?:p|ul|ol|div|pre|h[1-6]|table|tr)(?:\s[^>]*)?>/gi, '\n');
  s = s.replace(/<[^>]*>/g, '');
  s = decodeEntities(s);
  s = s
    .split('\n')
    .map((l) => l.trimEnd())
    .join('\n');
  s = s.replace(/\n{3,}/g, '\n\n');
  return s.trim();
}

type Raw = {
  name?: string;
  memberof?: string;
  access?: string;
  kind?: string;
  synonyms?: string[];
  description?: string;
  params?: { name: string; type?: { names?: string[] }; description?: string }[];
  examples?: string[];
  tags?: { title?: string }[];
};

export function extractDocs(docJson: { docs: Raw[] }, source: DocsSource): DocsFile {
  if (!docJson || !Array.isArray(docJson.docs)) throw new Error('doc.json has no docs array');
  const seen = new Set<string>();
  const entries: DocEntry[] = [];
  for (const r of docJson.docs) {
    if (!r.name) continue;
    if (r.memberof !== undefined && r.memberof !== 'Pattern') continue;
    if (r.access === 'private') continue;
    if (r.kind === 'package') continue;
    if (seen.has(r.name)) continue;
    seen.add(r.name);
    entries.push({
      name: r.name,
      synonyms: r.synonyms ?? [],
      description: htmlToText(r.description ?? ''),
      params: (r.params ?? []).map((p) => ({
        name: p.name,
        types: p.type?.names ?? [],
        description: htmlToText(p.description ?? ''),
      })),
      examples: r.examples ?? [],
      superdirt: (r.tags ?? []).some((t) => t.title === 'superdirtonly'),
    });
  }
  // Stable sort in code-unit order; ties keep input order.
  entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return { schema: 2, source: { repo: source.repo, commit: source.commit }, entries };
}

export function renderDocs(file: DocsFile): string {
  return jsonText(file);
}

/** Repo-relative path of the committed docs file. */
export const DOCS_FILE = 'crates/strudeland-cli/strudel-docs.json';

/** The docs file for the vendored Strudel under `root`: `doc.json` plus `UPSTREAM.json`'s source. */
export function extractVendored(root: string): DocsFile {
  const vendor = join(root, 'vendor/strudel');
  const upstream = JSON.parse(readFileSync(join(vendor, 'UPSTREAM.json'), 'utf8')) as DocsSource;
  const docJson = JSON.parse(readFileSync(join(vendor, 'doc.json'), 'utf8'));
  return extractDocs(docJson, upstream);
}

/** Regenerates `DOCS_FILE` under `root`; returns the number of entries. */
export function writeVendoredDocs(root: string): number {
  const file = extractVendored(root);
  writeFileSync(join(root, DOCS_FILE), renderDocs(file));
  return file.entries.length;
}

if (import.meta.main) {
  const n = writeVendoredDocs(join(import.meta.dir, '..'));
  console.log(`wrote ${n} entries`);
}
