import { describe, expect, mock, test } from 'bun:test';
import { get } from 'svelte/store';
import { createProjectActions } from '../src/lib/project/actions';

function setup() {
  const pickFolder = mock(async (): Promise<string | null> => '/music/new');
  const createProject = mock(async (_path: string, _name: string) => {});
  const session = {
    openProject: mock(async () => {}),
    openRecent: mock(async (_path: string) => {}),
    newProject: mock(async (askName: (dir: string) => Promise<string | null>) => {
      const dir = await pickFolder();
      if (dir === null) return;
      const name = await askName(dir);
      if (name !== null) await createProject(dir, name);
    }),
  };
  return { session, pickFolder, createProject, actions: createProjectActions(session) };
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
    expect(session.newProject).not.toHaveBeenCalled();
    expect(session.openRecent).not.toHaveBeenCalled();

    open.resolve();
    await first;
    await actions.openRecent('/music/recent');
    expect(session.openRecent).toHaveBeenCalledWith('/music/recent');
  });

  test('blocks other project commands while naming a project, and cancel creates nothing', async () => {
    const { session, actions, createProject } = setup();
    const creating = actions.run('newProject');
    await Promise.resolve();
    expect(get(actions.folder)).toBe('/music/new');

    await actions.run('newProject');
    await actions.run('openProject');
    actions.create('  ');
    expect(session.newProject).toHaveBeenCalledTimes(1);
    expect(session.openProject).not.toHaveBeenCalled();
    expect(get(actions.folder)).toBe('/music/new');

    actions.cancel();
    await creating;
    expect(get(actions.folder)).toBeNull();
    expect(createProject).not.toHaveBeenCalled();
    await actions.run('openProject');
    expect(session.openProject).toHaveBeenCalledTimes(1);
  });

  test('a cancelled folder picker permits another try and creation uses the trimmed name', async () => {
    const { session, actions, pickFolder, createProject } = setup();
    pickFolder.mockResolvedValueOnce(null);
    await actions.run('newProject');
    expect(get(actions.folder)).toBeNull();
    actions.create('ignored');
    expect(createProject).not.toHaveBeenCalled();

    const hold = Promise.withResolvers<void>();
    createProject.mockImplementation(() => hold.promise);
    const creating = actions.run('newProject');
    await Promise.resolve();
    actions.create('  New music  ');
    actions.create('duplicate');
    await actions.run('openProject');
    expect(session.openProject).not.toHaveBeenCalled();
    expect(get(actions.folder)).toBeNull();
    hold.resolve();
    await creating;
    expect(createProject.mock.calls).toEqual([['/music/new', 'New music']]);
  });

  test('teardown cancels a pending name and ignores a folder picked after teardown', async () => {
    const first = setup();
    const naming = first.actions.run('newProject');
    await Promise.resolve();
    expect(get(first.actions.folder)).toBe('/music/new');
    first.actions.destroy();
    await naming;
    expect(get(first.actions.folder)).toBeNull();
    expect(first.createProject).not.toHaveBeenCalled();

    const second = setup();
    const folder = Promise.withResolvers<string | null>();
    second.pickFolder.mockImplementation(() => folder.promise);
    const picking = second.actions.run('newProject');
    second.actions.destroy();
    folder.resolve('/late');
    await picking;
    expect(get(second.actions.folder)).toBeNull();
    expect(second.createProject).not.toHaveBeenCalled();
  });
});
