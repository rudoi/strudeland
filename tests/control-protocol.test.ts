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
import { wireErrors, wireExamples, wireResult } from './helpers/wire-examples';

const sortedKeys = (o: object) => Object.keys(o).sort();

describe('ErrorCode', () => {
  test('matches the wire examples', () => {
    expect({ ...ErrorCode } as Record<string, number>).toEqual(wireErrors);
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
  const live = wireResult('state', 'full').live;

  test("has the fields of the wire example's live", () => {
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
    expect(sortedKeys(keys)).toEqual(sortedKeys(live));
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
    expect(sortedKeys(decode)).toEqual(sortedKeys(live.batchDecodeUs));
    expect(sortedKeys(rtt)).toEqual(sortedKeys(live.invokeRttMs));
  });
});

describe('Levels', () => {
  test("has the fields of the wire example's levels", () => {
    const keys = {
      peakDb: true,
      overs: true,
      limitDb: true,
      limiter: true,
    } satisfies Record<keyof LevelsSummary, true>;
    expect(sortedKeys(keys)).toEqual(sortedKeys(wireResult('state', 'full').levels));
  });
});

describe('METHOD_NAMES', () => {
  test('matches the wire examples', () => {
    const methods = wireExamples().map((e) => e.method);
    expect(new Set(methods).size).toBe(methods.length);
    expect([...METHOD_NAMES].sort()).toEqual([...methods].sort());
  });

  test('the methods the app answers are the ones ControlRequest excludes', () => {
    const app = wireExamples()
      .filter((e) => e.answeredBy === 'app')
      .map((e) => e.method)
      .sort();
    expect(app).toEqual(['ping', 'screenshot']);
  });

  test('has no duplicates', () => {
    expect(new Set(METHOD_NAMES).size).toBe(METHOD_NAMES.length);
  });
});

describe('wireResult', () => {
  test('reads olderResults as well as results', () => {
    expect(wireResult('state', 'minimal')).toBeObject();
    expect(() => wireResult('state', 'nope')).toThrow("state has no wire result 'nope'");
  });
});
