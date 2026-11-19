/**
 * The token system's consistency rules: colours, font sizes, radii, z-indexes and durations come
 * from `src/styles/tokens.css`, and only that file names the palette. Every rule reads the source
 * text of `src/**` (comments stripped) and is checked against a known bad fixture first, so a
 * broken pattern cannot pass silently.
 */
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { strudelandSettings } from '../src/lib/editor/theme';

const ROOT = join(import.meta.dir, '..');
const TOKENS = 'src/styles/tokens.css';
const BASE = 'src/styles/base.css';
const EDITOR_CSS = 'src/styles/editor.css';

/**
 * The names Strudel writes onto `:root` (the keys of the strudeland theme settings, plus
 * `--foreground` and `--background`). Never defined in `tokens.css`, but may be read.
 */
const STRUDEL_VARS = [
  ...new Set([...Object.keys(strudelandSettings(() => 'x')), 'foreground', 'background']),
].map((k) => '--' + k);

/** Text roles that must reach 4.5:1 on every surface. */
const TEXT_ROLES = [
  '--text',
  '--text-dim',
  '--text-faint',
  '--placeholder',
  '--link',
  '--syntax-text',
  '--syntax-keyword',
  '--syntax-string',
  '--syntax-number',
  '--syntax-fn',
  '--syntax-property',
  '--syntax-comment',
  '--syntax-punct',
  '--syntax-operator',
];
const SURFACES = ['--bg', '--surface', '--surface-2', '--surface-3'];

/** The CSS named colours (lowercase). */
const NAMED_COLOURS = new Set(
  (
    'aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet ' +
    'brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan ' +
    'darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta ' +
    'darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue ' +
    'darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey ' +
    'dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray ' +
    'green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush ' +
    'lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray ' +
    'lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray ' +
    'lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon ' +
    'mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue ' +
    'mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin ' +
    'navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen ' +
    'paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple ' +
    'rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna ' +
    'silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle ' +
    'tomato turquoise violet wheat white whitesmoke yellow yellowgreen'
  ).split(' '),
);

// ---------------------------------------------------------------------------------------------
// Source reading

/** Strips block, HTML and line comments (a `//` after `:` or a quote is a URL, not a comment). */
function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/(?<![:'"\w\\])\/\/[^\n]*/g, '');
}

function listSources(): string[] {
  return [...new Bun.Glob('src/**/*.{svelte,css,ts}').scanSync({ cwd: ROOT })]
    .map((p) => p.split('\\').join('/'))
    .sort();
}

/** Memoised: each file is read and stripped once per run. */
const sources = new Map<string, string>();
function source(path: string): string {
  let text = sources.get(path);
  if (text === undefined) {
    text = stripComments(readFileSync(join(ROOT, path), 'utf8'));
    sources.set(path, text);
  }
  return text;
}

/** A function of no arguments, evaluated once on first call. */
function once<T>(make: () => T): () => T {
  let value: T | undefined;
  let made = false;
  return () => {
    if (!made) {
      value = make();
      made = true;
    }
    return value as T;
  };
}

/** Declarations `prop: value` whose property name matches `prop` (kebab or camel case). */
function declarations(text: string, prop: string): string[] {
  const re = new RegExp(`(?<![\\w-])(?:${prop})\\s*:\\s*([^;{}\\n]+)`, 'g');
  return [...text.matchAll(re)].map((m) => m[1].trim().replace(/^['"`]|['"`],?$/g, ''));
}

// ---------------------------------------------------------------------------------------------
// The defined set

function customPropsDefined(text: string): string[] {
  return [...text.matchAll(/(?<![\w-])(--[\w-]+)\s*:/g)].map((m) => m[1]);
}

/** A selector list of only `:root`, `[data-tone…]` and `[data-cat…]`: where tones are set. */
const TONE_SELECTOR =
  /^(?::root|\[data-(?:tone|cat)[^\]]*\])(?:\s*,\s*(?::root|\[data-(?:tone|cat)[^\]]*\]))*$/;

/** The tokens, Strudel's variables, and the `--tone*` properties `base.css`'s tone blocks set. */
const definedSet = once((): Set<string> => {
  const defined = new Set<string>([...customPropsDefined(source(TOKENS)), ...STRUDEL_VARS]);
  for (const block of source(BASE).matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (TONE_SELECTOR.test(block[1].trim())) {
      customPropsDefined(block[2])
        .filter((d) => d.startsWith('--tone'))
        .forEach((d) => defined.add(d));
    }
  }
  return defined;
});

// ---------------------------------------------------------------------------------------------
// Rules: each returns its violations (empty when the text passes).

type Rule = (path: string, text: string) => string[];

function undefinedVars(defined: Set<string>): Rule {
  return (_path, text) =>
    [...text.matchAll(/var\(\s*(--[\w-]+)/g)]
      .map((m) => m[1])
      .filter((v) => !defined.has(v))
      .map((v) => `undefined ${v}`);
}

const HEX = /(?<![\w&])#[0-9a-fA-F]{3,8}(?![\w-])/g;
const COLOUR_FN = /(?<![\w-])(rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(/g;
const COLOUR_PROPS =
  'color|background[\\w-]*|border[\\w-]*|outline[\\w-]*|fill|stroke|box-shadow|boxShadow|' +
  'text-shadow|textShadow|caret-color|caretColor|accent-color|accentColor';

const colourLiterals: Rule = (path, text) => {
  if (path === TOKENS) return [];
  const found = [...text.matchAll(HEX), ...text.matchAll(COLOUR_FN)].map((m) => m[0]);
  for (const value of declarations(text, COLOUR_PROPS)) {
    for (const word of value.replace(/--[\w-]+/g, ' ').match(/[a-zA-Z]+/g) ?? []) {
      if (NAMED_COLOURS.has(word.toLowerCase())) found.push(word);
    }
  }
  return found.map((f) => `colour literal ${f}`);
};

const PALETTE =
  /(?<![\w-])--(?:(?:gray|ink|syn)-[\w-]+|shadow-color|(?:lime|cyan|pink|violet|amber|red)(?![\w-]))/g;

const palettePrivate: Rule = (path, text) =>
  path === TOKENS ? [] : [...text.matchAll(PALETTE)].map((m) => `palette ${m[0]}`);

const scales: Rule = (_path, text) => {
  const bad: string[] = [];
  for (const v of declarations(text, 'font-size|fontSize')) {
    if (!/^var\(--fs-[\w-]+\)$/.test(v) && v !== 'inherit') bad.push(`font-size ${v}`);
  }
  for (const v of declarations(text, 'font')) {
    if (/\d\s*px/.test(v)) bad.push(`font ${v}`);
  }
  for (const v of declarations(text, 'z-index|zIndex')) {
    if (!/^var\(--z-[\w-]+\)$/.test(v)) bad.push(`z-index ${v}`);
  }
  for (const v of declarations(text, 'border(?:-[a-z]+)*-radius|border\\w*Radius')) {
    const ok = v.split(/\s+/).every((p) => /^(var\(--radius-[\w-]+\)|0|50%)$/.test(p));
    if (!ok) bad.push(`radius ${v}`);
  }
  for (const v of declarations(text, '(?:transition|animation)(?:-[a-z-]+|[A-Z]\\w*)?')) {
    if (/(?<![\w.-])\d*\.?\d+m?s(?![\w-])/.test(v)) bad.push(`duration ${v}`);
  }
  return bad;
};

const important: Rule = (path, text) =>
  path === EDITOR_CSS ? [] : (text.match(/!\s*important/g) ?? []).map(() => '!important');

const allRules = once((): Record<string, Rule> => ({
  defined: undefinedVars(definedSet()),
  colourLiterals,
  palettePrivate,
  scales,
  important,
}));

function violations(path: string, text = source(path)): string[] {
  return Object.values(allRules()).flatMap((rule) => rule(path, text));
}

// ---------------------------------------------------------------------------------------------
// Contrast

function tokenMap(): Map<string, string> {
  const root = source(TOKENS).match(/:root\s*\{([^}]*)\}/);
  if (!root) throw new Error('no :root block in tokens.css');
  const map = new Map<string, string>();
  for (const m of root[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) map.set(m[1], m[2].trim());
  return map;
}

/** Follows plain `var(--x)` aliases down to a `#hex`; throws on anything else. */
function resolveHex(map: Map<string, string>, token: string, seen = new Set<string>()): string {
  if (seen.has(token)) throw new Error(`alias cycle at ${token}`);
  seen.add(token);
  const value = map.get(token);
  if (value === undefined) throw new Error(`unknown token ${token}`);
  if (/^#[0-9a-fA-F]{6}$/.test(value)) return value.toLowerCase();
  const alias = value.match(/^var\((--[\w-]+)\)$/);
  if (!alias) throw new Error(`${token} is not a plain alias or #hex: ${value}`);
  return resolveHex(map, alias[1], seen);
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// ---------------------------------------------------------------------------------------------

const SOURCES = listSources();

describe('the rules flag known bad fixtures', () => {
  const rules = allRules();
  const cases: [string, string, string][] = [
    ['defined', 'a.css', '.x { color: var(--nope); }'],
    ['colourLiterals', 'a.css', '.x { color: #ff0000; }'],
    ['colourLiterals', 'a.css', '.x { border: 1px solid #abc; }'],
    ['colourLiterals', 'a.css', '.x { background: rgba(0, 0, 0, 0.5); }'],
    ['colourLiterals', 'a.ts', "ctx.fillStyle = 'oklch(0.7 0.1 120)';"],
    ['colourLiterals', 'a.css', '.x { color: red; }'],
    ['colourLiterals', 'a.ts', "const t = { backgroundColor: 'white' };"],
    ['colourLiterals', 'a.css', '.x { box-shadow: 0 0 0 1px Black; }'],
    ['palettePrivate', 'a.css', '.x { color: var(--gray-3); }'],
    ['palettePrivate', 'a.css', '.x { color: var(--lime); }'],
    ['palettePrivate', 'a.css', '.x { color: var(--syn-mint); }'],
    ['palettePrivate', 'a.css', '.x { box-shadow: 0 0 4px var(--shadow-color); }'],
    ['scales', 'a.css', '.x { font-size: 12px; }'],
    ['scales', 'a.ts', "const t = { fontSize: '15px' };"],
    ['scales', 'a.css', '.x { font: 13px/1.4 sans-serif; }'],
    ['scales', 'a.css', '.x { z-index: 10; }'],
    ['scales', 'a.css', '.x { border-radius: 3px; }'],
    ['scales', 'a.css', '.x { border-top-left-radius: 4px; }'],
    ['scales', 'a.css', '.x { transition: background 120ms ease; }'],
    ['scales', 'a.css', '.x { animation: pulse 1.5s infinite; }'],
    ['important', 'a.css', '.x { color: inherit !important; }'],
  ];
  for (const [rule, path, text] of cases) {
    test(`${rule}: ${text}`, () => {
      expect(rules[rule](path, text).length).toBeGreaterThan(0);
    });
  }

  test('good fixtures pass every rule', () => {
    const good = [
      '.x { color: var(--text); background: transparent; border: 1px solid var(--border); }',
      '.x { font-size: var(--fs-ui); font: inherit; z-index: var(--z-pop); }',
      '.x { border-radius: var(--radius-sm) var(--radius-sm) 0 0; border-radius: 50%; }',
      '.x { transition: background var(--dur-fast) var(--ease); }',
      '.x { outline: var(--focus-ring); color: currentColor; background: none; }',
      '<a href="#app" id="test-canvas">x</a>',
      '.x { color: var(--tone); background: var(--tone-wash); }',
    ];
    for (const text of good) expect(violations('a.css', text)).toEqual([]);
  });

  test('comments are not checked', () => {
    const text = stripComments('/* color: #fff */\n// z-index: 9\n<!-- red -->\n.x {}');
    expect(violations('a.css', text)).toEqual([]);
  });

  test('a URL is not a comment', () => {
    expect(stripComments("const u = 'https://strudel.cc';")).toContain('strudel.cc');
  });

  test('the Strudel variables may be read', () => {
    expect(violations('a.css', '.x { outline-color: var(--foreground); }')).toEqual([]);
  });

  test('the Strudel set is the theme settings keys plus --foreground and --background', () => {
    expect(STRUDEL_VARS).toContain('--muted');
    expect(STRUDEL_VARS).toContain('--gutterForeground');
    expect(STRUDEL_VARS.filter((v) => v === '--foreground')).toHaveLength(1);
    expect(STRUDEL_VARS).toHaveLength(10);
  });

  test('tokens.css may hold literals and the palette', () => {
    expect(colourLiterals(TOKENS, '--x: #fff;')).toEqual([]);
    expect(palettePrivate(TOKENS, '--bg: var(--gray-1);')).toEqual([]);
  });

  test('editor.css may use !important', () => {
    expect(important(EDITOR_CSS, '.cm-editor { background: transparent !important; }')).toEqual([]);
  });
});

describe('src follows the token rules', () => {
  test('the token files are scanned', () => {
    expect(SOURCES).toContain(TOKENS);
    expect(SOURCES).toContain(BASE);
  });

  for (const path of SOURCES) {
    test(path, () => {
      expect(violations(path)).toEqual([]);
    });
  }

  test('base.css defines the tone properties', () => {
    for (const v of ['--tone', '--tone-wash', '--tone-edge', '--tone-ink']) {
      expect(definedSet()).toContain(v);
    }
  });

  test('the tone selector takes :root and data-tone/data-cat lists only', () => {
    expect(TONE_SELECTOR.test(':root, [data-tone], [data-cat]')).toBe(true);
    expect(TONE_SELECTOR.test("[data-cat='1']")).toBe(true);
    expect(TONE_SELECTOR.test('.x, [data-tone]')).toBe(false);
  });

  test("tokens.css never defines Strudel's variables", () => {
    const defined = customPropsDefined(source(TOKENS));
    for (const v of STRUDEL_VARS) expect(defined).not.toContain(v);
  });
});

describe('contrast', () => {
  const map = tokenMap();
  for (const role of TEXT_ROLES) {
    test(`${role} reaches 4.5:1 on every surface`, () => {
      const fg = resolveHex(map, role);
      for (const surface of SURFACES) {
        const ratio = contrast(fg, resolveHex(map, surface));
        if (ratio < 4.5) throw new Error(`${role} on ${surface}: ${ratio.toFixed(2)}:1`);
      }
    });
  }

  test('the resolver throws on a tint or an unknown token', () => {
    expect(() => resolveHex(map, '--accent-wash')).toThrow();
    expect(() => resolveHex(map, '--nope')).toThrow();
  });

  test('contrast maths', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrast('#777777', '#777777')).toBeCloseTo(1, 5);
  });
});

describe('report', () => {
  test('raw px in padding, margin and gap', () => {
    let count = 0;
    for (const path of SOURCES) {
      for (const v of declarations(source(path), 'padding[\\w-]*|margin[\\w-]*|gap')) {
        count += (v.match(/\d\s*px/g) ?? []).length;
      }
    }
    console.log(`theme-tokens: ${count} raw px values in padding, margin and gap`);
    expect(count).toBeGreaterThanOrEqual(0);
  });
});
