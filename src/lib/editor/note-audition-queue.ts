import type { NoteAuditionEdit } from './note-audition';

const SETTLE_MS = 120;

interface AuditionQueueDeps {
  eligible(): boolean;
  prepare(): Promise<boolean>;
  play(note: NoteAuditionEdit): void;
  stop(): void;
}

/** Owns the most recent typed pitch until it settles or a delimiter confirms it. */
export function createAuditionQueue(deps: AuditionQueueDeps) {
  let pending: { note: NoteAuditionEdit; ready: Promise<boolean> } | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function clearTimer() {
    clearTimeout(timer);
    timer = undefined;
  }

  async function playPending(candidate: NonNullable<typeof pending>) {
    clearTimer();
    const ready = await candidate.ready;
    if (pending !== candidate) return;
    pending = undefined;
    if (ready && deps.eligible()) deps.play(candidate.note);
  }

  return {
    edit(edit: NoteAuditionEdit | null) {
      if (edit?.kind === 'confirm') {
        const candidate = pending;
        const note = candidate?.note;
        if (
          candidate &&
          note?.from === edit.from &&
          note.to === edit.to &&
          note.pitch === edit.pitch
        ) {
          void playPending(candidate);
          return;
        }
      }
      clearTimer();
      pending = undefined;
      if (edit?.kind !== 'note' || !deps.eligible()) return;
      // Call prepare before yielding so audio can be resumed inside the typing gesture.
      const candidate = { note: edit, ready: deps.prepare() };
      pending = candidate;
      timer = setTimeout(() => void playPending(candidate), SETTLE_MS);
    },
    stop() {
      clearTimer();
      pending = undefined;
      deps.stop();
    },
  };
}
