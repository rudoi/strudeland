import { expect, mock, test } from 'bun:test';
import type { NoteAuditionEdit } from '../src/lib/editor/note-audition';
import { createAuditionQueue } from '../src/lib/editor/note-audition-queue';

const note: NoteAuditionEdit = {
  kind: 'note',
  from: 6,
  to: 8,
  pitch: 55,
  sound: 'triangle',
  controls: {},
};
const confirm: NoteAuditionEdit = { ...note, kind: 'confirm' };

test('a delimiter only confirms the pending same note and plays it once', async () => {
  const play = mock();
  const prepare = mock(async () => true);
  const queue = createAuditionQueue({ play, prepare, eligible: () => true, stop: mock() });

  queue.edit(confirm);
  expect(prepare).not.toHaveBeenCalled();
  queue.edit(note);
  expect(prepare).toHaveBeenCalledTimes(1);
  expect(play).not.toHaveBeenCalled();
  queue.edit(confirm);
  queue.edit(confirm);
  await Promise.resolve();
  expect(play).toHaveBeenCalledTimes(1);
  expect(play).toHaveBeenCalledWith(note);
  queue.stop();
});

test('cancelled audio preparation and loss of eligibility never play a stale note', async () => {
  const play = mock();
  const stop = mock();
  let unlock!: (ready: boolean) => void;
  let eligible = true;
  const prepare = () => new Promise<boolean>((resolve) => (unlock = resolve));
  const queue = createAuditionQueue({ play, prepare, eligible: () => eligible, stop });

  queue.edit(note);
  queue.edit(confirm);
  queue.stop();
  unlock(true);
  await Promise.resolve();
  expect(play).not.toHaveBeenCalled();
  expect(stop).toHaveBeenCalledTimes(1);

  queue.edit(note);
  queue.edit(confirm);
  eligible = false;
  unlock(true);
  await Promise.resolve();
  expect(play).not.toHaveBeenCalled();
});

test('the newest typed pitch plays after settling without a delimiter', async () => {
  const play = mock();
  const queue = createAuditionQueue({
    play,
    stop: mock(),
    prepare: async () => true,
    eligible: () => true,
  });
  const next = { ...note, pitch: 67, to: 9 };
  queue.edit(note);
  queue.edit(next);
  await Bun.sleep(170);
  expect(play.mock.calls).toEqual([[next]]);
  queue.stop();
});
