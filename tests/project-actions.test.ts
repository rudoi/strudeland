import { describe, expect, mock, test } from 'bun:test';
import { get } from 'svelte/store';
import { createProjectActions } from '../src/lib/project/actions';

function setup() {
  const session = {
    openProject: mock(async () => {}),
    openRecent: mock(async (_path: string) => {}),
    pickNewProjectFolder: mock(async (): Promise<string | null> => '/music/new'),
    createProject: mock(async (_path: string, _name: string) => {}),
  };
  return { session, actions: createProjectActions(session) };
}

describe('project actions', () => {
  test('opens a project once while the session is busy with the folder picker or switch', async () => {
    const { session, actions } = setup();
    const open = Promise.withResolvers<void>();
    session.openProject.mockImplementation(() => open.promise);

    const first = actions.run('openProject');
    await actions.run('openProject');
    await actions.run('newProject');
    await actions.openRecent('/music/recent');
    expect(session.openProject).toHaveBeenCalledTimes(1);
    expect(session.pickNewProjectFolder).not.toHaveBeenCalled();
    expect(session.openRecent).not.toHaveBeenCalled();

    open.resolve();
    await first;
    await actions.openRecent('/music/recent');
    expect(session.openRecent).toHaveBeenCalledWith('/music/recent');
  });

  test('blocks other project commands while naming a project, and cancel creates nothing', async () => {
    const { session, actions } = setup();
    await actions.run('newProject');
    expect(get(actions.folder)).toBe('/music/new');

    await actions.run('newProject');
    await actions.run('openProject');
    await actions.create('  ');
    expect(session.pickNewProjectFolder).toHaveBeenCalledTimes(1);
    expect(session.openProject).not.toHaveBeenCalled();
    expect(get(actions.folder)).toBe('/music/new');

    actions.cancel();
    expect(get(actions.folder)).toBeNull();
    expect(session.createProject).not.toHaveBeenCalled();
    await actions.run('openProject');
    expect(session.openProject).toHaveBeenCalledTimes(1);
  });

  test('a cancelled folder picker permits another try and creation uses the trimmed name', async () => {
    const { session, actions } = setup();
    session.pickNewProjectFolder.mockResolvedValueOnce(null);
    await actions.run('newProject');
    expect(get(actions.folder)).toBeNull();
    await actions.create('ignored');
    expect(session.createProject).not.toHaveBeenCalled();

    await actions.run('newProject');
    const creation = Promise.withResolvers<void>();
    session.createProject.mockImplementation(() => creation.promise);
    const first = actions.create('  New music  ');
    await actions.run('openProject');
    expect(session.openProject).not.toHaveBeenCalled();
    expect(session.createProject).toHaveBeenCalledWith('/music/new', 'New music');
    expect(get(actions.folder)).toBeNull();
    creation.resolve();
    await first;
  });
});
