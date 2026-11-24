import * as core from '@strudel/core';
import { transpiler } from '@strudel/transpiler';
import controlNames from '../../../crates/strudeland-project/instrument-controls.json';
import type { StrudelPattern } from './types';

type Scalar = string | number | boolean;

interface InstrumentControl {
  control: string;
  value: Scalar | { readonly param: string };
}

interface InstrumentDefinition {
  readonly schema: 1;
  readonly parameters: Readonly<Record<string, Scalar>>;
  readonly voices: readonly {
    readonly name: string;
    readonly chain: readonly InstrumentControl[];
  }[];
}

/** Validated by the project crate. The browser render input can also supply this JSON shape. */
export type InstrumentSnapshot = Readonly<Record<string, InstrumentDefinition>>;

type Transform = (pattern: StrudelPattern) => StrudelPattern;
type Setter = (value: StrudelPattern, pattern: StrudelPattern) => StrudelPattern;
type Namespace = Readonly<Record<string, (options?: unknown) => Transform>>;

const IDENTIFIER = /^[a-z][a-z0-9_]{0,63}$/;
const WINDOWS_DEVICE = /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])$/;
const OPERATION = 'strudelandApplyInstrument';
const BINDING = 'strudeland.instrument-namespace';
const namespaceKey = Symbol.for(BINDING);
const bindings = globalThis as typeof globalThis & { [namespaceKey]?: () => Namespace };
const setters = new Map<string, Setter>(
  controlNames.map((name) => {
    const setter = core[name];
    if (typeof setter !== 'function') throw new Error(`Unknown core instrument control '${name}'`);
    return [name, setter];
  }),
);
let applyInstrument:
  ((transform: Transform, pattern: StrudelPattern) => StrudelPattern) | undefined;

/** Install one operation for all instrument definitions. The transform is an argument to it. */
export function registerInstrumentOperation(): void {
  if (applyInstrument) return;
  if (
    OPERATION in core.Pattern.prototype ||
    `_${OPERATION}` in core.Pattern.prototype ||
    OPERATION in core.strudelScope
  ) {
    throw new Error(`Instrument operation '${OPERATION}' is already registered`);
  }
  core.register(OPERATION, (transform: Transform, pattern: StrudelPattern) => transform(pattern));
  // A compiled transform is already scalar and has no source location to merge.
  const applyRaw = core.Pattern.prototype[`_${OPERATION}`] as (
    this: StrudelPattern,
    transform: Transform,
  ) => StrudelPattern;
  applyInstrument = (transform, pattern) => applyRaw.call(core.reify(pattern), transform);
}

function scalar(value: unknown): value is Scalar {
  return (
    typeof value === 'string' ||
    typeof value === 'boolean' ||
    (typeof value === 'number' && Number.isFinite(value))
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  if (typeof value !== 'object' || value === null) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === null || proto === Object.prototype;
}

function compileInstrument(id: string, definition: unknown): Namespace[string] {
  const fail = (message: string): never => {
    throw new Error(`Instrument '${id}': ${message}`);
  };
  if (!isRecord(definition) || definition.schema !== 1) return fail('expected schema 1');
  if (Object.keys(definition).length !== 3) return fail('expected schema, parameters and voices');
  const { parameters, voices } = definition;
  if (!isRecord(parameters)) return fail('parameters must be an object');
  const defaults = new Map<string, Scalar>();
  for (const [name, value] of Object.entries(parameters)) {
    if (!IDENTIFIER.test(name) || !scalar(value)) return fail(`invalid parameter '${name}'`);
    defaults.set(name, value);
  }
  if (!Array.isArray(voices) || voices.length === 0) return fail('expected at least one voice');
  const names = new Set<string>();
  const kit = voices.length > 1;
  const compiledVoices = voices.map((voice: unknown) => {
    if (!isRecord(voice) || typeof voice.name !== 'string' || !IDENTIFIER.test(voice.name)) {
      return fail('invalid voice name');
    }
    const { name, chain } = voice;
    if (Object.keys(voice).length !== 2) return fail(`voice '${name}' must contain name and chain`);
    if (names.has(name)) return fail(`duplicate voice '${name}'`);
    names.add(name);
    if (!Array.isArray(chain) || chain.length === 0) return fail(`voice '${name}' has no controls`);
    const controls = chain.map((step: unknown) => {
      if (!isRecord(step) || Object.keys(step).length !== 2 || typeof step.control !== 'string') {
        return fail(`invalid control in voice '${name}'`);
      }
      const { control, value } = step;
      const set = setters.get(control);
      if (!set) return fail(`unknown control '${control}' in voice '${name}'`);
      if (scalar(value)) return { set, resolve: () => value };
      if (isRecord(value) && Object.keys(value).length === 1 && typeof value.param === 'string') {
        const { param } = value;
        if (!defaults.has(param)) return fail(`unknown parameter '${param}' in voice '${name}'`);
        return { set, resolve: (options: ReadonlyMap<string, Scalar>) => options.get(param)! };
      }
      return fail(`invalid value for '${control}' in voice '${name}'`);
    });
    return { name, controls };
  });

  return (options = {}) => {
    if (!isRecord(options)) return fail('options must be an object');
    const parameters = new Map(defaults);
    for (const [name, value] of Object.entries(options)) {
      const fallback = defaults.get(name);
      if (fallback === undefined) return fail(`unknown option '${name}'`);
      if (!scalar(value) || typeof value !== typeof fallback) {
        return fail(`option '${name}' must be a ${typeof fallback}`);
      }
      parameters.set(name, value);
    }

    const voices = compiledVoices.map(({ name, controls }) => ({
      label: `${id}/${name}`,
      controls: controls.map(({ set, resolve }) => ({
        set,
        value: core.pure(resolve(parameters)),
      })),
    }));
    const transform: Transform = (input) => {
      const patterns = voices.map(({ label, controls }) => {
        const voice = controls.reduce((pattern, { set, value }) => set(value, pattern), input);
        return kit
          ? voice.fmap((value: Record<string, unknown>) =>
              value.label == null ? { ...value, label } : value,
            )
          : voice;
      });
      return kit ? core.stack(...patterns) : patterns[0];
    };
    return (pattern) => applyInstrument!(transform, pattern);
  };
}

function compileNamespace(snapshot: unknown): Namespace {
  if (!isRecord(snapshot)) throw new Error('Instrument snapshot must be an object');
  const namespace: Record<string, Namespace[string]> = Object.create(null);
  for (const [id, definition] of Object.entries(snapshot)) {
    if (!IDENTIFIER.test(id) || WINDOWS_DEVICE.test(id))
      throw new Error(`Invalid instrument '${id}'`);
    namespace[id] = compileInstrument(id, definition);
  }
  return Object.freeze(namespace);
}

/** Wrap already-transpiled code so its source locations are still those in the song. */
export function createInstrumentTranspiler() {
  let takeNamespace: (() => Namespace) | undefined;
  const clear = () => {
    if (takeNamespace && bindings[namespaceKey] === takeNamespace) delete bindings[namespaceKey];
    takeNamespace = undefined;
  };
  return {
    clear,
    transpile(code: string, options: object, snapshot?: InstrumentSnapshot) {
      const result = transpiler(code, options);
      let binding = 'Object.freeze(Object.create(null))';
      if (snapshot !== undefined) {
        const namespace = compileNamespace(snapshot);
        if (Object.keys(namespace).length > 0) {
          if (!applyInstrument) throw new Error('Instrument operation is unavailable');
          if (Object.hasOwn(bindings, namespaceKey))
            throw new Error('Instrument scope is occupied');
          takeNamespace = () => {
            clear();
            return namespace;
          };
          bindings[namespaceKey] = takeNamespace;
          binding = `globalThis[Symbol.for(${JSON.stringify(BINDING)})]()`;
        }
      }
      // The inner block permits a song-local declaration named `instruments`.
      return {
        ...result,
        output: `return await (async (instruments) => { { ${result.output}\n} })(${binding});`,
      };
    },
  };
}
