# App icon design

Use a lowercase `<s>` monogram. Set the letter in the bundled JetBrains Mono Variable at weight 800 and the brackets at 750. The plate uses `--bg`, the letter uses `--accent`, and the brackets use `--text`. A thin edge uses `--border-strong`.

Place the centered mark on an inset rounded square with transparent corners and canvas padding. Keep the glyph outlines in the [vector master](../../src-tauri/icons/app-icon.svg), with source identifiers for the font and theme. Export the desktop assets using the pinned Tauri CLI. Review the 16 and 32 px renderings and both light and dark contexts before accepting the spacing. Validate the macOS bundle resource, Windows ICO frames, and Ubuntu PNG dimensions.
