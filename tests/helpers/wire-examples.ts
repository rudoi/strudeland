// The control protocol's wire examples (`crates/strudeland-ctl/tests/wire/`), read for the
// frontend tests. Nothing under `src/` imports this: the app bundle never reads the crate.

import { readdirSync, readFileSync } from 'node:fs';
import type { MethodName } from '../../src/lib/control/protocol';

const DIR = new URL('../../crates/strudeland-ctl/tests/wire/', import.meta.url);

/** A named params or result value. */
export interface WireEntry {
  name: string;
  value: unknown;
}

/** Params a method refuses, with the exact error. */
export interface WireInvalid {
  name: string;
  params: unknown;
  code: number;
  message: string;
}

/** One `<method>.json`. */
export interface WireExample {
  method: MethodName;
  answeredBy: 'app' | 'webview';
  params: WireEntry[];
  results: WireEntry[];
  /** An older app's replies, with fields missing. */
  olderResults?: WireEntry[];
  /** Refuses whoever answers the method (`answeredBy`). */
  invalid: WireInvalid[];
  /** A webview method's refusals that the app answers before forwarding. */
  invalidApp?: WireInvalid[];
}

const read = (file: string): unknown => JSON.parse(readFileSync(new URL(file, DIR), 'utf8'));

const SHARED = new Set(['errors.json', 'limits.json']);

const examples: WireExample[] = readdirSync(DIR)
  .filter((f) => f.endsWith('.json') && !SHARED.has(f))
  .sort()
  .map((f) => read(f) as WireExample);

/** Every `<method>.json`, read once, in sorted file order. */
export function wireExamples(): WireExample[] {
  return examples;
}

function wireExample(method: MethodName): WireExample {
  const hit = examples.find((e) => e.method === method);
  if (!hit) throw new Error(`no wire example for ${method}`);
  return hit;
}

/**
 * A named result's value, from `results` or `olderResults` (as Rust's `wire_fixtures::result`);
 * throws naming the method and name when it is missing.
 */
export function wireResult(method: MethodName, name: string): any {
  const ex = wireExample(method);
  const hit = [...ex.results, ...(ex.olderResults ?? [])].find((r) => r.name === name);
  if (!hit) throw new Error(`${method} has no wire result '${name}'`);
  return hit.value;
}

export const wireErrors = read('errors.json') as Record<string, number>;

export const wireLimits = read('limits.json') as {
  eventsLimitDefault: number;
  eventsLimitMax: number;
};
