// bun test preload (bunfig.toml): resolves Vite `?audioworklet` imports to a stub module, since bun
// has no equivalent of vite-plugin-bundle-audioworklet.
import { fileURLToPath } from 'node:url';
import { plugin } from 'bun';
import stub from './audioworklet-stub';

plugin({
  name: 'audioworklet-stub',
  setup(build) {
    build.onResolve({ filter: /\?audioworklet$/ }, () => ({
      path: fileURLToPath(import.meta.resolve('./audioworklet-stub.ts')),
    }));
    // An extensionless relative import of a `.ts` worklet (`./master-worklet?audioworklet`) skips
    // onResolve: bun resolves the file itself and only asks onLoad, with the query kept.
    build.onLoad({ filter: /\?audioworklet$/ }, () => ({
      contents: `export default ${JSON.stringify(stub)};`,
      loader: 'js',
    }));
  },
});
