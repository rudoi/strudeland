import { parse, parseExpressionAt, type AnyNode, type Comment } from 'acorn';
import { validateEventSpec, type EventEditSpec } from './event-edits';

const options = { ecmaVersion: 2022, allowAwaitOutsideFunction: true } as const;
export const EDIT_COMMENT = '@piano-edits';

export function songEditComment(code: string) {
  if (!code.includes(EDIT_COMMENT)) return;
  const comments: Comment[] = [];
  const tree = parse(code, { ...options, onComment: comments });
  const directives = comments.filter(
    (c) =>
      c.type === 'Block' &&
      c.value.trimStart().startsWith(EDIT_COMMENT) &&
      !tree.body.some((node) => node.start <= c.start && node.end >= c.end),
  );
  if (!directives.length) return;
  const comment = directives[0];
  if (directives.length > 1 || code.slice(comment.end).trim())
    throw new Error('piano edits: place one event declaration at the end of the song');
  const start = code.indexOf(EDIT_COMMENT, comment.start) + EDIT_COMMENT.length;
  const end = comment.end - 2;
  const spec: unknown = JSON.parse(code.slice(start, end));
  validateEventSpec(spec);
  const node = parseExpressionAt(code.slice(0, end), start, options);
  return { spec, node: node as AnyNode, start, end };
}

export type { EventEditSpec };
