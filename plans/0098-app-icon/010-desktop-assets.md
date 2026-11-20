# Desktop icon assets

## Goal

Ship the approved typographic icon in Strudeland's macOS, Windows, and Ubuntu packaging.

## Prerequisites

The approved design, bundled JetBrains Mono Variable face, existing theme stylesheet, and pinned Tauri CLI.

## Implementation

Extract the three glyph outlines at the documented weights, compose them in a square vector master, and generate the desktop PNG, ICNS, and ICO files. Keep only desktop assets used by the app and its packaging.

## Validation

Compare outputs from a second generation, inspect 16, 32, and 128 px renders, and confirm that a newly built macOS bundle contains the exact shipped icon.

## Commit

`feat(design): add the Strudeland app icon`
