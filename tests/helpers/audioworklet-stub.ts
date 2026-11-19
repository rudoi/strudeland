// Stands in for superdough's `./worklets.mjs?audioworklet` import under bun test. Vite inlines the
// real worklet bundle as a data URL; tests never run audio worklets, so an empty one will do.
export default 'data:text/javascript;base64,';
