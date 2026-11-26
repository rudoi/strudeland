import { describe, expect, test } from 'bun:test';
import { FakeChannel, mockTauriCore, mockTauriModule } from './helpers/tauri-core-mock';

const core = mockTauriCore();
mockTauriModule('@tauri-apps/api/event', { listen: async () => () => {} });

describe('the Tauri module mocks', () => {
  test('keeps the real exports, so modules that import them still load', async () => {
    const mocked = await import('@tauri-apps/api/core');
    expect(typeof mocked.SERIALIZE_TO_IPC_FN).toBe('string');
    expect(typeof mocked.convertFileSrc).toBe('function');
    const dpi = await import('@tauri-apps/api/dpi');
    expect(typeof dpi.LogicalSize).toBe('function');
  });

  test('a module mock keeps the real exports beside its overrides', async () => {
    const event = await import('@tauri-apps/api/event');
    expect(typeof event.TauriEvent).toBe('object');
    expect(typeof event.emit).toBe('function');
    expect(await (await event.listen('x', () => {}))()).toBeUndefined();
    const window = await import('@tauri-apps/api/window');
    expect(typeof window.getCurrentWindow).toBe('function');
  });

  test('replaces invoke, Channel and isTauri', async () => {
    const mocked = await import('@tauri-apps/api/core');
    core.answer((cmd) => `${cmd}!`);
    expect(await mocked.invoke('ping')).toBe('ping!');
    expect(core.calls).toEqual([{ cmd: 'ping', args: undefined }]);
    expect(mocked.Channel as unknown).toBe(FakeChannel);
    core.setTauri(false);
    expect(mocked.isTauri()).toBe(false);
    core.setTauri(true);
  });
});
