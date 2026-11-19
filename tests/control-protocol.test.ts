import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import {
  ControlError,
  ErrorCode,
  METHOD_NAMES,
  type ControlRequest,
  type LevelsSummary,
  type LiveStats,
  type MethodName,
} from '../src/lib/control/protocol';

const rust = readFileSync(
  new URL('../crates/strudeland-ctl/src/protocol.rs', import.meta.url),
).toString();

/** Every `pub const NAME: i64 = N;` in the Rust file. */
const rustCodes = new Map(
  [...rust.matchAll(/pub const ([A-Z_]+): i64 = (-?\d+);/g)].map((m) => [m[1]!, Number(m[2])]),
);

describe('ErrorCode', () => {
  for (const [name, code] of Object.entries(ErrorCode)) {
    test(`${name} matches protocol.rs`, () => {
      expect(rustCodes.get(name)).toBe(code);
    });
  }

  test('the Rust file defines no code the frontend lacks', () => {
    expect([...rustCodes.keys()].sort()).toEqual(Object.keys(ErrorCode).sort());
  });

  test('the inspection codes have their documented values', () => {
    expect(ErrorCode.NOTHING_EVALUATED).toBe(-32007);
    expect(ErrorCode.SCREENSHOT_FAILED).toBe(-32008);
  });

  test('the song and project codes have their documented values', () => {
    expect(ErrorCode.UNSAVED).toBe(-32011);
    expect(ErrorCode.SONG_EXISTS).toBe(-32012);
    expect(ErrorCode.OPEN_FAILED).toBe(-32013);
    expect(ErrorCode.BUSY).toBe(-32014);
  });

  test('codes are distinct', () => {
    const values = Object.values(ErrorCode);
    expect(new Set(values).size).toBe(values.length);
  });
});

describe('ControlError', () => {
  test('carries its code and message', () => {
    const e = new ControlError(ErrorCode.NO_SUCH_SONG, "no such song 'x'");
    expect(e).toBeInstanceOf(Error);
    expect(e.code).toBe(-32005);
    expect(e.message).toBe("no such song 'x'");
  });
});

describe('methods', () => {
  test('events is forwarded to the webview; screenshot is not', () => {
    const forwarded: ControlRequest['method'] = 'events';
    const named: MethodName[] = [forwarded, 'screenshot'];
    expect(named).toEqual(['events', 'screenshot']);
    // @ts-expect-error screenshot is answered in Rust
    const bad: ControlRequest['method'] = 'screenshot';
    expect(typeof bad).toBe('string');
  });
});

describe('LiveStats', () => {
  const methods = readFileSync(
    new URL('../crates/strudeland-ctl/src/methods.rs', import.meta.url),
  ).toString();
  const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
  const fields = (name: string) => {
    const body = methods.match(new RegExp(`pub struct ${name} \\{([^}]*)\\}`))![1]!;
    return [...body.matchAll(/pub ([a-z_0-9]+):/g)].map((m) => m[1]!);
  };

  test('has the fields of the Rust LiveStats', () => {
    const keys = {
      underruns: true,
      lateHaps: true,
      haps: true,
      batches: true,
      skippedHaps: true,
      renderHeadroomMs: true,
      batchDecodeUs: true,
      invokeRttMs: true,
    } satisfies Record<keyof LiveStats, true>;
    expect(fields('LiveStats').map(camel).sort()).toEqual(Object.keys(keys).sort());
  });

  test('the windowed structs match', () => {
    const decode = { p50: true, max: true } satisfies Record<
      keyof LiveStats['batchDecodeUs'],
      true
    >;
    const rtt = { p50: true, p95: true, max: true } satisfies Record<
      keyof LiveStats['invokeRttMs'],
      true
    >;
    expect(fields('BatchDecode').sort()).toEqual(Object.keys(decode).sort());
    expect(fields('InvokeRtt').sort()).toEqual(Object.keys(rtt).sort());
  });
});

describe('Levels', () => {
  const methods = readFileSync(
    new URL('../crates/strudeland-ctl/src/methods.rs', import.meta.url),
  ).toString();
  const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
  const fields = (name: string) => {
    const body = methods.match(new RegExp(`pub struct ${name} \\{([^}]*)\\}`))![1]!;
    return [...body.matchAll(/pub ([a-z_0-9]+):/g)].map((m) => m[1]!);
  };

  test('has the fields of the Rust Levels', () => {
    const keys = {
      peakDb: true,
      overs: true,
      limitDb: true,
      limiter: true,
    } satisfies Record<keyof LevelsSummary, true>;
    expect(fields('Levels').map(camel).sort()).toEqual(Object.keys(keys).sort());
  });
});

describe('METHOD_NAMES', () => {
  const methods = readFileSync(
    new URL('../crates/strudeland-ctl/src/methods.rs', import.meta.url),
  ).toString();
  const rustNames = [...methods.matchAll(/Method::[A-Za-z]+ => "([a-z.]+)"/g)].map((m) => m[1]!);

  test('matches the Rust Method enum', () => {
    expect(rustNames.length).toBeGreaterThan(0);
    expect([...rustNames].sort()).toEqual([...METHOD_NAMES].sort());
  });

  test('has no duplicates', () => {
    expect(new Set(METHOD_NAMES).size).toBe(METHOD_NAMES.length);
  });
});
