# App icon design

Implemented and visually approved. The final desktop assets reproduce from the outlined master, and the aligned ICNS was verified inside the signed macOS debug bundle.

Use a lowercase `<s>` monogram. Set the letter in the bundled JetBrains Mono Variable at weight 800 and the brackets at 750. The plate uses `--bg`, the letter uses `--accent`, and the brackets use `--text`. A thin edge uses `--border-strong`.

Place the centered mark on an inset rounded square with transparent corners and canvas padding. Align the visible vertical centers of the three glyphs on the tile's centerline. Keep the glyph outlines in the [vector master](../../src-tauri/icons/app-icon.svg), with source identifiers for the font and theme. Export the desktop assets using the pinned Tauri CLI. Review the 16 and 32 px renderings and both light and dark contexts before accepting the spacing. Validate the macOS bundle resource, Windows ICO frames, and Ubuntu PNG dimensions.
