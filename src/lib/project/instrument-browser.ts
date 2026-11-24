import { writable } from 'svelte/store';
import { errorMessage } from '../errors';
import type { InstrumentSnapshot } from '../strudel/instruments';
import type { InstrumentBrowserApi, InstrumentEntry, InstrumentVersion } from './api';

type Source = 'library' | 'project';
type Listing = { entries: InstrumentEntry[]; error?: string } | null;

interface BrowserState {
  source: Source;
  search: string;
  lists: Record<Source, Listing>;
  rows: InstrumentEntry[];
  version: InstrumentVersion | null;
  definition: InstrumentSnapshot[string] | null;
  readError: string;
  writing: boolean;
  writeError: string;
}

/** All requests belong to this opening of the dialog, for one captured project root. */
export function createInstrumentBrowser(api: InstrumentBrowserApi, root: string, stop: () => void) {
  const state: BrowserState = {
    source: 'library',
    search: '',
    lists: { library: null, project: null },
    rows: [],
    version: null,
    definition: null,
    readError: '',
    writing: false,
    writeError: '',
  };
  const store = writable(state);
  let selectedId = '';
  let listRequest = 0;
  let detailRequest = 0;
  let closed = false;

  function reconcile(reload = false) {
    const list = state.lists[state.source];
    const term = state.search.trim().toLowerCase();
    state.rows = (list?.entries ?? []).filter((row) => row.id.includes(term));
    const entry = state.rows.find((row) => row.id === selectedId) ?? state.rows[0];
    const sourceRoot = state.source === 'project' ? root : null;
    const previous = state.version;
    if (
      !reload &&
      previous?.id === entry?.id &&
      previous?.fingerprint === entry?.fingerprint &&
      (!entry || previous?.root === sourceRoot)
    ) {
      store.set(state);
      return;
    }
    const request = ++detailRequest;
    stop();
    state.version = entry ? { ...entry, root: sourceRoot } : null;
    state.definition = null;
    state.readError = '';
    state.writeError = '';
    if (state.version) {
      selectedId = state.version.id;
      void api.instrumentDefinition(state.version).then(
        (definition) => {
          if (request !== detailRequest) return;
          state.definition = definition;
          store.set(state);
        },
        (e) => {
          if (request !== detailRequest) return;
          state.readError = errorMessage(e);
          store.set(state);
        },
      );
    }
    store.set(state);
  }

  async function refresh() {
    const request = ++listRequest;
    async function load(source: Source) {
      let list: Listing;
      try {
        list = {
          entries: await (source === 'library' ? api.libraryEntries() : api.projectEntries(root)),
        };
      } catch (e) {
        list = { entries: [], error: errorMessage(e) };
      }
      if (closed || request !== listRequest) return;
      state.lists[source] = list;
      reconcile(source === state.source && !!state.readError);
    }
    await Promise.all([load('library'), load('project')]);
  }

  function show(source: Source, id = selectedId) {
    state.source = source;
    selectedId = id;
    reconcile();
  }

  async function writePin() {
    const version = state.version;
    const { library, project } = state.lists;
    if (closed || state.writing || !version || !project || project.error) return;
    const from = library?.entries.find((row) => row.id === version.id);
    const pin = project.entries.find((row) => row.id === version.id);
    if (!from || from.fingerprint === pin?.fingerprint) return;
    const detail = detailRequest;
    state.writing = true;
    state.writeError = '';
    store.set(state);
    try {
      const result = await api.writeInstrumentPin(
        root,
        version.id,
        from.fingerprint,
        pin?.fingerprint ?? null,
      );
      if (closed) return;
      const follow = detail === detailRequest;
      // Show the published version immediately. Refresh both lists for other changes.
      state.lists.project = {
        entries: [...project.entries.filter((row) => row.id !== result.id), result],
      };
      if (follow) show('project', result.id);
      await refresh();
    } catch (e) {
      if (!closed && detail === detailRequest) state.writeError = errorMessage(e);
    } finally {
      if (!closed) {
        state.writing = false;
        store.set(state);
      }
    }
  }

  return {
    store: { subscribe: store.subscribe },
    refresh,
    show,
    writePin,
    filter(search: string) {
      state.search = search;
      reconcile();
    },
    close() {
      closed = true;
      detailRequest++;
      stop();
    },
  };
}
