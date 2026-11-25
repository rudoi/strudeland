import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { headerRanges, MetaError, parseMeta } from '../src/lib/project/meta';
import { buildStructure, StructureError } from '../src/lib/strudel/structure';

// The Rust crate's templates. MAIN_SONG is read from the crate source so the two can't drift;
// NEW_SONG is `new_song("verse")` (a format! string, so it is copied).
const templateRs = readFileSync(
  new URL('../crates/strudeland-project/src/template.rs', import.meta.url),
  'utf8',
);
const MAIN_SONG = /pub const MAIN_SONG: &str = r#"([\s\S]*?)"#;/.exec(templateRs)![1]!;

const NEW_SONG = `const meta = {
  title: 'verse',
  bpm: 120,
};

$: s("sine*4").note("c2")
`;

const ok = (code: string) => {
  const r = parseMeta(code);
  expect(r.error).toBeNull();
  return r.meta;
};

/** Parse `code`, expect a MetaError, and check it points at `needle`'s first occurrence. */
function errAt(code: string, needle: string) {
  const r = parseMeta(code);
  expect(r.meta).toEqual({});
  expect(r.error).toBeInstanceOf(MetaError);
  const err = r.error!;
  const pos = code.indexOf(needle);
  expect(pos).toBeGreaterThanOrEqual(0);
  const before = code.slice(0, pos).split('\n');
  expect({ pos: err.pos, line: err.line, column: err.column }).toEqual({
    pos,
    line: before.length,
    column: before[before.length - 1].length,
  });
  return err;
}

describe('no header', () => {
  test('empty, comment-only and headerless files give {}', () => {
    expect(ok('')).toEqual({});
    expect(ok('// just a comment\n/* and another */\n')).toEqual({});
    expect(ok('$: s("sine")')).toEqual({});
  });

  test('a header that is not the first statement is ignored', () => {
    expect(ok("setcpm(1); const meta = { title: 'x' }")).toEqual({});
    expect(ok("'use strict';\nconst meta = { title: 'x' }")).toEqual({});
  });

  test('leading comments do not count as statements', () => {
    expect(ok("// hi\n/* there */\nconst meta = { title: 'x' }")).toEqual({ title: 'x' });
  });

  test('let, var, several declarators, other names and non-object inits are not headers', () => {
    expect(ok("let meta = { title: 'x' }")).toEqual({});
    expect(ok("var meta = { title: 'x' }")).toEqual({});
    expect(ok("const meta = { title: 'x' }, x = 1")).toEqual({});
    expect(ok("const x = 1, meta = { title: 'x' }")).toEqual({});
    expect(ok("const info = { title: 'x' }")).toEqual({});
    expect(ok('const meta = 5')).toEqual({});
    expect(ok('const meta = [1]')).toEqual({});
    expect(ok("const { meta } = { meta: { title: 'x' } }")).toEqual({});
  });

  test('a non-header is not checked, even when it would be a MetaError', () => {
    expect(parseMeta('let meta = { title: f() }')).toEqual({ meta: {}, error: null });
  });

  test('export const meta is a syntax error in script mode, so no header and no error', () => {
    expect(parseMeta("export const meta = { title: 'x' }")).toEqual({ meta: {}, error: null });
  });
});

describe('engine', () => {
  test('superdough and rust parse; absent is undefined', () => {
    expect(ok("const meta = { engine: 'rust' };").engine).toBe('rust');
    expect(ok("const meta = { engine: 'superdough' };").engine).toBe('superdough');
    expect(ok("const meta = { title: 'x' };").engine).toBeUndefined();
  });

  test('another value is a MetaError pointing at the value', () => {
    errAt("const meta = { engine: 'x' };", "'x'");
    errAt('const meta = { engine: 3 };', '3');
    expect(parseMeta("const meta = { engine: 'x' };").error!.message).toBe(
      "meta.engine must be 'superdough' or 'rust'",
    );
    expect(parseMeta('const meta = { engine: 3 };').error!.message).toBe(
      "meta.engine must be 'superdough' or 'rust'",
    );
  });

  test('a double-quoted engine is mini-notation, so a header error', () => {
    errAt('const meta = { engine: "rust" };', '"rust"');
    expect(parseMeta('const meta = { engine: "rust" };').error!.message).toBe(
      'meta.engine must use single quotes: double-quoted strings are mini-notation',
    );
  });
});

describe('seed', () => {
  test('numbers of either sign parse', () => {
    expect(ok('const meta = { seed: 7 };').seed).toBe(7);
    expect(ok('const meta = { seed: -3 };').seed).toBe(-3);
    expect(ok('const meta = { seed: 0 };').seed).toBe(0);
    expect(ok('const meta = { seed: 2147483647 };').seed).toBe(2147483647);
  });

  test('absent when the header has none', () => {
    expect(ok("const meta = { title: 'x' };").seed).toBeUndefined();
  });

  test('a non-number is a MetaError pointing at the value', () => {
    errAt("const meta = { seed: 'a' };", "'a'");
    errAt("const meta = { seed: '7' };", "'7'");
    errAt('const meta = { seed: foo };', 'foo');
    expect(parseMeta("const meta = { seed: 'a' };").error!.message).toBe(
      'meta.seed must be an integer between -2147483648 and 2147483647',
    );
  });

  test('a fraction or a seed outside 32 bits is a MetaError pointing at the value', () => {
    errAt('const meta = { seed: 1.5 };', '1.5');
    errAt('const meta = { seed: 2147483648 };', '2147483648');
    errAt('const meta = { seed: -2147483649 };', '-2147483649');
    expect(ok('const meta = { seed: -2147483648 };').seed).toBe(-2147483648);
  });

  test('every occurrence of a duplicate key is checked', () => {
    errAt("const meta = { seed: 1, seed: 'b' };", "'b'");
    errAt("const meta = { seed: 'b', seed: 1 };", "'b'");
  });
});

describe('happy path', () => {
  test('title and bpm', () => {
    expect(ok("const meta = { title: 'Intro theme', bpm: 120 };")).toEqual({
      title: 'Intro theme',
      bpm: 120,
    });
  });

  test('all accepted value kinds', () => {
    const code = `const meta = {
      title: "double",
      'quoted key': 'single',
      bpm: 92.5,
      offset: -3,
      big: 1e3,
      on: true,
      off: false,
      none: null,
      tags: ['a', -1, [true, null], { deep: { x: 1 } }],
      nested: { a: { b: [] }, },
    };`;
    expect(ok(code)).toEqual({
      title: 'double',
      'quoted key': 'single',
      bpm: 92.5,
      offset: -3,
      big: 1000,
      on: true,
      off: false,
      none: null,
      tags: ['a', -1, [true, null], { deep: { x: 1 } }],
      nested: { a: { b: [] } },
    });
  });

  test('an empty header is {}', () => {
    expect(ok('const meta = {}')).toEqual({});
  });

  test('unknown keys are kept', () => {
    expect(ok("const meta = { author: 'me', key: 'Cm' }")).toEqual({ author: 'me', key: 'Cm' });
  });

  test('a __proto__ key is a plain own property', () => {
    const meta = ok("const meta = { __proto__: { title: 'x' } }");
    expect(Object.getPrototypeOf(meta)).toBe(Object.prototype);
    expect(meta.title).toBeUndefined();
    expect(Object.keys(meta)).toEqual(['__proto__']);
  });

  test('a duplicate key keeps the last value', () => {
    expect(ok("const meta = { title: 'a', title: 'b' }")).toEqual({ title: 'b' });
  });

  test('the main song template', () => {
    expect(ok(MAIN_SONG)).toEqual({
      title: 'main',
      bpm: 120,
      meter: '4/4',
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'body', bars: 4, loop: true },
        { name: 'tail', bars: 2 },
      ],
    });
  });

  test('the new song template', () => {
    expect(ok(NEW_SONG)).toEqual({ title: 'verse', bpm: 120 });
  });

  test('later meta.bpm references and top-level await still parse', () => {
    const code =
      "const meta = { bpm: 140 };\nsetcpm(meta.bpm / 4)\nawait samples('x')\n$: s(meta.bpm)";
    expect(ok(code)).toEqual({ bpm: 140 });
  });
});

describe('MetaError', () => {
  test('identifier value', () => {
    const e = errAt('const meta = { x: foo }', 'foo');
    expect([e.pos, e.line, e.column]).toEqual([18, 1, 18]);
    expect(e.message).toBe('meta.x must be a literal value, not an identifier');
  });

  test('positions on later lines', () => {
    const e = errAt("const meta = {\n  title: 'a',\n  bpm: tempo,\n};", 'tempo');
    expect([e.pos, e.line, e.column]).toEqual([36, 3, 7]);
  });

  test('call', () => {
    errAt('const meta = { x: f(1) }', 'f(1)');
  });

  test('spread in an object and in an array', () => {
    errAt('const meta = { ...other }', '...other');
    errAt('const meta = { x: [1, ...rest] }', '...rest');
  });

  test('computed key', () => {
    errAt("const meta = { [k]: 'v' }", 'k]');
  });

  test('numeric key', () => {
    errAt("const meta = { 1: 'v' }", "1: 'v'");
  });

  test('getter, setter and method', () => {
    errAt('const meta = { get x() { return 1 } }', 'get x');
    errAt('const meta = { set x(v) {} }', 'set x');
    errAt('const meta = { x() { return 1 } }', 'x()');
  });

  test('shorthand property', () => {
    errAt('const meta = { title }', 'title');
  });

  test('template literals, with or without expressions', () => {
    errAt('const meta = { x: `a${b}` }', '`a');
    errAt('const meta = { x: `plain` }', '`plain');
  });

  test('other expressions and literal kinds', () => {
    errAt('const meta = { x: +1 }', '+1');
    errAt('const meta = { x: -y }', '-y');
    errAt("const meta = { x: -'1' }", "-'1'");
    errAt('const meta = { x: 1 + 2 }', '1 + 2');
    errAt('const meta = { x: /re/ }', '/re/');
    errAt('const meta = { x: 10n }', '10n');
    errAt('const meta = { x: () => 1 }', '() => 1');
    errAt('const meta = { x: undefined }', 'undefined');
  });

  test('array hole', () => {
    errAt('const meta = { x: [1, , 2] }', '[1');
  });

  test('nested error reports the inner node', () => {
    const e = errAt('const meta = { a: { b: [1, { c: nope }] } }', 'nope');
    expect(e.message).toContain('meta.a.b[1].c');
  });

  test('title must be a string', () => {
    const e = errAt('const meta = { title: 5 }', '5');
    expect(e.message).toBe('meta.title must be a string');
    errAt('const meta = { title: null }', 'null');
    errAt("const meta = { title: ['a'] }", "['a']");
  });

  test('bpm must be a number from 20 to 400', () => {
    const e = errAt("const meta = { bpm: '120' }", "'120'");
    expect(e.message).toBe('meta.bpm must be a number from 20 to 400');
    errAt('const meta = { bpm: 19 }', '19');
    errAt('const meta = { bpm: 401 }', '401');
    errAt('const meta = { bpm: -120 }', '-120');
    errAt('const meta = { bpm: NaN }', 'NaN');
    errAt('const meta = { bpm: Infinity }', 'Infinity');
    errAt('const meta = { bpm: 1e999 }', '1e999');
    errAt('const meta = { bpm: -1e999 }', '-1e999');
  });

  test('a duplicate key cannot hide a bad value', () => {
    errAt("const meta = { title: 5, title: 'b' }", '5');
  });

  test('a duplicate bpm is checked on every occurrence', () => {
    errAt('const meta = { bpm: 1000, bpm: 120 }', '1000');
    errAt('const meta = { bpm: 120, bpm: 1000 }', '1000');
  });

  test('title and bpm checks are top-level only', () => {
    expect(ok('const meta = { section: { title: 5, bpm: "x" } }')).toEqual({
      section: { title: 5, bpm: 'x' },
    });
  });

  test('the first offending node wins', () => {
    errAt('const meta = { a: one, b: two }', 'one');
  });
});

describe('syntax errors and garbage', () => {
  test('a syntax error in the header gives {} and no error', () => {
    expect(parseMeta("const meta = { title: 'x' ")).toEqual({ meta: {}, error: null });
    expect(parseMeta("const meta = { title: 'x', }}")).toEqual({ meta: {}, error: null });
    expect(parseMeta("const meta = { title: 'x' } junk")).toEqual({ meta: {}, error: null });
    expect(parseMeta("const meta = { title: 'x' } /* unterminated")).toEqual({
      meta: {},
      error: null,
    });
  });

  test('a syntax error after the header still reads the header', () => {
    expect(parseMeta("const meta = { title: 'x' };\n$: s(")).toEqual({
      meta: { title: 'x' },
      error: null,
    });
    expect(parseMeta("const meta = { title: 'x' }\n)(")).toEqual({
      meta: { title: 'x' },
      error: null,
    });
    expect(parseMeta("const meta = { title: 'x' } // note\nlet = ;")).toEqual({
      meta: { title: 'x' },
      error: null,
    });
    expect(parseMeta("const meta = { title: 'x' } /* a\nb */ }")).toEqual({
      meta: { title: 'x' },
      error: null,
    });
    errAt('const meta = { x: foo };\n)(', 'foo');
  });

  test('an expression continuing the object is not a header', () => {
    expect(ok("const meta = { title: 'x' }\n.title")).toEqual({});
    expect(ok("const meta = { title: 'x' }\n['title']")).toEqual({});
    expect(ok("const meta = { title: 'x' } ? 1 : 2")).toEqual({});
  });

  test('never throws', () => {
    const inputs = [
      '\u0000',
      '{{{{',
      '}}}}',
      'const',
      'const meta',
      'const meta =',
      'const meta = {',
      '`${',
      '/*',
      "'",
      '\uD800',
      'const meta = ' + '['.repeat(500) + ']'.repeat(500),
      'const meta = { a: ' + '{ a: '.repeat(200) + '1' + ' }'.repeat(201),
      Array.from({ length: 200 }, (_, i) => String.fromCharCode((i * 7919) % 0x2fff)).join(''),
    ];
    for (const input of inputs) {
      expect(() => parseMeta(input)).not.toThrow();
    }
  });
});

const SECTIONS = `const meta = {
  title: 'gorim',
  bpm: 108,
  meter: '6/8',
  sections: [
    { name: 'intro', bars: 4 },
    { name: 'body', bars: 16, loop: true },
    { name: 'tail', bars: 2 },
  ],
  loops: 2,
};

$: song(meta, { intro: 'a', body: 'b', tail: 'c' })
`;

describe('structure fields', () => {
  test('a full sectioned header', () => {
    expect(ok(SECTIONS)).toEqual({
      title: 'gorim',
      bpm: 108,
      meter: '6/8',
      sections: [
        { name: 'intro', bars: 4 },
        { name: 'body', bars: 16, loop: true },
        { name: 'tail', bars: 2 },
      ],
      loops: 2,
    });
  });

  test('bpm limits are inclusive', () => {
    expect(ok('const meta = { bpm: 20 }')).toEqual({ bpm: 20 });
    expect(ok('const meta = { bpm: 400 }')).toEqual({ bpm: 400 });
  });

  test('meter', () => {
    expect(ok("const meta = { meter: '11/8' }")).toEqual({ meter: '11/8' });
    const e = errAt("const meta = { meter: '4/3' }", "'4/3'");
    expect(e.message).toBe(
      "meta.meter must be a string 'N/D' with N from 1 to 32 and D one of 1, 2, 4, 8, 16",
    );
    errAt('const meta = { meter: 4 }', '4 }');
    errAt("const meta = {\n  title: 'x',\n  meter: '0/4',\n}", "'0/4'");
    errAt("const meta = { meter: '4/4', meter: '33/4' }", "'33/4'");
  });

  test('sections errors point at the sections value with the index in the message', () => {
    const code = `const meta = {
  sections: [
    { name: 'intro', bars: 4 },
    { name: 'body', bars: 0 },
  ],
};`;
    const e = errAt(code, '[\n    {');
    expect(e.line).toBe(2);
    expect(e.column).toBe(12);
    expect(e.message).toBe('meta.sections[1].bars must be an integer from 1 to 10000');
  });

  test('every sections rule', () => {
    const cases: [string, string][] = [
      ['[]', 'meta.sections must be a non-empty array of sections'],
      ["'intro'", 'meta.sections must be a non-empty array of sections'],
      [
        "[{ name: 'a', bars: 1 }, 5]",
        "meta.sections[1] must be an object like { name: 'intro', bars: 4 }",
      ],
      [
        "[{ name: '1a', bars: 1 }]",
        'meta.sections[0].name must be an identifier (letters, digits and _, not starting with a digit)',
      ],
      [
        "[{ name: 'a', bars: 1 }, { name: 'a', bars: 2 }]",
        "meta.sections[1].name 'a' is already used",
      ],
      ["[{ name: 'a', bars: 1.5 }]", 'meta.sections[0].bars must be an integer from 1 to 10000'],
      ["[{ name: 'a', bars: 1, loop: 1 }]", 'meta.sections[0].loop must be true or false'],
      [
        "[{ name: 'a', bars: 1, loop: true }, { name: 'b', bars: 1, loop: true }]",
        'meta.sections[1].loop: only one section can loop',
      ],
      ["[{ name: 'a', bars: 1, fade: 2 }]", "meta.sections[0] has an unknown key 'fade'"],
    ];
    for (const [value, message] of cases) {
      const e = errAt(`const meta = { title: 'x', sections: ${value} }`, value);
      expect(e.message).toBe(message);
    }
  });

  test('a non-literal inside sections is still a literal error at the inner node', () => {
    const e = errAt("const meta = { sections: [{ name: 'a', bars: n }] }", 'n }');
    expect(e.message).toBe('meta.sections[0].bars must be a literal value, not an identifier');
  });

  test('loops', () => {
    const loop = "sections: [{ name: 'a', bars: 1 }, { name: 'b', bars: 2, loop: true }]";
    expect(ok(`const meta = { ${loop}, loops: 1000 }`).loops).toBe(1000);
    expect(ok(`const meta = { loops: 4, ${loop} }`).loops).toBe(4);
    const range = errAt(`const meta = { ${loop}, loops: 0 }`, '0 }');
    expect(range.message).toBe('meta.loops must be an integer from 1 to 1000');
    errAt(`const meta = { ${loop}, loops: 2.5 }`, '2.5');
    errAt(`const meta = { loops: 1001, ${loop} }`, '1001');
  });

  test('loops needs a loop section, positioned at the loops value', () => {
    const e = errAt("const meta = { loops: 2, sections: [{ name: 'a', bars: 1 }] }", '2,');
    expect(e.message).toBe('meta.loops needs a section with loop: true or a meta.loop range');
    errAt('const meta = { loops: 2 }', '2 }');
    // The last `sections` wins, so its loop section is the one that counts.
    errAt(
      "const meta = { sections: [{ name: 'a', bars: 1, loop: true }], sections: [{ name: 'a', bars: 1 }], loops: 3 }",
      '3 }',
    );
  });

  test('a bad duplicate loops is caught; the last one is checked against the sections', () => {
    errAt(
      "const meta = { loops: 0, loops: 2, sections: [{ name: 'a', bars: 1, loop: true }] }",
      '0,',
    );
    errAt('const meta = { loops: 1, loops: 2 }', '2 }');
  });

  test('meter and section names must be single-quoted (double quotes are mini-notation)', () => {
    const m = errAt('const meta = { meter: "6/8" }', '"6/8"');
    expect(m.message).toBe(
      'meta.meter must use single quotes: double-quoted strings are mini-notation',
    );
    // Even when the value is invalid anyway: the quote style is the first thing to fix.
    errAt('const meta = { meter: "4/3" }', '"4/3"');
    const code = `const meta = {
  sections: [
    { name: 'intro', bars: 4 },
    { bars: 2, name: "body" },
  ],
};`;
    const n = errAt(code, '"body"');
    expect(n.message).toBe(
      'meta.sections[1].name must use single quotes: double-quoted strings are mini-notation',
    );
    expect(n.line).toBe(4);
    // A duplicate name key cannot hide one.
    errAt(`const meta = { sections: [{ name: "a", name: 'a', bars: 1 }] }`, '"a"');
    // The title is only displayed (read statically), never read at runtime: double quotes are fine.
    expect(ok('const meta = { title: "x", meter: \'6/8\' }')).toEqual({ title: 'x', meter: '6/8' });
    // Other keys are not read by the app as strings.
    expect(ok('const meta = { key: "Cm", x: { meter: "6/8" } }')).toEqual({
      key: 'Cm',
      x: { meter: '6/8' },
    });
  });

  test('structure keys are top-level only and unknown keys stay allowed', () => {
    expect(
      ok("const meta = { duration: 30, fadeOut: true, x: { meter: 'x', sections: 1, loops: 0 } }"),
    ).toEqual({ duration: 30, fadeOut: true, x: { meter: 'x', sections: 1, loops: 0 } });
  });
});

describe('headerRanges', () => {
  test('the header object with offsets into the code', () => {
    const code = "// c\nconst meta = { title: 'x', bpm: 90 };\n$: s('bd')";
    const h = headerRanges(code)!;
    expect(h.type).toBe('ObjectExpression');
    expect(code.slice(h.start, h.end)).toBe("{ title: 'x', bpm: 90 }");
    const [title, bpm] = h.properties as any[];
    expect(code.slice(title.key.start, title.key.end)).toBe('title');
    expect(code.slice(title.value.start, title.value.end)).toBe("'x'");
    expect(code.slice(bpm.value.start, bpm.value.end)).toBe('90');
  });

  test('nested sections are reachable', () => {
    const h = headerRanges(SECTIONS)!;
    const sections = (h.properties as any[]).find((p) => p.key.name === 'sections');
    const body = sections.value.elements[1];
    const bars = body.properties.find((p: any) => p.key.name === 'bars');
    expect(SECTIONS.slice(bars.value.start, bars.value.end)).toBe('16');
  });

  test('an empty header', () => {
    const h = headerRanges('const meta = {}')!;
    expect(h.properties).toEqual([]);
    expect([h.start, h.end]).toEqual([13, 15]);
  });

  test('null for no header, a syntax error or a MetaError', () => {
    expect(headerRanges('')).toBeNull();
    expect(headerRanges("$: s('bd')")).toBeNull();
    expect(headerRanges("let meta = { title: 'x' }")).toBeNull();
    expect(headerRanges("const meta = { title: 'x' ")).toBeNull();
    expect(headerRanges('const meta = { title: f() }')).toBeNull();
    expect(headerRanges('const meta = { bpm: 1000 }')).toBeNull();
    expect(headerRanges('const meta = { loops: 2 }')).toBeNull();
  });

  test('a syntax error after the header still gives the header', () => {
    expect(headerRanges("const meta = { title: 'x' };\n$: s(")).not.toBeNull();
  });
});

describe('meter changes and loop ranges', () => {
  const layout = "sections: [{ name: 'F', bars: 2 }, { name: 'E', bars: 2 }]";
  test('can resolve metadata independently of property order', () => {
    const meta = ok(`const meta = { loops: 2, loop: { from: 'F', through: 'E' },
      meterChanges: [{ bar: 2, meter: '3/4' }], ${layout} }`);
    expect(meta.loop).toEqual({ from: 'F', through: 'E' });
    expect(meta.meterChanges).toEqual([{ bar: 2, meter: '3/4' }]);
  });
  test('reports invalid range and meter changes against their metadata', () => {
    expect(
      errAt(`const meta = { ${layout}, loop: { from: 'E', through: 'F' } }`, "{ from: 'E'").message,
    ).toContain('at or after');
    expect(
      errAt(`const meta = { ${layout}, loop: { from: 'F', through: 'G' } }`, "{ from: 'F'").message,
    ).toContain('unknown section');
    expect(
      errAt(
        "const meta = { sections: [{ name: 'F', bars: 2, loop: true }], loop: { from: 'F', through: 'F' } }",
        "{ from: 'F'",
      ).message,
    ).toContain('cannot be used');
    expect(
      errAt(`const meta = { ${layout}, meterChanges: [{ bar: 5, meter: '3/4' }] }`, '[{ bar: 5')
        .message,
    ).toContain('at most 4');
    expect(
      errAt(
        `const meta = { ${layout}, meterChanges: [{ bar: 3, meter: '3/4' }, { bar: 3, meter: '4/4' }] }`,
        '[{ bar: 3',
      ).message,
    ).toContain('after the previous');
    errAt(`const meta = { ${layout}, loop: { from: "F", through: 'E' } }`, '"F"');
    errAt(`const meta = { ${layout}, meterChanges: [{ bar: 2, meter: "3/4" }] }`, '"3/4"');
  });
});

describe('the first error of a header with several faults', () => {
  const one = "sections: [{ name: 'a', bars: 1 }]";
  const two = "sections: [{ name: 'a', bars: 1 }, { name: 'b', bars: 1 }]";
  const looped = "sections: [{ name: 'a', bars: 1, loop: true }]";
  const four = "sections: [{ name: 'a', bars: 4 }]";
  const RANGE = 'meta.loops must be an integer from 1 to 1000';
  const NO_LOOP = 'meta.loops needs a section with loop: true or a meta.loop range';
  // [header body, message, needle, which occurrence of the needle (0-based)]; null message = ok.
  const rows: [string, string | null, string?, number?][] = [
    [
      `${one}, loop: { from: 'x', through: 'a' }, loops: 0, loops: 2`,
      "meta.loop.from names an unknown section 'x'",
      '{ from',
    ],
    ['loops: 0, loops: 2', RANGE, '0'],
    [`${looped}, loops: 0, loops: 2`, RANGE, '0,'],
    ['loops: 2, loops: 3', NO_LOOP, '3'],
    ['loops: 2, loops: 0', RANGE, '0'],
    [`${one}, loop: { from: 'a', through: 'a' }, loops: 1001, loops: 0`, RANGE, '1001'],
    [
      `loops: 0, ${one}, loop: { from: 'x', through: 'a' }`,
      "meta.loop.from names an unknown section 'x'",
      '{ from',
    ],
    [
      `loops: 0, ${one}, meterChanges: [{ bar: 2, meter: '3/4' }]`,
      'meta.meterChanges[0].bar must be a whole bar number greater than 1 and at most 1',
      '[{ bar',
    ],
    [
      `${looped}, meterChanges: [{ bar: 5, meter: '3/4' }], loop: { from: 'a', through: 'a' }`,
      'meta.loop cannot be used with section loop: true',
      '{ from',
    ],
    [
      `meterChanges: [{ bar: 2, meter: '3/4' }], loops: 2`,
      'meta.meterChanges needs meta.sections',
      '[{ bar',
    ],
    [
      `loop: { from: 'a', through: 'a' }, meterChanges: [{ bar: 2, meter: '3/4' }]`,
      'meta.loop needs meta.sections',
      '{ from',
    ],
    [`${two}, loop: { from: 'x', through: 'a' }, loop: { from: 'a', through: 'b' }`, null],
    [
      `${two}, loop: { from: 'a', through: 'b' }, loop: { from: 'b', through: 'a' }`,
      'meta.loop.through must be at or after meta.loop.from',
      '{ from',
      1,
    ],
    [`${looped}, loop: { from: 'a', through: 'a' }, ${one}`, null],
    [
      `${looped}, loop: { from: 'a', through: 'a' }, ${looped}`,
      'meta.loop cannot be used with section loop: true',
      '{ from',
    ],
    [
      `${four}, meterChanges: [{ bar: 9, meter: '3/4' }], meterChanges: [{ bar: 2, meter: '3/4' }]`,
      null,
    ],
    [
      `${four}, meterChanges: [{ bar: 2, meter: '3/4' }], meterChanges: [{ bar: 9, meter: '3/4' }]`,
      'meta.meterChanges[0].bar must be a whole bar number greater than 1 and at most 4',
      '[{ bar',
      1,
    ],
    [
      `loop: { from: 'x', through: 'a' }, bpm: 10`,
      'meta.bpm must be a number from 20 to 400',
      '10',
    ],
    [`${one}, loop: { from: 'a', through: 'a' }, loops: 0, loops: 3`, RANGE, '0'],
  ];

  for (const [body, message, needle, nth = 0] of rows) {
    test(`{ ${body} }`, () => {
      const code = `const meta = { ${body} }`;
      const r = parseMeta(code);
      if (message === null) {
        expect(r.error).toBeNull();
        return;
      }
      let pos = -1;
      for (let i = 0; i <= nth; i++) pos = code.indexOf(needle!, pos + 1);
      expect(pos).toBeGreaterThan(0);
      expect(r.meta).toEqual({});
      expect(r.error).toBeInstanceOf(MetaError);
      const { line, column } = r.error!;
      expect({ message: r.error!.message, pos: r.error!.pos, line, column }).toEqual({
        message,
        pos,
        line: 1,
        column: pos,
      });
    });
  }
});

describe('the header text and the evaluated object agree on the cross-field rules', () => {
  const a = { name: 'a', bars: 1 };
  const b = { name: 'b', bars: 1 };
  const cases: [string, Record<string, unknown>][] = [
    ['loop needs sections', { loop: { from: 'a', through: 'a' } }],
    ['loop.from unknown', { sections: [a], loop: { from: 'x', through: 'a' } }],
    ['loop.through unknown', { sections: [a], loop: { from: 'a', through: 'x' } }],
    ['loop backwards', { sections: [a, b], loop: { from: 'b', through: 'a' } }],
    [
      'loop with a loop section',
      { sections: [{ ...a, loop: true }], loop: { from: 'a', through: 'a' } },
    ],
    ['meterChanges needs sections', { meterChanges: [{ bar: 2, meter: '3/4' }] }],
    ['meterChanges past the end', { sections: [a, b], meterChanges: [{ bar: 3, meter: '3/4' }] }],
    ['loops needs a loop', { sections: [a], loops: 2 }],
    ['loops needs a loop, no sections', { loops: 2 }],
  ];
  /** A header with single-quoted strings, as songs write them. */
  const header = (meta: Record<string, unknown>) =>
    `const meta = ${JSON.stringify(meta).replaceAll('"', "'")};`;

  for (const [name, meta] of cases) {
    test(name, () => {
      let evaluated: unknown;
      try {
        buildStructure(meta);
      } catch (e) {
        evaluated = e;
      }
      expect(evaluated).toBeInstanceOf(StructureError);
      const parsed = parseMeta(header(meta)).error;
      expect(parsed).toBeInstanceOf(MetaError);
      expect(parsed!.message).toBe((evaluated as StructureError).message);
    });
  }
});
