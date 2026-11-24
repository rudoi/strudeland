import { describe, expect, test } from 'bun:test';
import { get } from 'svelte/store';
import { createInstrumentPreview } from '../src/lib/audio/instrument-preview';
import type { InstrumentBrowserApi, InstrumentEntry } from '../src/lib/project/api';
import { createInstrumentBrowser } from '../src/lib/project/instrument-browser';
import { encodeWav } from '../src/lib/render/wav';
import { fakeAudioGraph } from './helpers/fake-audio-graph';
import { deferred } from './helpers/fake-project-api';

const bass = { id: 'bass', fingerprint: 'library-version' };
const keys = { id: 'keys', fingerprint: 'other-version' };
const definition = {
  schema: 1,
  parameters: { amount: 1 },
  voices: [{ name: 'bass', chain: [{ control: 'gain', value: 0.5 }] }],
} as const;

function catalog() {
  let pins: InstrumentEntry[] = [];
  const writes: unknown[][] = [];
  const reads: unknown[] = [];
  const api: InstrumentBrowserApi = {
    libraryEntries: async () => [bass, keys],
    projectEntries: async (root) => {
      expect(root).toBe('/project');
      return pins;
    },
    instrumentDefinition: async (version) => {
      reads.push(version);
      return definition;
    },
    instrumentPreview: async () => new Uint8Array(),
    writeInstrumentPin: async (...args) => {
      writes.push(args);
      pins = [bass];
      return bass;
    },
  };
  let stops = 0;
  const browser = createInstrumentBrowser(api, '/project', () => stops++);
  return {
    api,
    browser,
    state: () => get(browser.store),
    reads,
    writes,
    stops: () => stops,
    setPins: (entries: InstrumentEntry[]) => (pins = entries),
  };
}

describe('instrument browser', () => {
  test('pins and updates the displayed versions, then reads the project copy and preserves the filter', async () => {
    const h = catalog();
    await h.browser.refresh();
    h.browser.filter(' BAS ');
    expect(h.state().rows).toEqual([bass]);
    await h.browser.writePin();
    expect(h.writes).toEqual([['/project', 'bass', bass.fingerprint, null]]);
    expect(h.state().source).toBe('project');
    expect(h.state().search).toBe(' BAS ');
    expect(h.reads.at(-1)).toEqual({ ...bass, root: '/project' });
    h.setPins([{ ...bass, fingerprint: 'old' }]);
    await h.browser.refresh();
    await h.browser.writePin();
    expect(h.writes.at(-1)).toEqual(['/project', 'bass', bass.fingerprint, 'old']);
    await h.browser.writePin();
    expect(h.writes).toHaveLength(2);
  });

  test('a failed global list leaves project-only pins readable; refresh retries a stale definition', async () => {
    const h = catalog();
    h.setPins([keys]);
    h.api.libraryEntries = async () => {
      throw Error('library unavailable');
    };
    h.api.instrumentDefinition = async () => {
      throw Error('changed');
    };
    await h.browser.refresh();
    expect(h.state().lists.library?.error).toBe('library unavailable');
    h.browser.show('project');
    await Promise.resolve();
    expect(h.state().readError).toBe('changed');
    h.api.instrumentDefinition = async () => definition;
    await h.browser.refresh();
    expect(h.state().definition).toEqual(definition);
    expect(h.state().readError).toBe('');
    expect(h.state().version).toEqual({ ...keys, root: '/project' });
  });

  test('late details and writes cannot change the next selection or restart after close', async () => {
    const h = catalog();
    const hold = deferred();
    h.api.instrumentDefinition = async ({ id }) => {
      if (id === bass.id) {
        await hold.promise;
        throw Error('superseded');
      }
      return definition;
    };
    await h.browser.refresh();
    h.api.writeInstrumentPin = async () => {
      await hold.promise;
      h.setPins([bass]);
      return bass;
    };
    const writing = h.browser.writePin();
    h.browser.show('library', keys.id);
    hold.resolve();
    await writing;
    expect(h.state().version).toEqual({ ...keys, root: null });
    expect(h.state().definition).toEqual(definition);
    expect(h.state().readError).toBe('');
    const before = h.stops();
    h.browser.show('library', bass.id);
    h.browser.close();
    await Promise.resolve();
    await Promise.resolve();
    expect(h.stops()).toBeGreaterThan(before);
    expect(h.state().readError).toBe('');
  });

  test('stale update reports the error beside the unchanged project pin and can be retried', async () => {
    const h = catalog();
    h.setPins([{ ...bass, fingerprint: 'old' }]);
    h.api.writeInstrumentPin = async () => {
      throw Error('pin changed');
    };
    await h.browser.refresh();
    h.browser.show('project');
    await h.browser.writePin();
    expect(h.state().writeError).toBe('pin changed');
    expect(h.state().version?.fingerprint).toBe('old');
    h.setPins([{ ...bass, fingerprint: 'newer' }]);
    await h.browser.refresh();
    expect(h.state().writeError).toBe('');
    expect(h.state().version?.fingerprint).toBe('newer');
  });
});

const wav = encodeWav([new Float32Array([0, 0.4]), new Float32Array([0.5, 0])], 48000, 'f32');

describe('instrument preview on the existing audio graph', () => {
  test('prepares audio on the gesture, plays both channels and disconnects on end or stop', async () => {
    const fake = fakeAudioGraph();
    const calls: string[] = [];
    const player = createInstrumentPreview({
      graph: () => fake.graph,
      eligible: () => true,
      prepare: async () => {
        calls.push('prepare');
        return true;
      },
      render: async (version) => {
        calls.push(version.fingerprint);
        return wav;
      },
    });
    const version = { ...bass, root: null };
    const playing = player.play(version);
    expect(calls).toEqual(['prepare']);
    await playing;
    expect(calls).toEqual(['prepare', bass.fingerprint]);
    expect(get(player.store).status).toBe('playing');
    expect(fake.buffers[0].data[1]).toEqual(new Float32Array([0.5, 0]));
    await player.play({ ...version, root: '/project' });
    expect(fake.sources()[0].disconnected).toBe(true);
    expect(fake.sources()[0].stops).toBe(1);
    fake.sources()[1].end();
    expect(get(player.store).status).toBe('idle');
    expect(fake.sources().every((node) => node.disconnected)).toBe(true);
  });

  test('interrupting a render or pending audio preparation never connects a late source', async () => {
    const fake = fakeAudioGraph();
    let eligible = true;
    const preparing = deferred();
    const rendering = deferred();
    let requests = 0;
    const player = createInstrumentPreview({
      graph: () => fake.graph,
      eligible: () => eligible,
      prepare: async () => {
        await preparing.promise;
        return true;
      },
      render: async () => {
        requests++;
        await rendering.promise;
        return wav;
      },
    });
    const version = { ...bass, root: null };
    const first = player.play(version);
    player.stop();
    preparing.resolve();
    await first;
    expect(requests).toBe(0);
    const second = player.play(version);
    await Promise.resolve();
    await Promise.resolve();
    expect(requests).toBe(1);
    player.stop();
    rendering.resolve();
    await second;
    expect(get(player.store).status).toBe('idle');
    eligible = false;
    await player.play(version);
    expect(get(player.store).status).toBe('error');
    expect(fake.sources()).toHaveLength(0);
  });

  test('a render failure is visible and the next preview can recover', async () => {
    const fake = fakeAudioGraph();
    let fail = true;
    const player = createInstrumentPreview({
      graph: () => fake.graph,
      eligible: () => true,
      prepare: async () => true,
      render: async () => {
        if (fail) throw Error('unsupported sound');
        return wav;
      },
    });
    await player.play({ ...bass, root: null });
    expect(get(player.store)).toEqual({ status: 'error', message: 'unsupported sound' });
    fail = false;
    await player.play({ ...bass, root: null });
    expect(get(player.store).status).toBe('playing');
  });
});
