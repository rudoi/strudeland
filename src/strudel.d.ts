declare module '@strudel/core';
declare module '@strudel/core/fraction.mjs';
declare module '@strudel/mini';
declare module '@strudel/transpiler';
declare module '@strudel/tonal';
declare module '@strudel/draw';
declare module '@strudel/webaudio';
declare module '@strudel/codemirror';
declare module '@strudel/codemirror/basicSetup.mjs';
declare module 'superdough';
declare module 'superdough/rng.mjs';
declare module 'superdough/nodePools.mjs';
declare module 'superdough/superdoughoutput.mjs';
declare module 'vite-plugin-bundle-audioworklet';
/** A Vite `?audioworklet` import: the module bundled into a data URL. */
declare module '*?audioworklet' {
  const url: string;
  export default url;
}
