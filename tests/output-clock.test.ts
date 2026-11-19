import { describe, expect, test } from 'bun:test';
import { createOutputClock } from '../src/lib/audio/output-clock';
import { fakeAudioGraph } from './helpers/fake-audio-graph';

describe('output clock', () => {
  test('read: the stamp as contextTime and performanceMs', () => {
    const g = fakeAudioGraph({ outputTimestamp: { contextTime: 2.5, performanceTime: 1234.5 } });
    const clock = createOutputClock({ graph: () => g.graph });
    expect(clock.read()).toEqual({ contextTime: 2.5, performanceMs: 1234.5 });
    g.setOutputTimestamp({ contextTime: 2.75, performanceTime: 1484.5 });
    expect(clock.read()).toEqual({ contextTime: 2.75, performanceMs: 1484.5 });
  });

  test('read: null for a zero, negative, missing or non-finite value', () => {
    const g = fakeAudioGraph();
    const clock = createOutputClock({ graph: () => g.graph });
    expect(clock.read()).toBeNull(); // WebKit's 0/0 before the first render quantum
    for (const stamp of [
      { contextTime: 0, performanceTime: 10 },
      { contextTime: 1, performanceTime: 0 },
      { contextTime: -1, performanceTime: 10 },
      { contextTime: Number.NaN, performanceTime: 10 },
      { contextTime: 1, performanceTime: Infinity },
      { contextTime: 1 },
      { performanceTime: 10 },
      {},
    ] as AudioTimestamp[]) {
      g.setOutputTimestamp(stamp);
      expect([stamp, clock.read()]).toEqual([stamp, null]);
    }
  });

  test('read: null without a graph, without the method, or when it throws', () => {
    expect(createOutputClock({ graph: () => null }).read()).toBeNull();
    const g = fakeAudioGraph({ outputTimestamp: 'missing' });
    expect(createOutputClock({ graph: () => g.graph }).read()).toBeNull();
    g.setOutputTimestamp('throw');
    expect(createOutputClock({ graph: () => g.graph }).read()).toBeNull();
    const throwing = createOutputClock({
      graph: () => {
        throw new Error('no context');
      },
    });
    expect(throwing.read()).toBeNull();
    expect(throwing.now()).toBeNull();
    expect(throwing.latency()).toEqual({ baseLatency: 0, outputLatency: null });
  });

  test('read: a null stamp object is null', () => {
    const g = fakeAudioGraph();
    (g.graph.context as any).getOutputTimestamp = () => null;
    expect(createOutputClock({ graph: () => g.graph }).read()).toBeNull();
  });

  test('latency: base and output latency; a missing or zero outputLatency is null', () => {
    const g = fakeAudioGraph({ baseLatency: 0.005, outputLatency: 0.02 });
    const clock = createOutputClock({ graph: () => g.graph });
    expect(clock.latency()).toEqual({ baseLatency: 0.005, outputLatency: 0.02 });
    g.setLatency({ outputLatency: undefined });
    expect(clock.latency()).toEqual({ baseLatency: 0.005, outputLatency: null });
    g.setLatency({ outputLatency: 0 });
    expect(clock.latency()).toEqual({ baseLatency: 0.005, outputLatency: null });
    g.setLatency({ outputLatency: Number.NaN, baseLatency: undefined });
    expect(clock.latency()).toEqual({ baseLatency: 0, outputLatency: null });
    g.setLatency({ baseLatency: -1 });
    expect(clock.latency()).toEqual({ baseLatency: 0, outputLatency: null });
    g.setLatency({ baseLatency: 0 });
    expect(clock.latency()).toEqual({ baseLatency: 0, outputLatency: null });
    expect(createOutputClock({ graph: () => null }).latency()).toEqual({
      baseLatency: 0,
      outputLatency: null,
    });
  });

  test('now: the context time, null without a graph', () => {
    const g = fakeAudioGraph({ currentTime: 0 });
    let graph: typeof g.graph | null = g.graph;
    const clock = createOutputClock({ graph: () => graph });
    expect(clock.now()).toBe(0); // a running context at 0 is not "no context"
    g.setTime(3.25);
    expect(clock.now()).toBe(3.25);
    graph = null;
    expect(clock.now()).toBeNull();
  });
});
