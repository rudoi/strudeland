import type { ControlApi } from '../../src/lib/control/api';
import type { ControlInfo, ControlReply, ControlRequest } from '../../src/lib/control/protocol';

/**
 * An in-memory ControlApi. `send` delivers a request to the registered `onRequest` callback and
 * resolves with what `reply` received for its id. `log` records the calls in order.
 */
export function fakeControlApi(
  info: ControlInfo = { status: 'listening', path: '/tmp/ctl.sock', detail: null },
) {
  let cb: ((req: ControlRequest) => void) | null = null;
  const waiting = new Map<number, (r: ControlReply) => void>();
  const fake = {
    info,
    attaches: 0,
    /** How many times an `onRequest` unlisten function was called. */
    unlistens: 0,
    /** Whether an `onRequest` callback is registered. */
    get listening() {
      return cb !== null;
    },
    log: [] as string[],
    replies: [] as { id: number; reply: ControlReply }[],
    /** When set, `attach` rejects with it. */
    failAttach: null as unknown,
    /** When set, `reply` rejects with it (after recording the reply). */
    failReply: null as unknown,
    send(req: ControlRequest): Promise<ControlReply> {
      if (!cb) throw new Error('no onRequest listener');
      const p = new Promise<ControlReply>((r) => waiting.set(req.id, r));
      cb(req);
      return p;
    },
  };
  const api: ControlApi = {
    async attach() {
      fake.log.push('attach');
      fake.attaches++;
      if (fake.failAttach !== null) throw fake.failAttach;
      return fake.info;
    },
    async onRequest(handler) {
      fake.log.push('onRequest');
      cb = handler;
      return () => {
        fake.log.push('unlisten');
        fake.unlistens++;
        cb = null;
      };
    },
    async reply(id, reply) {
      fake.log.push(`reply:${id}`);
      fake.replies.push({ id, reply });
      waiting.get(id)?.(reply);
      waiting.delete(id);
      if (fake.failReply !== null) throw fake.failReply;
    },
  };
  return { api, fake };
}
