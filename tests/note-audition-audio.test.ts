import { expect, mock, test } from 'bun:test';
import { registerSound, soundMap } from 'superdough';
import { captureAuditionSynths, createNoteAuditionPlayer } from '../src/lib/audio/note-audition';
import { fakeAudioGraph } from './helpers/fake-audio-graph';

const note = { pitch: 55, sound: 'supersaw', controls: {} };

function synth(context: AudioContext) {
  return { node: context.createGain(), stop: mock((_t: number) => {}) };
}

test('routes a bounded synth envelope and gain to the master, then disconnects on natural end', async () => {
  const audio = fakeAudioGraph({ currentTime: 3 });
  const handle = synth(audio.graph.context);
  const trigger = mock(
    (_when: number, _value: Record<string, number>, _ended: () => void) => handle,
  );
  const lookup = mock((_name: string) => trigger);
  const player = createNoteAuditionPlayer({ graph: () => audio.graph, lookup });
  await player.play({
    pitch: 0,
    sound: 'SUPERSAW',
    controls: { attack: 2, decay: -5, sustain: 12, release: 9, gain: 5, duration: 10, freq: 900 },
  });
  expect(lookup).toHaveBeenCalledWith('supersaw');
  const [when, value, ended] = trigger.mock.calls[0];
  expect(when).toBe(3);
  expect(value).toEqual({
    freq: expect.closeTo(8.1758, 4),
    duration: 0.16,
    attack: 0.16,
    decay: 0,
    sustain: 1,
    release: 0.12,
  });
  const [source, output] = audio.gains();
  expect(source.connects).toEqual([output]);
  expect(audio.destination.connects).toEqual([output]);
  expect((output as unknown as GainNode).gain.value).toBe(0.5);
  ended();
  player.stop();
  expect(output.disconnected).toBe(true);
  expect(handle.stop).not.toHaveBeenCalled();
});

test('a replacement cuts the sounding voice and an old ended event cannot cancel the replacement', async () => {
  const audio = fakeAudioGraph();
  const triggers: (() => void)[] = [];
  const handles: ReturnType<typeof synth>[] = [];
  const player = createNoteAuditionPlayer({
    graph: () => audio.graph,
    lookup: () => (_when, _value, ended) => {
      triggers.push(ended);
      const handle = synth(audio.graph.context);
      handles.push(handle);
      return handle;
    },
  });
  const clearFirst = mock();
  const clearSecond = mock();
  await player.play(note, () => clearFirst);
  const firstOutput = audio.gains()[1];
  audio.setTime(4);
  await player.play({ ...note, controls: { gain: 0.25 } }, () => clearSecond);
  const secondOutput = audio.gains()[3];
  expect((secondOutput as unknown as GainNode).gain.value).toBe(0.125);
  expect(firstOutput.disconnected).toBe(true);
  expect(handles[0].stop).toHaveBeenCalledWith(4);
  triggers[0]();
  expect(secondOutput.disconnected).toBe(false);
  expect(clearFirst).toHaveBeenCalledTimes(1);
  expect(clearSecond).not.toHaveBeenCalled();
  player.stop();
  player.stop();
  expect(secondOutput.disconnected).toBe(true);
  expect(handles[1].stop).toHaveBeenCalledTimes(1);
  expect(clearSecond).toHaveBeenCalledTimes(1);
});

for (const change of ['stop', 'supersede', 'rebuild graph'] as const) {
  test(`a pending trigger cannot connect after ${change}`, async () => {
    const audio = fakeAudioGraph();
    let graph = audio.graph;
    const first = synth(graph.context);
    const second = synth(graph.context);
    let resolve!: (handle: typeof first) => void;
    const pending = new Promise<typeof first>((accept) => (resolve = accept));
    const player = createNoteAuditionPlayer({
      graph: () => graph,
      lookup: (name) => () => (name === 'sine' ? second : pending),
    });
    const onSounding = mock();
    const playing = player.play(note, onSounding);
    if (change === 'stop') player.stop();
    if (change === 'supersede') await player.play({ ...note, sound: 'sine' });
    if (change === 'rebuild graph') graph = fakeAudioGraph().graph;
    resolve(first);
    await playing;
    expect(audio.gains()[0].connects).toEqual([]);
    expect(first.stop).toHaveBeenCalledTimes(1);
    expect(onSounding).not.toHaveBeenCalled();
    expect(audio.destination.connects).toHaveLength(change === 'supersede' ? 1 : 0);
    player.stop();
    if (change === 'supersede') expect(second.stop).toHaveBeenCalledTimes(1);
  });
}

test('invalid pitches, unsupported names, absent graph and unavailable worklets stay silent', async () => {
  const audio = fakeAudioGraph();
  const trigger = mock(() => Promise.reject(new Error('worklet unavailable')));
  const lookup = mock(() => trigger);
  let running = true;
  const player = createNoteAuditionPlayer({ graph: () => (running ? audio.graph : null), lookup });
  for (const pitch of [-1, 128, NaN, 60.5]) await player.play({ ...note, pitch });
  for (const sound of ['bd', 'bus', 'user', 'one']) await player.play({ ...note, sound });
  expect(lookup).not.toHaveBeenCalled();
  running = false;
  await player.play(note);
  expect(trigger).not.toHaveBeenCalled();
  running = true;
  await player.play(note);
  expect(trigger).toHaveBeenCalledTimes(1);
  expect(audio.destination.connects).toHaveLength(0);
});

test('shows the outline only for an audible connected voice and clears it on natural end', async () => {
  const audio = fakeAudioGraph();
  const trigger = mock((_when, _value, _ended: () => void) => synth(audio.graph.context));
  const player = createNoteAuditionPlayer({ graph: () => audio.graph, lookup: () => trigger });
  const clear = mock();
  const onSounding = mock(() => clear);
  await player.play({ ...note, controls: { gain: 0 } }, onSounding);
  expect(onSounding).not.toHaveBeenCalled();
  await player.play(note, onSounding);
  expect(onSounding).toHaveBeenCalledTimes(1);
  expect(clear).not.toHaveBeenCalled();
  trigger.mock.calls[1][2]();
  player.stop();
  expect(clear).toHaveBeenCalledTimes(1);
});

test('a registered built-in name that a song subsequently redefines stays silent', async () => {
  const previousSounds = soundMap.get();
  try {
    const audio = fakeAudioGraph();
    const stock = mock(() => synth(audio.graph.context));
    const custom = mock(() => synth(audio.graph.context));
    registerSound('sine', stock, { type: 'synth', prebake: true });
    captureAuditionSynths();
    const player = createNoteAuditionPlayer({ graph: () => audio.graph });
    await player.play({ ...note, sound: 'sine' });
    registerSound('sine', custom, { type: 'synth', prebake: true });
    await player.play({ ...note, sound: 'sine' });
    expect(stock).toHaveBeenCalledTimes(1);
    expect(custom).not.toHaveBeenCalled();
  } finally {
    soundMap.set(previousSounds);
    captureAuditionSynths();
  }
});
