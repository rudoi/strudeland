import { eventPitch, identifyEvent, verifyEventOrigin } from '../lib/strudel/event-edits';
import { Pattern, State, TimeSpan } from '@strudel/core';
import { registerTranspilerPlugin } from '@strudel/transpiler';
import { parse, tokenizer, tokTypes, type AnyNode } from 'acorn';
import type {
  InspectionRequest,
  InspectionResult,
  RollNote,
  SourceSpan,
} from '../lib/piano-roll/types';
import { PlanJobError } from '../lib/render/job-plan';
import { queryMetered } from '../lib/strudel/metered-pattern';
import { applySeed } from '../lib/strudel/seed';
import type { SongStructure } from '../lib/strudel/structure';
import { layoutClock } from '../lib/strudel/timing';
import { LINEAR_SEGMENTS } from '../lib/strudel/transport';

export const MAX_INSPECTION_BARS = 16;
const MAX_BAR = 1_000_000;
// Keep the worst-case reply under the host's 1 MiB readout limit, including
// escaped sound names, capped fractions and source spans.
const MAX_NOTES = 1_024;
const MAX_CONTRIBUTORS = 8;
const MAX_HAPS_TO_SCAN = 16_384;
const EDGE = /^-?\d{1,24}(?:\/[1-9]\d{0,23})?$/;

export type InspectionWindow = Omit<InspectionRequest, 'code' | 'root'>;

export function inspectionWindow(request: InspectionWindow): boolean {
  const { view, from, to, anchorBar } = request;
  const bar = (n: number) => Number.isSafeInteger(n) && n >= 0 && n <= MAX_BAR;
  return (
    (view === 'song' || view === 'expression') &&
    bar(from) &&
    bar(to) &&
    from < to &&
    to - from <= MAX_INSPECTION_BARS &&
    (anchorBar === undefined || bar(anchorBar))
  );
}

interface CapturedPattern {
  query(state: unknown): RawHap[];
  seed(seed: number): CapturedPattern;
}

interface Capture {
  code: string;
  span: SourceSpan;
  name: string;
  type: AnyNode['type'];
  rewritten: boolean;
  patterns: Set<CapturedPattern>;
}

let activeCapture: Capture | undefined;

registerTranspilerPlugin({
  walk({ input }: { input: string }) {
    const capture = activeCapture;
    if (!capture || capture.code !== input) return;
    return {
      leave(this: { replace(node: object): unknown }, node: AnyNode) {
        if (
          node.type !== capture.type ||
          node.start !== capture.span.from ||
          node.end !== capture.span.to
        )
          return;
        capture.rewritten = true;
        return this.replace({
          type: 'CallExpression',
          callee: { type: 'Identifier', name: capture.name },
          arguments: [node],
          optional: false,
        });
      },
    };
  },
});

function valueSlot(node: AnyNode, parent: AnyNode, key: string): boolean {
  switch (parent.type) {
    case 'VariableDeclarator':
      return key === 'init';
    case 'ExpressionStatement':
      return key === 'expression' && !('directive' in parent);
    case 'ReturnStatement':
    case 'AwaitExpression':
      return key === 'argument';
    case 'CallExpression':
      return key === 'arguments';
    case 'MemberExpression':
      return key === 'object' && !parent.optional;
    case 'ArrayExpression':
      return key === 'elements';
    case 'ArrowFunctionExpression':
      return key === 'body' && node.type !== 'BlockStatement';
    case 'ConditionalExpression':
      return key === 'consequent' || key === 'alternate';
    case 'LogicalExpression':
    case 'SequenceExpression':
      return true;
    case 'Property':
      return key === 'value' && parent.kind === 'init' && !parent.shorthand && !parent.method;
    default:
      return false;
  }
}

const captureTypes = new Set([
  'Identifier',
  'CallExpression',
  'NewExpression',
  'TaggedTemplateExpression',
  'MemberExpression',
  'ChainExpression',
  'ConditionalExpression',
  'LogicalExpression',
  'SequenceExpression',
  'AwaitExpression',
]);

interface Visit {
  node: AnyNode;
  parent?: AnyNode;
  key?: string;
  chain?: boolean;
  pattern?: boolean;
}

function captureSite(code: string, span: SourceSpan): Pick<Capture, 'name' | 'type'> {
  const ast = parse(code, { ecmaVersion: 2022, allowAwaitOutsideFunction: true });
  const names = new Set<string>();
  const stack: Visit[] = [{ node: ast }];
  let type: AnyNode['type'] | undefined;
  let dynamicScope = false;
  for (let entry = stack.pop(); entry; entry = stack.pop()) {
    const { node, parent, key } = entry;
    const chain = entry.chain || node.type === 'ChainExpression';
    const pattern = entry.pattern || node.type === 'ObjectPattern' || node.type === 'ArrayPattern';
    if (node.type === 'Identifier') names.add(node.name);
    if (node.type === 'WithStatement') dynamicScope = true;
    if (
      node.start === span.from &&
      node.end === span.to &&
      parent &&
      key &&
      !entry.chain &&
      !pattern &&
      captureTypes.has(node.type) &&
      valueSlot(node, parent, key)
    ) {
      type = node.type;
    }
    for (const [childKey, children] of Object.entries(node)) {
      for (const child of Array.isArray(children) ? children : [children]) {
        if (child && typeof child.type === 'string') {
          stack.push({ node: child, parent: node, key: childKey, chain, pattern });
        }
      }
    }
  }
  if (!type || dynamicScope)
    throw new PlanJobError('plan', 'This expression cannot be observed in place.');
  let name = '__strudelandInspectPattern';
  while (names.has(name) || code.includes(name) || Object.hasOwn(globalThis, name)) name += '_';
  return { name, type };
}

/** Capture values during the original evaluation. Leave receivers, references and short-circuiting
 * alone, and keep the identity hook available until any query of the captured pattern is done. */
export async function inspectExpression(
  code: string,
  span: SourceSpan,
  run: (selected: () => CapturedPattern) => Promise<InspectionResult>,
): Promise<InspectionResult> {
  const { name, type } = captureSite(code, span);
  const capture: Capture = { code, span, name, type, rewritten: false, patterns: new Set() };
  activeCapture = capture;
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value(value: unknown) {
      if (value instanceof Pattern && capture.patterns.size < 2)
        capture.patterns.add(value as CapturedPattern);
      return value;
    },
  });
  try {
    return await run(() => {
      if (!capture.rewritten)
        throw new PlanJobError('plan', 'This expression changed shape during transpilation.');
      if (capture.patterns.size === 0) {
        throw new PlanJobError('plan', 'The expression produced no pattern during evaluation.');
      }
      if (capture.patterns.size > 1) {
        throw new PlanJobError('plan', 'The expression produced multiple pattern instances.');
      }
      return capture.patterns.values().next().value!;
    });
  } finally {
    activeCapture = undefined;
    Reflect.deleteProperty(globalThis, name);
  }
}

export function queryExpression(
  pattern: CapturedPattern,
  structure: SongStructure,
  cps: number,
  seed: number | undefined,
  request: InspectionWindow,
): RawHap[] {
  const { from, to, anchorBar } = request;
  const state = new State(new TimeSpan(from, to), {
    _cps: cps,
    cyclist: 'cyclist',
    _segments: LINEAR_SEGMENTS,
  });
  const seeded = applySeed(pattern, seed);
  if (!structure.meterChanges?.length) return seeded.query(state);

  const written = layoutClock(structure, LINEAR_SEGMENTS);
  const local = {
    ...written,
    meterAt: (cycle: number) =>
      anchorBar === undefined ? structure.meter : written.meterAt(anchorBar + cycle),
    nextMeterBoundary: (cycle: number) =>
      anchorBar === undefined ? Infinity : written.nextMeterBoundary(anchorBar + cycle) - anchorBar,
  };
  return queryMetered(seeded, state, structure, local, cps);
}

interface FractionEdge {
  toFraction(): string;
  valueOf(): number;
}

interface HapSpan {
  begin: FractionEdge;
  end: FractionEdge;
}

interface RawHap {
  whole?: HapSpan;
  part: HapSpan;
  value: unknown;
  context?: { locations?: { start: number; end: number }[] };
}

function pitchOf(
  value: unknown,
  allowPrimitive: boolean,
): { pitch: number; sound?: string } | null {
  const pitch = eventPitch(value, allowPrimitive);
  if (pitch === undefined) return null;
  const s = (value as { s?: unknown })?.s;
  return { pitch, ...(typeof s === 'string' && s.length <= 64 ? { sound: s } : {}) };
}

/** Cooked string escapes and normalized CRs can move mini offsets. Keep those
 * haps visible without offering uncertain links back to the raw source. */
function uncertainSpans(code: string): SourceSpan[] {
  if (!/[\\\r]/.test(code)) return [];
  const lex = tokenizer(code, { ecmaVersion: 2022 });
  const spans: SourceSpan[] = [];
  for (let token = lex.getToken(); token.type !== tokTypes.eof; token = lex.getToken()) {
    if (token.type === tokTypes.string || token.type === tokTypes.template) {
      if (/[\\\r]/.test(code.slice(token.start, token.end))) {
        spans.push({ from: token.start, to: token.end });
      }
    }
  }
  return spans;
}

function intersectsUncertain(spans: readonly SourceSpan[], from: number, to: number): boolean {
  let low = 0;
  let high = spans.length;
  while (low < high) {
    const mid = (low + high) >>> 1;
    if (spans[mid]!.to <= from) low = mid + 1;
    else high = mid;
  }
  return low < spans.length && spans[low]!.from < to;
}

function contributors(hap: RawHap, code: string, uncertain: readonly SourceSpan[]): SourceSpan[] {
  const output: SourceSpan[] = [];
  const locations = hap.context?.locations;
  if (!Array.isArray(locations)) return output;
  for (const location of locations.slice(0, MAX_CONTRIBUTORS)) {
    const { start, end } = location ?? {};
    if (
      !Number.isSafeInteger(start) ||
      !Number.isSafeInteger(end) ||
      start < 0 ||
      start >= end ||
      end > code.length ||
      intersectsUncertain(uncertain, start, end)
    ) {
      continue;
    }
    const raw = code.slice(start, end);
    const from = start + raw.length - raw.trimStart().length;
    const to = start + raw.trimEnd().length;
    if (from < to && !output.some((span) => span.from === from && span.to === to)) {
      output.push({ from, to });
    }
  }
  return output;
}

/** A sole top-level expression has no competing scheduled pattern in a fresh isolate.
 * An exact source match gives otherwise unlocated notes a link without wrapping their haps. */
function directOrigin(code: string, request: InspectionWindow): SourceSpan | undefined {
  const { expression, view } = request;
  if (!expression || view === 'expression') return expression;
  const { body } = parse(code, { ecmaVersion: 2022, allowAwaitOutsideFunction: true });
  if (body.length !== 1) return;
  const statement = body[0]?.type === 'LabeledStatement' ? body[0].body : body[0];
  if (
    statement?.type === 'ExpressionStatement' &&
    statement.expression.start === expression.from &&
    statement.expression.end === expression.to
  ) {
    return expression;
  }
}

/** Retain individual haps and visible fragments. A note's pitch and whole span are not an identity
 * for overlapping voices. The command applies a deadline before entering the isolate. */
export function inspectHaps(
  haps: RawHap[],
  code: string,
  request: InspectionWindow,
  query: (from: number, to: number) => RawHap[],
): InspectionResult {
  const uncertain = uncertainSpans(code);
  const fallbackOrigin = directOrigin(code, request);
  const notes: { note: RollNote; order: number }[] = [];
  const stochastic =
    /\b(?:rand|irand|perlin|choose|chooseCycles|wchoose|degrade|degradeBy|sometimes|random|slider|cF)\b/.test(
      code,
    );
  const groupKey = (h: RawHap) => {
    const arc = h.whole ?? h.part;
    return (h.whole ? 'w' : 'p') + arc.begin.toFraction() + ':' + arc.end.toFraction();
  };
  const groups = new Map<string, RawHap[]>();
  for (const h of haps.slice(0, MAX_HAPS_TO_SCAN)) {
    const key = groupKey(h),
      group = groups.get(key);
    if (group) group.push(h);
    else groups.set(key, [h]);
  }
  let bytes = 0;
  const probes = new Map<string, RawHap[]>();
  let truncated = haps.length > MAX_HAPS_TO_SCAN;
  for (let i = 0; i < Math.min(haps.length, MAX_HAPS_TO_SCAN); i++) {
    const hap = haps[i]!;
    const data = pitchOf(hap.value, request.view === 'expression');
    if (!data) continue;
    const { begin, end } = hap.whole ?? hap.part;
    const start = begin.toFraction();
    const finish = end.toFraction();
    const visibleStart = hap.part.begin.toFraction();
    const visibleEnd = hap.part.end.toFraction();
    if (![start, finish, visibleStart, visibleEnd].every((edge) => EDGE.test(edge))) {
      truncated = true;
      continue;
    }
    if (notes.length === MAX_NOTES) {
      truncated = true;
      break;
    }
    const locations = contributors(hap, code, uncertain);
    const origin =
      locations.length === 1 ? locations[0] : locations.length === 0 ? fallbackOrigin : undefined;
    let event = identifyEvent(
      hap,
      groups.get(groupKey(hap))!,
      request.view === 'expression',
      stochastic,
    );
    // A full onset observation already contains the tie group if its part starts
    // at the query boundary. Probe continuations, competing voices and random lanes.
    if (event && hap.whole && (stochastic || event.count || visibleStart !== start)) {
      let at = probes.get(start);
      if (!at) {
        at = query(Number(begin), Number(begin));
        probes.set(start, at);
      }
      event = verifyEventOrigin(hap, event, at);
    }
    // Native readout has a byte cap. Four bytes per code unit is a conservative UTF-8 bound.
    bytes += 384 + (event ? JSON.stringify(event).length * 4 : 0) + locations.length * 52;
    if (bytes > 980_000) {
      truncated = true;
      break;
    }
    notes.push({
      order: Number(hap.part.begin),
      note: {
        id: String(i),
        ...(event ? { event } : {}),
        start,
        end: finish,
        ...data,
        visibleStart,
        visibleEnd,
        ...(locations.length ? { contributors: locations } : {}),
        ...(origin ? { origin } : {}),
      },
    });
  }
  notes.sort((a, b) => a.order - b.order);
  return { notes: notes.map(({ note }) => note), truncated };
}
