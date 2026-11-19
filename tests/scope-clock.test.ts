import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createScopeClock, SCOPE_CLOCK_KEY, SCOPE_BUFFER_SIZE } =
  await import('../src/lib/scope/clock');

/** A fake rAF: frames run only when the test calls `frame()`. */
function setup(readResult: boolean | (() => boolean) = true) {
  const pending = new Map<number, (t: number) => void>();
  let next = 0;
  let cancels = 0;
  const reads: Float32Array[] = [];
  const clock = createScopeClock(
    (buf) => {
      reads.push(buf);
      return typeof readResult === 'function' ? readResult() : readResult;
    },
    (cb) => (pending.set(++next, cb), next),
    (h) => {
      cancels++;
      pending.delete(h);
    },
  );
  const frame = () => {
    const cbs = [...pending.values()];
    pending.clear();
    cbs.forEach((cb) => cb(0));
  };
  return { clock, reads, frame, pending, cancels: () => cancels };
}

const spy = () => {
  const calls: (Float32Array | null)[] = [];
  return { calls, fn: (b: Float32Array | null) => void calls.push(b) };
};

const setHidden = (hidden: boolean) => {
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  document.dispatchEvent(new Event('visibilitychange'));
};

describe('createScopeClock', () => {
  test('SCOPE_CLOCK_KEY is a symbol', () => {
    expect(typeof SCOPE_CLOCK_KEY).toBe('symbol');
  });

  test('one read per frame for any number of subscribers, all given the same buffer', () => {
    const s = setup();
    const a = spy();
    const b = spy();
    const c = spy();
    s.clock.subscribe(a.fn);
    s.clock.subscribe(b.fn);
    s.clock.subscribe(c.fn);
    s.clock.setRunning(true);
    s.frame();
    s.frame();
    s.frame();
    expect(s.reads).toHaveLength(3);
    expect(s.reads[0]).toHaveLength(SCOPE_BUFFER_SIZE);
    expect(new Set(s.reads).size).toBe(1); // allocated once
    for (const x of [a, b, c]) {
      expect(x.calls).toHaveLength(3);
      expect(x.calls.every((v) => v === s.reads[0])).toBe(true);
    }
    expect(s.pending.size).toBe(1); // never more than one frame queued
  });

  test('a new subscriber is not called on subscribe', () => {
    const s = setup();
    s.clock.setRunning(true);
    const a = spy();
    s.clock.subscribe(a.fn);
    expect(a.calls).toEqual([]);
  });

  test('no frames without subscribers', () => {
    const s = setup();
    s.clock.setRunning(true);
    expect(s.pending.size).toBe(0);
    s.frame();
    expect(s.reads).toHaveLength(0);
  });

  test('no frames while not running', () => {
    const s = setup();
    const a = spy();
    s.clock.subscribe(a.fn);
    expect(s.pending.size).toBe(0);
    s.frame();
    expect(s.reads).toHaveLength(0);
    expect(a.calls).toEqual([]);
  });

  test('a single null on stop', () => {
    const s = setup();
    const a = spy();
    s.clock.subscribe(a.fn);
    s.clock.setRunning(true);
    s.frame();
    s.clock.setRunning(false);
    s.clock.setRunning(false);
    s.frame();
    expect(a.calls).toEqual([s.reads[0]!, null]);
    expect(s.reads).toHaveLength(1);
    expect(s.pending.size).toBe(0);
    expect(s.cancels()).toBe(1);
  });

  test('a stop before the first frame still rests subscribers once', () => {
    const s = setup();
    const a = spy();
    s.clock.subscribe(a.fn);
    s.clock.setRunning(true);
    s.clock.setRunning(false);
    expect(a.calls).toEqual([null]);
    expect(s.reads).toHaveLength(0);
  });

  test('no null when nobody subscribes', () => {
    const s = setup();
    const a = spy();
    const unsub = s.clock.subscribe(a.fn);
    s.clock.setRunning(true);
    s.frame();
    unsub();
    unsub(); // idempotent
    expect(a.calls).toHaveLength(1); // the last unsubscribe stops the loop without calling it
    expect(s.pending.size).toBe(0);
    s.clock.setRunning(false);
    expect(a.calls).toHaveLength(1);
  });

  test('an unsubscribe that leaves others keeps the loop going', () => {
    const s = setup();
    const a = spy();
    const b = spy();
    const unsubA = s.clock.subscribe(a.fn);
    s.clock.subscribe(b.fn);
    s.clock.setRunning(true);
    s.frame();
    unsubA();
    s.frame();
    expect(a.calls).toHaveLength(1);
    expect(b.calls).toHaveLength(2);
    expect(s.reads).toHaveLength(2);
  });

  test('read returning false sends null for that frame', () => {
    let ok = true;
    const s = setup(() => ok);
    const a = spy();
    s.clock.subscribe(a.fn);
    s.clock.setRunning(true);
    s.frame();
    ok = false;
    s.frame();
    ok = true;
    s.frame();
    expect(a.calls).toEqual([s.reads[0]!, null, s.reads[0]!]);
  });

  test('running is true for every loop call (with or without samples), false for the rest call', () => {
    let ok = false;
    const { clock, frame } = setup(() => ok);
    const seen: [boolean, boolean][] = [];
    clock.subscribe((b, running) => void seen.push([b !== null, running]));
    clock.setRunning(true);
    frame();
    ok = true;
    frame();
    clock.setRunning(false);
    expect(seen).toEqual([
      [false, true],
      [true, true],
      [false, false],
    ]);
  });

  test('a subscriber that stops the clock inside a frame gets one null and no further frames', () => {
    const s = setup();
    const calls: (Float32Array | null)[] = [];
    s.clock.subscribe((b) => {
      calls.push(b);
      if (b) s.clock.setRunning(false);
    });
    s.clock.setRunning(true);
    s.frame();
    s.frame();
    expect(calls).toEqual([s.reads[0]!, null]);
    expect(s.pending.size).toBe(0);
  });

  test('hidden pauses with a null, visible resumes', () => {
    const s = setup();
    const a = spy();
    s.clock.subscribe(a.fn);
    s.clock.setRunning(true);
    s.frame();
    try {
      setHidden(true);
      expect(a.calls).toEqual([s.reads[0]!, null]);
      expect(s.pending.size).toBe(0);
      s.frame();
      expect(s.reads).toHaveLength(1);
      setHidden(true); // a second hidden event does nothing
      expect(a.calls).toHaveLength(2);
      setHidden(false);
      expect(s.pending.size).toBe(1);
      s.frame();
      expect(s.reads).toHaveLength(2);
      expect(a.calls).toHaveLength(3);
    } finally {
      setHidden(false);
    }
  });

  test('becoming visible while stopped does not start the loop', () => {
    const s = setup();
    s.clock.subscribe(spy().fn);
    try {
      setHidden(true);
      setHidden(false);
      expect(s.pending.size).toBe(0);
    } finally {
      setHidden(false);
    }
  });

  test('dispose removes the visibilitychange listener, once', () => {
    const removed: unknown[] = [];
    const real = document.removeEventListener.bind(document);
    document.removeEventListener = ((type: string, fn: EventListener) => {
      if (type === 'visibilitychange') removed.push(fn);
      real(type, fn);
    }) as typeof document.removeEventListener;
    const added: unknown[] = [];
    const realAdd = document.addEventListener.bind(document);
    document.addEventListener = ((type: string, fn: EventListener) => {
      if (type === 'visibilitychange') added.push(fn);
      realAdd(type, fn);
    }) as typeof document.addEventListener;
    try {
      const s = setup();
      expect(added).toHaveLength(1);
      s.clock.dispose();
      s.clock.dispose(); // idempotent
      expect(removed).toEqual(added);
    } finally {
      document.removeEventListener = real;
      document.addEventListener = realAdd;
    }
  });

  test('dispose stops the loop without calling subscribers, and nothing restarts it', () => {
    const s = setup();
    const a = spy();
    s.clock.subscribe(a.fn);
    s.clock.setRunning(true);
    s.frame();
    s.clock.dispose();
    expect(s.pending.size).toBe(0);
    expect(s.cancels()).toBe(1);
    expect(a.calls).toEqual([s.reads[0]!]); // no rest null: the subscribers are dropped
    try {
      setHidden(true);
      setHidden(false);
    } finally {
      setHidden(false);
    }
    s.clock.setRunning(true);
    const b = spy();
    const unsub = s.clock.subscribe(b.fn);
    unsub();
    s.frame();
    expect(s.pending.size).toBe(0);
    expect(s.reads).toHaveLength(1);
    expect(a.calls).toHaveLength(1);
    expect(b.calls).toEqual([]);
  });

  test('a clock without injected raf looks it up only when a frame is needed', () => {
    const saved = globalThis.requestAnimationFrame;
    let calls = 0;
    // @ts-expect-error: removed to prove creation does not need it
    delete globalThis.requestAnimationFrame;
    try {
      const clock = createScopeClock(() => true);
      clock.setRunning(true);
      globalThis.requestAnimationFrame = (() => (calls++, 1)) as typeof requestAnimationFrame;
      clock.subscribe(() => {});
      expect(calls).toBe(1);
    } finally {
      globalThis.requestAnimationFrame = saved;
    }
  });
});
