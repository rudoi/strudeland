# Typographic app icon

## Goal

Give Strudeland a recognizable desktop icon using the same type and colors as its editor. Build the artwork from glyph outlines and simple geometry by hand.

## Scope

- Replace the desktop icon assets consumed by macOS, Windows, and Ubuntu packages.
- Keep one editable master and generate the platform assets from it. Keep the app name, in-app wordmark, and package identifiers in their current roles.
- Generative artwork is outside scope.

## Architecture

Use the bundled JetBrains Mono face from [`src/styles/fonts.css`](../../src/styles/fonts.css) and the accent, text, and background colors in [`src/styles/tokens.css`](../../src/styles/tokens.css). Put the approved design in a square vector master with glyphs converted to paths so its appearance is independent of fonts installed during packaging. Use the pinned Tauri icon command to produce the PNG, ICNS, and ICO assets referenced by [`src-tauri/tauri.conf.json`](../../src-tauri/tauri.conf.json). Inspect the generated files before replacing the desktop assets in [`src-tauri/icons/`](../../src-tauri/icons/).

Load the exact bundled JetBrains Mono Variable face for the mark and resolve colors from the
theme stylesheet before exporting. Record the originating token names and glyph weights with the
master so the flattened assets can be reproduced.

## Technical design

- **Recommended `<s>`**. Center a heavy lime lowercase `s` between off-white chevrons on a near-black tile. The letter identifies Strudeland and the chevrons evoke Strudel's cycle alternation notation. It can also read as an HTML tag, so compare it at actual icon size. [Strudel mini-notation](https://strudel.cc/learn/mini-notation/#angle-brackets)
- **Alternative `s()`**. Use the sound function's short name and parentheses. It reads as a call and has a broader silhouette. [Vendored sound control](../../vendor/strudel/packages/core/controls.mjs)
- **Alternative `$:`**. Use the live pattern label. Its two glyphs leave space at 16 pixels and have a terminal-like appearance. [Repository pattern example](../../tests/render/corpus/fx-mod.js)
- Compare the playback treatment (`--accent` letter, `--text` brackets) with the editor treatment
  (`--syntax-fn` letter, `--syntax-punct` brackets), both on `--bg`. The editor treatment borrows the
  function and punctuation colors from the [CodeMirror theme](../../src/lib/editor/theme.ts).
- Refine glyph weight, side bearings, and optical centering at 16, 32, and 128 pixels. Inspect the exported transparent corners and contrast on light and dark surfaces. Confirm the packaged macOS icon in Finder or the Dock and verify the Windows ICO and Ubuntu PNG assets.

## Dependencies

The bundled JetBrains Mono font, the existing theme colors, and the repository's Tauri CLI and packaging paths.

## Risks

Small platform icons can lose the separation between glyphs. Check the actual exported sizes and adjust spacing or weight if strokes merge. The OS can cache the existing icon, so use an isolated freshly built package for visual verification.

## Open questions

Choose the mark and color treatment during refinement. The recommended starting point is `<s>`
in the playback treatment because the letter and brackets stay distinct at small sizes.
