/**
 * Names in the app's eval scope (`src/lib/strudel/prebake.ts`) that the headless host's scope
 * does not have, each with the reason. A name leaves this list only by gaining a shim
 * (`src/host/shims.ts`); `tests/host-scope.test.ts` keeps it exact in both directions.
 * Functions of `@strudel/webaudio` are never here: the host stubs them.
 */

const AUDIO_STATE = 'audio state';
const EDITOR_ONLY = 'editor only';

export const NOT_REACHABLE: Record<string, string> = {
  // Non-function exports of @strudel/webaudio (superdough's module state).
  DEFAULT_MAX_POLYPHONY: AUDIO_STATE,
  Warpmode: AUDIO_STATE,
  analysers: AUDIO_STATE,
  analysersData: AUDIO_STATE,
  distortionAlgorithms: AUDIO_STATE,
  maxPolyphony: AUDIO_STATE,
  multiChannelOrbits: AUDIO_STATE,
  noises: AUDIO_STATE,
  soundMap: AUDIO_STATE,
  // Exports of @strudel/codemirror without a shim.
  BlockWidget: EDITOR_ONLY,
  SliderWidget: EDITOR_ONLY,
  StrudelMirror: EDITOR_ONLY,
  Vim: EDITOR_ONLY,
  activateTheme: EDITOR_ONLY,
  addWidget: EDITOR_ONLY,
  basicSetup: EDITOR_ONLY,
  codemirrorSettings: EDITOR_ONLY,
  compartments: EDITOR_ONLY,
  defaultSettings: EDITOR_ONLY,
  extensions: EDITOR_ONLY,
  flash: EDITOR_ONLY,
  flashField: EDITOR_ONLY,
  highlightExtension: EDITOR_ONLY,
  highlightMiniLocations: EDITOR_ONLY,
  initEditor: EDITOR_ONLY,
  initTheme: EDITOR_ONLY,
  injectStyle: EDITOR_ONLY,
  isFlashEnabled: EDITOR_ONLY,
  isPatternHighlightingEnabled: EDITOR_ONLY,
  minimalSetup: EDITOR_ONLY,
  parseBooleans: EDITOR_ONLY,
  registerWidget: EDITOR_ONLY,
  setFlash: EDITOR_ONLY,
  setMiniLocations: EDITOR_ONLY,
  setSliderWidgets: EDITOR_ONLY,
  setWidget: EDITOR_ONLY,
  settings: EDITOR_ONLY,
  showMiniLocations: EDITOR_ONLY,
  sliderPlugin: EDITOR_ONLY,
  theme: EDITOR_ONLY,
  themeColors: EDITOR_ONLY,
  themes: EDITOR_ONLY,
  toggleBlockComment: EDITOR_ONLY,
  toggleBlockCommentByLine: EDITOR_ONLY,
  toggleComment: EDITOR_ONLY,
  toggleLineComment: EDITOR_ONLY,
  updateMiniLocations: EDITOR_ONLY,
  updateSliderWidgets: EDITOR_ONLY,
  updateWidgets: EDITOR_ONLY,
  widgetPlugin: EDITOR_ONLY,
};
