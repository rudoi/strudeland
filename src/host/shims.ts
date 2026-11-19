/**
 * The headless host's replacements for the editor's and the audio output's value-affecting names:
 * each has the real function's effect on the pattern and hap values, nothing else (no DOM, no
 * canvas, no audio). The table of `0012` "Shims" is exhaustive; `tests/host-scope.test.ts` fails on
 * any app-scope name that is neither real, shimmed, stubbed nor in `NOT_REACHABLE`, and
 * `tests/host-shims.test.ts` compares each shim's haps with the real function's.
 *
 * Importing this module installs the `Pattern.prototype` methods and registers the widget types
 * (once per fresh host context). `shims` is the object the host prebake passes to `evalScope`.
 */

import { Pattern, pure, ref, registerControl } from '@strudel/core';
import { registerWidgetType } from '@strudel/transpiler';

type PatternMethod = (this: any, ...args: any[]) => any;

/** The widget methods, in `@strudel/codemirror`'s `widget.mjs` registration order: the
 *  transpiler numbers widget ids (`_widget__scope_0`, ...) by it. */
export const WIDGET_TYPES = [
  '_pianoroll',
  '_punchcard',
  '_spiral',
  '_scope',
  '_pitchwheel',
  '_spectrum',
] as const;

// The real scope methods return `this.analyze(id).draw(...)`; `draw` returns `this` without a
// `window`.
function analyzeById(this: any, config: { id?: unknown } = {}): any {
  return this.analyze(config.id ?? 1);
}

/** The `Pattern.prototype` methods the host installs, by name. */
export const PATTERN_SHIMS: Readonly<Record<string, PatternMethod>> = Object.freeze({
  // @strudel/webaudio scope.mjs and spectrum.mjs (`scope` is the same function as `tscope`).
  scope: analyzeById,
  tscope: analyzeById,
  fscope: analyzeById,
  spectrum: analyzeById,
  // @strudel/webaudio webaudio.mjs: not `this`, because `onSchedule` sets `dominantSchedule`,
  // which decides whether the hap goes to the default output (`routesToDefault`).
  rawdsp: function (this: any) {
    return this.onSchedule(() => {}, 1);
  },
  // @strudel/codemirror widget.mjs (the real ones draw on a canvas widget).
  _pianoroll: function (this: any, id: any, _options: any = { fold: 1 }) {
    return this.tag(id);
  },
  _punchcard: function (this: any, id: any, _options: any = { fold: 1 }) {
    return this.tag(id);
  },
  _spiral: function (this: any, id: any, _options: any = { fold: 1 }) {
    return this.tag(id);
  },
  _scope: function (this: any, id: any, _options: any = { fold: 1 }) {
    return this.tag(id).analyze(id);
  },
  _pitchwheel: function (this: any, id: any, _options: any = { fold: 1 }) {
    return this.tag(id);
  },
  _spectrum: function (this: any, id: any, _options: any = { fold: 1 }) {
    return this.analyze(id);
  },
  // @strudel/draw: replaced, because their default `ctx = getDrawContext()` reads `window`.
  pianoroll: function (this: any) {
    return this;
  },
  pitchwheel: function (this: any, options: { id?: unknown } = {}) {
    return this.tag(options.id ?? 1);
  },
});

/** Installs the `PATTERN_SHIMS` on `Pattern.prototype` and registers the widget types. Runs
 *  once at import (a fresh host context imports this module once). */
export function installShims(): void {
  for (const [name, fn] of Object.entries(PATTERN_SHIMS)) (Pattern.prototype as any)[name] = fn;
  for (const type of WIDGET_TYPES) registerWidgetType(type);
}

installShims();

/** The slider state (`@strudel/codemirror` slider.mjs `sliderValues`): written at eval time,
 *  read at query time. Nothing moves a slider in the host. */
export const HOST_SLIDER_VALUES: Record<string, unknown> = {};

/** The scope names the host prebake adds (`evalScope`): the codemirror values songs use. */
export const shims = {
  sliderValues: HOST_SLIDER_VALUES,
  // The transpiled form of `slider(value, min, max)`.
  sliderWithID: (id: string, value: unknown, _min?: unknown, _max?: unknown) => {
    HOST_SLIDER_VALUES[id] = value;
    return ref(() => HOST_SLIDER_VALUES[id]);
  },
  slider: (value: unknown) => {
    console.warn('slider will only work when the transpiler is used... passing value as is');
    return pure(value);
  },
  // A value key, as in codemirror.mjs (the bag `registerControl` returns); the engine ignores it.
  markcss: registerControl('markcss'),
};
