/** Saves an object's own properties and puts them back: for tests that import a module whose
 *  side effects replace `Pattern.prototype` methods (`bun test` shares modules across files).
 *  Import every real module that adds methods (core, mini, tonal, draw, transpiler, and webaudio or
 *  codemirror when the test loads them) before taking the snapshot: a key added after it is
 *  deleted on restore, and a cached module never adds it again. */
export function snapshotOwnProperties(target: object): () => void {
  const saved = Object.getOwnPropertyDescriptors(target);
  return () => {
    for (const key of Reflect.ownKeys(target)) {
      if (!(key in saved)) delete (target as any)[key];
    }
    for (const key of Reflect.ownKeys(saved)) {
      Object.defineProperty(target, key, saved[key as any]!);
    }
  };
}

/** The real modules the host prebake loads (they add `Pattern.prototype` methods at import). */
export async function importHostRealModules(): Promise<void> {
  await Promise.all([
    import('@strudel/core'),
    import('@strudel/mini'),
    import('@strudel/tonal'),
    import('@strudel/draw'),
    import('@strudel/transpiler'),
  ]);
}
