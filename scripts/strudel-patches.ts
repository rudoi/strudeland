// Parser for vendor/strudel/PATCHES.md, the log of local patches to the vendored Strudel. Used by the
// sync script and the vendor integrity test.

export type PatchEntry = {
  id: string;
  title: string;
  /** `candidate`, `strudeland-only` or `submitted <url>`. */
  upstream: string;
  /** Paths relative to `vendor/strudel/`, in file order. */
  files: string[];
};

const ID = /^[a-z0-9][a-z0-9-]*$/;
const UPSTREAM = /^Upstream: (candidate|strudeland-only|submitted https?:\/\/\S+)$/;
const FENCE = /^(`{3,}|~{3,})(.*)$/;

/** Why `path` is not an acceptable `files` entry, or null when it is. */
export function patchPathProblem(path: string): string | null {
  if (path.trim() === '') return 'blank line in the files block';
  if (path.startsWith('#') || path.startsWith('//')) return `comment in the files block: ${path}`;
  if (path !== path.trim()) return `path has surrounding whitespace: "${path}"`;
  if (path.includes('\\')) return `path has a backslash: ${path}`;
  if (path.startsWith('/')) return `path has a leading /: ${path}`;
  if (path.startsWith('./')) return `path has a leading ./: ${path}`;
  const segments = path.split('/');
  if (segments.includes('..')) return `path has a .. segment: ${path}`;
  if (segments.some((s) => s === '' || s === '.')) return `path is not normalized: ${path}`;
  return null;
}

type Section = { heading: string; line: number; body: string[] };

function sections(text: string): Section[] {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const out: Section[] = [];
  let fence: string | null = null;
  let current: Section | null = null;
  lines.forEach((line, i) => {
    if (fence === null && line.startsWith('## ')) {
      current = { heading: line, line: i + 1, body: [] };
      out.push(current);
      return;
    }
    const m = FENCE.exec(line);
    if (m) {
      if (fence === null) fence = m[1];
      else if (m[1][0] === fence[0] && m[1].length >= fence.length && m[2].trim() === '')
        fence = null;
    }
    current?.body.push(line);
  });
  return out;
}

/** Parses PATCHES.md. Text before the first `## ` heading is ignored. Throws on a malformed entry. */
export function parsePatches(text: string): PatchEntry[] {
  const entries: PatchEntry[] = [];
  const seen = new Set<string>();
  for (const s of sections(text)) {
    const heading = /^## ([^:]*): (.*)$/.exec(s.heading);
    if (!heading) throw new Error(`PATCHES.md line ${s.line}: heading is not "## <id>: <title>"`);
    const id = heading[1];
    const title = heading[2].trim();
    const fail: (rule: string) => never = (rule) => {
      throw new Error(`PATCHES.md entry "${id}": ${rule}`);
    };
    if (!ID.test(id)) fail('id must match [a-z0-9][a-z0-9-]*');
    if (seen.has(id)) fail('duplicate id');
    seen.add(id);
    if (title === '') fail('empty title');

    const upstreamLine = s.body[0];
    if (upstreamLine === undefined || !upstreamLine.startsWith('Upstream:'))
      fail('the line after the heading must be "Upstream: …"');
    const upstream = UPSTREAM.exec(upstreamLine);
    if (!upstream)
      fail(
        `unknown Upstream line "${upstreamLine}" (candidate, strudeland-only or submitted <url>)`,
      );

    const blocks: string[][] = [];
    let block: string[] | null = null;
    let fence: string | null = null;
    for (const line of s.body.slice(1)) {
      const m = FENCE.exec(line);
      if (fence === null) {
        if (!m) continue;
        fence = m[1];
        if (m[2].trim() === 'files') {
          block = [];
          blocks.push(block);
        }
        continue;
      }
      if (m && m[1][0] === fence[0] && m[1].length >= fence.length && m[2].trim() === '') {
        fence = null;
        block = null;
        continue;
      }
      block?.push(line);
    }
    if (fence !== null) fail('unclosed fenced block');
    if (blocks.length === 0) fail('no files block');
    if (blocks.length > 1) fail('more than one files block');
    const files = blocks[0];
    if (files.length === 0) fail('the files block is empty');
    const inEntry = new Set<string>();
    for (const path of files) {
      const problem = patchPathProblem(path);
      if (problem) fail(problem);
      if (inEntry.has(path)) fail(`path listed twice: ${path}`);
      inEntry.add(path);
    }
    entries.push({ id, title, upstream: upstream[1], files });
  }
  return entries;
}
