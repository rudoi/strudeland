// The control protocol's wire examples (`crates/strudeland-ctl/tests/wire/`), read for the
// frontend tests. Nothing under `src/` imports this: the app bundle never reads the crate.

import { readdirSync, readFileSync } from 'node:fs';
import type { MethodName } from '../../src/lib/control/protocol';

const DIR = new URL('../../crates/strudeland-ctl/tests/wire/', import.meta.url);

/** A named params or result value. */
export interface WireEntry {
  name: string;
  value: unknown;
  /** An older app's reply, with fields missing. */
  older?: boolean;
}

/** Params a method refuses, with the exact error. */
export interface WireInvalid {
  name: string;
  params: unknown;
  code: number;
  message: string;
  /** Overrides the method's `answeredBy` for this entry. */
  answeredBy?: 'app' | 'webview';
}

/** One `<method>.json`. */
export interface WireExample {
  method: MethodName;
  answeredBy: 'app' | 'webview';
  params: WireEntry[];
  results: WireEntry[];
  invalid: WireInvalid[];
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

export function wireExample(method: MethodName): WireExample {
  const hit = examples.find((e) => e.method === method);
  if (!hit) throw new Error(`no wire example for ${method}`);
  return hit;
}

/** A named result's value; throws naming the method and name when it is missing. */
export function wireResult(method: MethodName, name: string): any {
  const hit = wireExample(method).results.find((r) => r.name === name);
  if (!hit) throw new Error(`${method} has no wire result '${name}'`);
  return hit.value;
}

export const wireErrors = read('errors.json') as Record<string, number>;

export const wireLimits = read('limits.json') as {
  eventsLimitDefault: number;
  eventsLimitMax: number;
};
