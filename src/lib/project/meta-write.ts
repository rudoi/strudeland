/**
 * Minimal-diff edits of a song's `meta` header values, for the UI. Each edit is one CodeMirror
 * change spec, so the session can dispatch it as one undoable step. Refuses (null) rather than
 * guess whenever the header is not plainly editable.
 */
import type { Node, ObjectExpression, Property } from 'acorn';
import { headerRanges, LINE_BREAK, readSongHeader } from './song-header';

export type MetaPath = 'bpm' | 'meter' | 'loops' | `sections[${number}].bars`;

/** A CodeMirror change spec. */
export interface MetaEdit {
  from: number;
  to: number;
  insert: string;
}

const SECTION_BARS = /^sections\[(0|[1-9]\d*)\]\.bars$/;
/** A `meta` binding anywhere in the code; inserting a header next to one would redeclare it. */
const DECLARES_META = /\b(?:const|let|var)\s+meta\b/;

/** `value` as JS source: numbers via `String`, strings single-quoted and escaped. */
export function printValue(value: number | string): string {
  if (typeof value === 'number') return String(value);
  const escaped = value
    .replace(/\\/g, '\\\\')
    .replace(/'/g, "\\'")
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\u2028/g, '\\u2028')
    .replace(/\u2029/g, '\\u2029');
  return `'${escaped}'`;
}

function keyName(prop: Property): string | null {
  if (prop.computed) return null;
  if (prop.key.type === 'Identifier') return prop.key.name;
  if (prop.key.type === 'Literal' && typeof prop.key.value === 'string') return prop.key.value;
  return null;
}

/** The one property named `key`; undefined when absent, null when duplicated. */
function property(obj: ObjectExpression, key: string): Property | undefined | null {
  const found = obj.properties.filter(
    (p): p is Property => p.type === 'Property' && keyName(p) === key,
  );
  if (found.length > 1) return null;
  return found[0];
}

/** Leading whitespace of the line holding `pos`. */
function indentAt(code: string, pos: number): string {
  let start = pos;
  while (start > 0 && !LINE_BREAK.test(code[start - 1]!)) start--;
  return /^[ \t]*/.exec(code.slice(start))![0];
}

/** Insert `key: text` as the last property of `obj`, in the object's own layout. */
function insertProperty(code: string, obj: ObjectExpression, key: string, text: string): MetaEdit {
  const entry = `${key}: ${text}`;
  const last = obj.properties[obj.properties.length - 1] as Node | undefined;
  if (!last) {
    const inner = code.slice(obj.start + 1, obj.end - 1);
    if (/^\s*$/.test(inner)) return { from: obj.start + 1, to: obj.end - 1, insert: ` ${entry} ` };
    // Only comments inside: keep them after the new property.
    return { from: obj.start + 1, to: obj.start + 1, insert: ` ${entry},` };
  }
  // Multi-line when the closing brace is on a later line than the last property.
  if (!LINE_BREAK.test(code.slice(last.end, obj.end - 1))) {
    return { from: last.end, to: last.end, insert: `, ${entry}` };
  }
  // Copy the last property's indent; when it shares its line (e.g. with the `{`), indent one level
  // past the header's line.
  const indent = atLineStart(code, last.start)
    ? indentAt(code, last.start)
    : indentAt(code, obj.start) + '  ';
  return { from: last.end, to: last.end, insert: `,\n${indent}${entry}` };
}

/** True when only spaces or tabs come before `pos` on its line. */
function atLineStart(code: string, pos: number): boolean {
  let i = pos;
  while (i > 0 && (code[i - 1] === ' ' || code[i - 1] === '\t')) i--;
  return i === 0 || LINE_BREAK.test(code[i - 1]!);
}

function editFor(code: string, path: MetaPath, text: string): MetaEdit | null {
  const header = headerRanges(code);
  const bars = SECTION_BARS.exec(path);
  if (!header) {
    // A header that does not parse, or a `meta` declared elsewhere: never add a second one.
    if (bars || DECLARES_META.test(code)) return null;
    return { from: 0, to: 0, insert: `const meta = { ${path}: ${text} };\n\n` };
  }
  if (bars) {
    const sections = property(header, 'sections');
    if (!sections || sections.value.type !== 'ArrayExpression') return null;
    const section = sections.value.elements[Number(bars[1])];
    if (!section || section.type !== 'ObjectExpression') return null;
    const prop = property(section, 'bars');
    if (!prop) return null;
    return { from: prop.value.start, to: prop.value.end, insert: text };
  }
  const prop = property(header, path);
  if (prop === null) return null;
  if (prop) return { from: prop.value.start, to: prop.value.end, insert: text };
  return insertProperty(code, header, path, text);
}

/**
 * The change that sets `path` to `value` in the song's `meta` header: replaces an existing value,
 * inserts a missing `bpm`/`meter`/`loops` after the last property, or inserts a header when there
 * is none. Null (refused) when the header has a meta error or does not parse, a key on the path is
 * duplicated, the section does not exist, or the result would not read back as `value`.
 */
export function setMetaValue(
  code: string,
  path: MetaPath,
  value: number | string,
): MetaEdit | null {
  if (!readSongHeader(code).ok) return null;
  const edit = editFor(code, path, printValue(value));
  if (!edit) return null;
  // The result must read back cleanly (e.g. no NaN, no out-of-range loops without a loop section).
  const next = code.slice(0, edit.from) + edit.insert + code.slice(edit.to);
  const header = readSongHeader(next);
  if (!header.ok || readBack(header.meta, path) !== value) return null;
  return edit;
}

function readBack(meta: Record<string, unknown>, path: MetaPath): unknown {
  const bars = SECTION_BARS.exec(path);
  if (!bars) return meta[path];
  const sections = meta.sections;
  if (!Array.isArray(sections)) return undefined;
  return (sections[Number(bars[1])] as Record<string, unknown> | undefined)?.bars;
}
