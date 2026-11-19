import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { parseMeta } from '../src/lib/project/meta';
import {
  printValue,
  setMetaValue,
  type MetaEdit,
  type MetaPath,
} from '../src/lib/project/meta-write';

// The Rust crate's templates: MAIN_SONG read from the source, NEW_SONG a copy of `new_song`.
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

const applyEdit = (code: string, e: MetaEdit) =>
  code.slice(0, e.from) + e.insert + code.slice(e.to);

/** Set `path` to `value`, expect an edit, and check the result re-parses with it. */
function set(code: string, path: MetaPath, value: number | string): string {
  const edit = setMetaValue(code, path, value);
  expect(edit).not.toBeNull();
  const next = applyEdit(code, edit!);
  const { meta, error } = parseMeta(next);
  expect(error).toBeNull();
  const m = /^sections\[(\d+)\]\.bars$/.exec(path);
  if (m) expect((meta.sections as any[])[Number(m[1])].bars).toBe(value);
  else expect(meta[path]).toBe(value);
  return next;
}

const SECTIONED = `const meta = {
  title: 'gorim',
  bpm: 108,
  sections: [
    { name: 'intro', bars: 4 },
    { name: 'body', bars: 16, loop: true },
    { name: 'tail', bars: 2 },
  ],
  loops: 2,
};

$: song(meta, { intro: a, body: b, tail: c })
`;

describe('replace an existing value', () => {
  test('bpm, meter, loops: exactly the value range', () => {
    const edit = setMetaValue(SECTIONED, 'bpm', 140)!;
    expect(SECTIONED.slice(edit.from, edit.to)).toBe('108');
    expect(edit.insert).toBe('140');
    expect(set(SECTIONED, 'bpm', 140)).toBe(SECTIONED.replace('bpm: 108', 'bpm: 140'));
    expect(set(SECTIONED, 'loops', 3)).toBe(SECTIONED.replace('loops: 2', 'loops: 3'));
    const metered = SECTIONED.replace("title: 'gorim',", "title: 'gorim',\n  meter: '4/4',");
    expect(set(metered, 'meter', '6/8')).toBe(metered.replace("'4/4'", "'6/8'"));
  });

  test('sections[i].bars', () => {
    expect(set(SECTIONED, 'sections[0].bars', 8)).toBe(
      SECTIONED.replace("'intro', bars: 4", "'intro', bars: 8"),
    );
    expect(set(SECTIONED, 'sections[1].bars', 32)).toBe(
      SECTIONED.replace("'body', bars: 16", "'body', bars: 32"),
    );
    expect(set(SECTIONED, 'sections[2].bars', 1)).toBe(
      SECTIONED.replace("'tail', bars: 2", "'tail', bars: 1"),
    );
  });

  test('a string key and a value with a comment after it', () => {
    const code = `const meta = { 'bpm': 90 /* slow */, title: 'x' };\n`;
    expect(set(code, 'bpm', 100)).toBe(`const meta = { 'bpm': 100 /* slow */, title: 'x' };\n`);
  });

  test('a fractional bpm is printed as a number', () => {
    expect(set(`const meta = { bpm: 120 };\n`, 'bpm', 97.5)).toBe(`const meta = { bpm: 97.5 };\n`);
  });
});

describe('insert a missing key', () => {
  test('one-line header', () => {
    expect(set(`const meta = { title: 'x' };\n$: s("a")\n`, 'bpm', 90)).toBe(
      `const meta = { title: 'x', bpm: 90 };\n$: s("a")\n`,
    );
  });

  test('one-line header with a trailing comma keeps it', () => {
    expect(set(`const meta = { title: 'x', };\n`, 'meter', '3/4')).toBe(
      `const meta = { title: 'x', meter: '3/4', };\n`,
    );
  });

  test('multi-line header copies the indent', () => {
    const code = `const meta = {\n    title: 'x',\n    bpm: 100\n};\n`;
    expect(set(code, 'meter', '7/8')).toBe(
      `const meta = {\n    title: 'x',\n    bpm: 100,\n    meter: '7/8'\n};\n`,
    );
  });

  test('multi-line header with a trailing comma', () => {
    const code = `const meta = {\n  title: 'x',\n  bpm: 100,\n};\n`;
    expect(set(code, 'meter', '7/8')).toBe(
      `const meta = {\n  title: 'x',\n  bpm: 100,\n  meter: '7/8',\n};\n`,
    );
  });

  test('trailing comment after the last value, without a trailing comma', () => {
    const code = `const meta = {\n  title: 'x',\n  bpm: 100 // quarter notes\n};\n`;
    expect(set(code, 'meter', '6/8')).toBe(
      `const meta = {\n  title: 'x',\n  bpm: 100,\n  meter: '6/8' // quarter notes\n};\n`,
    );
  });

  test('trailing comment after the last value, with a trailing comma', () => {
    const code = `const meta = {\n  title: 'x',\n  bpm: 100, // quarter notes\n};\n`;
    expect(set(code, 'meter', '6/8')).toBe(
      `const meta = {\n  title: 'x',\n  bpm: 100,\n  meter: '6/8', // quarter notes\n};\n`,
    );
  });

  test('last property on the brace line of a multi-line header', () => {
    const code = `const meta = { title: 'x'\n};\n`;
    expect(set(code, 'bpm', 90)).toBe(`const meta = { title: 'x',\n  bpm: 90\n};\n`);
  });

  test('after a nested sections value', () => {
    const code = SECTIONED.replace('  loops: 2,\n', '');
    expect(set(code, 'loops', 4)).toBe(SECTIONED.replace('loops: 2', 'loops: 4'));
    const oneLine = `const meta = { sections: [{ name: 'a', bars: 2, loop: true }] };\n`;
    expect(set(oneLine, 'loops', 3)).toBe(
      `const meta = { sections: [{ name: 'a', bars: 2, loop: true }], loops: 3 };\n`,
    );
  });

  test('empty header', () => {
    expect(set(`const meta = {};\n`, 'bpm', 90)).toBe(`const meta = { bpm: 90 };\n`);
    expect(set(`const meta = {\n};\n`, 'bpm', 90)).toBe(`const meta = { bpm: 90 };\n`);
    expect(set(`const meta = { /* later */ };\n`, 'bpm', 90)).toBe(
      `const meta = { bpm: 90, /* later */ };\n`,
    );
  });

  test('no header: one is inserted at the top', () => {
    expect(set(`$: s("a")\n`, 'bpm', 90)).toBe(`const meta = { bpm: 90 };\n\n$: s("a")\n`);
    expect(set('', 'meter', '5/4')).toBe(`const meta = { meter: '5/4' };\n\n`);
  });
});

describe('the templates', () => {
  test('MAIN_SONG: every path', () => {
    for (const [path, value] of [
      ['bpm', 96],
      ['meter', '6/8'],
    ] as const) {
      set(MAIN_SONG, path, value);
    }
    const meta = parseMeta(MAIN_SONG).meta;
    if (Array.isArray(meta.sections)) {
      meta.sections.forEach((_, i) => set(MAIN_SONG, `sections[${i}].bars`, 3));
      if (meta.sections.some((s: any) => s.loop)) set(MAIN_SONG, 'loops', 5);
    }
  });

  test('NEW_SONG: replace bpm, insert meter after the trailing comma', () => {
    expect(set(NEW_SONG, 'bpm', 90)).toBe(NEW_SONG.replace('bpm: 120', 'bpm: 90'));
    expect(set(NEW_SONG, 'meter', '3/4')).toBe(
      NEW_SONG.replace('bpm: 120,\n', "bpm: 120,\n  meter: '3/4',\n"),
    );
  });
});

describe('refusals', () => {
  test('a header with a meta error', () => {
    expect(setMetaValue(`const meta = { bpm: 'fast' };\n`, 'bpm', 90)).toBeNull();
    expect(setMetaValue(`const meta = { title: x };\n`, 'bpm', 90)).toBeNull();
  });

  test('a duplicated key on the path', () => {
    expect(setMetaValue(`const meta = { bpm: 90, bpm: 100 };\n`, 'bpm', 120)).toBeNull();
    const dupSections = `const meta = { sections: [{ name: 'a', bars: 1 }], sections: [{ name: 'a', bars: 1 }] };\n`;
    expect(setMetaValue(dupSections, 'sections[0].bars', 2)).toBeNull();
    // A duplicate elsewhere does not block an unrelated path.
    expect(setMetaValue(`const meta = { title: 'a', title: 'b' };\n`, 'bpm', 90)).not.toBeNull();
  });

  test('a section index out of range, or no sections', () => {
    expect(setMetaValue(SECTIONED, 'sections[3].bars', 2)).toBeNull();
    expect(setMetaValue(`const meta = { bpm: 90 };\n`, 'sections[0].bars', 2)).toBeNull();
    expect(setMetaValue(`$: s("a")\n`, 'sections[0].bars', 2)).toBeNull();
  });

  test('a header that does not parse, or a meta declared elsewhere', () => {
    expect(setMetaValue(`const meta = { bpm: 90,, };\n`, 'bpm', 100)).toBeNull();
    expect(setMetaValue(`const meta = { bpm: 90\n$: s("a")\n`, 'bpm', 100)).toBeNull();
    expect(setMetaValue(`const meta = { bpm: 90 }, x = 1;\n`, 'bpm', 100)).toBeNull();
    expect(setMetaValue(`let x = 1;\nconst meta = { bpm: 90 };\n`, 'bpm', 100)).toBeNull();
  });

  test('a result that would not read back cleanly', () => {
    expect(setMetaValue(`const meta = { bpm: 90 };\n`, 'bpm', NaN)).toBeNull();
    expect(setMetaValue(`const meta = { bpm: 90 };\n`, 'bpm', 1000)).toBeNull();
    expect(setMetaValue(`const meta = { bpm: 90 };\n`, 'loops', 2)).toBeNull(); // no loop section
    expect(setMetaValue(SECTIONED, 'sections[0].bars', 0)).toBeNull();
    expect(setMetaValue(SECTIONED, 'meter', 'four')).toBeNull();
  });
});

describe('printing', () => {
  test('numbers and escaped single-quoted strings', () => {
    expect(printValue(120)).toBe('120');
    expect(printValue(-1.5)).toBe('-1.5');
    expect(printValue(`it's a \\ "test"\nline\r`)).toBe(`'it\\'s a \\\\ "test"\\nline\\r'`);
    expect(printValue('\u2028')).toBe(`'\\u2028'`);
  });

  test('an escaped string round-trips through the header', () => {
    // meter is the only string path and its values need no escaping; check any string here.
    for (const s of [`it's`, `back\\slash`, `two\nlines`, `"dq"`]) {
      expect(parseMeta(`const meta = { title: ${printValue(s)} };\n`).meta.title).toBe(s);
    }
  });
});
