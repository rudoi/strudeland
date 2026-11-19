import type { EvaluateOptions } from '../strudel/runtime';
import { buildStructure, type SongStructure } from '../strudel/structure';
import type { Meta } from './meta';

export const EMPTY_STRUCTURE = buildStructure({});

/** The structure a parsed header describes; the defaults when it has none or is invalid. */
export function staticStructure(meta: Meta): SongStructure {
  try {
    return buildStructure(meta);
  } catch {
    return EMPTY_STRUCTURE; // parseMeta validated the fields, so this does not happen
  }
}

/** The options `runtime.evaluate` gets for a song with this header (shared by app and renderer). */
export function evaluateOptions(meta: Meta): EvaluateOptions {
  const s = staticStructure(meta);
  return {
    cps: s.cps,
    meter: s.meter,
    seed: meta.seed,
    engine: meta.engine === 'rust' ? 'rust' : 'superdough',
  };
}
