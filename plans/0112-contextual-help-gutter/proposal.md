# Contextual help for Strudeland controls

## Goal

Make the consequence of an action legible before the user takes it. A single Help area for the active view answers what the control does, what it affects, and, when relevant, why it is unavailable. Hover and keyboard focus surface the same explanation. Start with the piano roll, where a user can edit a single use of a pattern or switch to the definition that other uses share.

| Piano roll action | Proposed help |
| --- | --- |
| Edit shared definition | Show the original pattern in the piano roll. Edits can affect every use of it. |
| Back to selection | Return to the pattern or occurrence you were viewing before opening the shared definition. |
| Previous bars, on the first page | You’re already viewing the first bars of this pattern. |

[PianoRoll.svelte](../../src/components/PianoRoll.svelte) derives the definition, its known uses, and the previous view from the current editor context. The shared-definition action changes the roll view. A subsequent edit can affect other uses, including uses the roll cannot enumerate. Back restores the prior roll context. It does not undo edits. The help belongs to the corresponding buttons and should describe their existing behavior. [Caret-driven piano roll editor contract](../0111-caret-driven-piano-roll/editor-experience.md)

## Scope

Cover existing app-owned WebView controls across project, editor, transport, recording, export, and instrument workflows. Include controls inside dialogs and menus, generated rows, keyboard-composite controls, controls with state-sensitive copy, and the authored CodeMirror widgets. Build the authoring API on the shared primitives and document equivalent markup for custom elements and imperative widgets. Keep action behavior, shortcuts, focus order, and existing browser semantics. Use the same Help treatment inside modal dialogs.

Native OS menu highlighting, selectors’ OS-rendered option popups, and native dialogs require platform integration. The WebView trigger or field still receives help. Existing Strudel function documentation and error/status messages have their own jobs and remain available. Introducing an app theme switcher or redesigning the general toolbar and timeline is outside this task.

This is **large and cross-cutting**. Keep fracture, singletonize, and loss check. No new dependency is expected. The application currently ships one dark theme and fixes the editor to the Strudeland theme. Use semantic design tokens and verify the applicable high-contrast/forced-colors behavior where the host supports it. [Theme tokens](../../src/styles/tokens.css) · [Editor configuration](../../src/lib/editor/setup.ts)

## Layout and prototype

Place the workspace Help strip directly above the status bar in the editing column on regular-height windows. The strip aligns with the timeline fields and leaves the two existing editing widths intact. Reserve two lines plus padding, approximately 48 CSS px at the current text settings. Use the app’s 13 px UI type, left-aligned body text, a quiet “Help” label, and the existing surface, border and text tokens. The idle message can teach both entry points with “Hover over a control or move keyboard focus to see what it does.” Avoid animations as focus moves.

For a short window with a project sidebar, use a reserved Help area at the bottom of the existing 200 px sidebar. This gives the roll more room and makes Help visible beside the workspace. Keep the reservation constant during hover, focus and content updates. The Welcome view has no sidebar, so use the horizontal strip there. Start with a height breakpoint near 560 CSS px and check short-wide and tall-narrow windows before settling the breakpoint. Use a stable layout slot and allow the sidebar lists to scroll with the Help area present.

The three WebView dialogs use `showModal()`. Give each a Help slot inside the dialog at its bottom, outside the scrolling content and without moving form actions as help changes. In Export Audio, use the dialog form’s existing body/footer split. Instrument Browser needs its scrollable content above a fixed Help slot. Reserve enough space for additional wrapping in the narrower New Project dialog. Only the active surface displays the current message. [App shell](../../src/App.svelte) · [Export Audio](../../src/components/ExportAudio.svelte) · [Instrument Browser](../../src/components/InstrumentBrowser.svelte) · [New Project](../../src/components/NewProject.svelte) · [HTML modal dialog and inertness](https://html.spec.whatwg.org/multipage/interaction.html#modal-dialogs-and-inert-subtrees)

### Prototype findings

Loaded the current Svelte app in Chromium with stubbed desktop IPC and a valid song with two uses of one definition. Injected temporary presentation and event code into the page to keep the app sources intact during the comparison. The two sizes come from [Tauri window configuration](../../src-tauri/tauri.conf.json).

| Variant | 1280 × 800 | 720 × 480 |
| --- | --- | --- |
| 48 px bottom strip | Editor stayed 1,079 px wide, height fell from 282 to 234 px, and the roll stayed 305 px tall. | Editor stayed 519 px wide. In this capture the roll fell from about 153 to 111 px, leaving very little visible grid. |
| 176 px right rail | Editor and roll width fell from 1,079 to 903 px. | Width fell from 519 to 343 px and roll toolbar height grew from 40 to 68 px. |
| Existing sidebar, 112 px reserved at bottom | Workspace dimensions matched the immediately preceding baseline. | Workspace dimensions matched the immediately preceding baseline. The shared-definition copy fit in four lines. Sidebar lists give up part of their height. |

The spike verified the first action on hover, the return action via real Tab navigation from the roll toolbar, focused help as the pointer moved to blank space, a disabled Previous bars hover, stable editor/roll bounds while switching help text, and a Help footer in the open Instrument Browser at both sizes. Reviewed captures at both sizes. Activating Edit shared definition with Enter moved focus to the document body at both sizes. The current theme’s text and dim-text tokens measure about 14.8:1 and 7.0:1 against the gutter surface. Browser and native WebView behavior can differ for disabled controls. The spike used minimal event logic and stubs, with native screen-reader behavior, production copy fit, all three production dialog layouts, and accessibility zoom still to verify.

## Architecture and technical design

### Authoring and accessibility

- Add a reactive optional `help` prop to [Button](../../src/components/ui/Button.svelte), [Field](../../src/components/ui/Field.svelte), [MenuItem](../../src/components/ui/MenuItem.svelte), [ListRow](../../src/components/ui/ListRow.svelte) and [Link](../../src/components/ui/Link.svelte). Callers derive a concise plain-text string from the same state as the action. A `Field` labels and describes the actual input. Its label and hit area can resolve to that same target on hover.
- Share a target binding with raw HTML controls and imperative widgets. Bind the exact focusable control and optionally its existing hit area or label. The same text drives the visible gutter and a control-owned accessible description. Give description nodes stable IDs for their mount lifetime, merge any existing `aria-describedby` IDs, and keep names, roles, values, selection state, and error descriptions intact. Put description nodes in valid markup, including for controls inside forms, lists and menus. Avoid a shared description node whose content changes due to unrelated pointer motion. [WAI-ARIA descriptions](https://www.w3.org/TR/wai-aria-1.2/#aria-describedby)
- Avoid an always-live gutter. Keyboard and screen-reader navigation can get the description from the control before activation. Pointer sweeps should generate no stream of screen-reader announcements. Verify how changed copy is exposed when a control retains focus, including after toggling. Keep accessibility announcements for meaningful errors and progress where the app already issues them.
- Replace explanatory `title` text on adopted controls where the description would repeat it. Preserve distinct information, such as a full path in a truncated row, a version fingerprint or the existing function docs. Review child titles when they belong to the same interactive hit area. Keep status, validation and prerequisite instructions visible where users need them.
- Keep native disabled semantics on buttons and fields. For hover, test hit detection over disabled elements and fieldsets independently of click events. If a WebView suppresses events there, give the existing nonfocusable hit area the same binding or resolve it through scope-local hit testing. At minimum, surface a keyboard-readable reason through related existing instructions or the field that resolves the blocker. For example, the export action’s destination validation should explain the same immediate prerequisite that its disabled help gives on hover. For existing focusable disabled menu items, retain the description and guard activation. [Menu keyboard pattern](https://www.w3.org/WAI/ARIA/apg/patterns/menubar/)

### Help ownership and lifecycle

Use one document-level controller with a presentation slot for each active surface. Observe pointer and focus targets through delegated events, with a small pure selection policy for testing. The binding supplies the current text and identity. Track the nearest actual control, including a label’s associated field, the actionable element behind a child icon, and a composite’s active member. An unannotated independent child is a boundary. An icon inside a button is part of that button. Prefer specific help over container help and avoid installing pointer listeners on individual piano-roll notes.

| Input or change | Help behavior |
| --- | --- |
| Tab, Shift+Tab, an existing composite-navigation key, or focus moved by a control | Show the new focus target. A stationary pointer over a different control does not override it. |
| Mouse or pen deliberately enters a new control | Show that control. Small movements within one control do not reselect it. |
| Pointer leaves a control for blank space | Restore current valid focus help, or idle when there is no focus target. |
| Focus leaves the active control | Re-evaluate the valid pointer candidate, or show idle. Avoid an intermediate idle flash when focus moves within the same control. |
| Open/close a modal or switch song, project, or roll view | Invalidate candidates from the old surface. Use the new view’s actual focus, including restored focus after closing a dialog. Ignore retained hover until a fresh pointer target is observed. |
| Active content changes, is disabled, or is removed | Read the current help, reconsider focus/hover validity, and fall back or clear when the target disappears. |
| Window blur, document hidden, or an OS dialog takes over | Discard stale candidates. Re-establish help from legitimate focus or pointer input on return. |
| Pointer leaves the WebView while it retains focus | Discard hover and retain valid focus help. |

Do not depend on `pointerleave` or `blur` being sent when an element is removed, or on an event continuing to target the element underneath a drag with pointer capture. Bound invalidation to known view changes and registered control changes. Treat pane scrolling, menu closure, DOM replacement and pointer capture release as test cases. Avoid global render work on every pixel of movement or meter tick. Test help behavior when pointer movements cross other controls and where keyboard selection uses `aria-activedescendant`. Preserve Escape for the dialog, menu, or editor that owns it.

### CodeMirror and native boundaries

Use [Editor setup](../../src/lib/editor/setup.ts) for the editor content surface and imperative registration. Give editor content brief help for writing/evaluating, using actual platform modifier labels. Treat suggestions as a keyboard composite with existing option selection. Evaluate CodeMirror’s `selectedCompletion` and `completionStatus` APIs for representing the selected suggestion, with editor-owned documentation remaining in its existing popup. Hover help for an option should describe accepting that suggestion. Loading, informational function tooltips and noninteractive visualization widgets are not additional action targets.

The authored `slider()` in [the vendored slider widget](../../vendor/strudel/packages/codemirror/slider.mjs) renders a range input under an `aria-hidden` wrapper. Verify keyboard reachability and its accessible name with the real widget. It needs a valid accessible description of what moving the authored value changes. First evaluate a small Strudeland-side ViewPlugin adapter scoped to editor widgets. If an upstream patch is needed, record it in `PATCHES.md` and run the vendor-integrity checks. Preserve slider source updates, live value updates and bounds.

[Tauri’s File menu](../../src-tauri/src/project_menu.rs) sends activation events for New Project, Open Project and Export Audio, and separately updates whether Export Audio is enabled. Tauri’s menu event exposes the activated item ID. It supplies no hover or focus highlight to this bridge. macOS also supplies predefined platform menus. Native folder pickers, unsaved and overwrite prompts, and microphone permission UI are outside the WebView DOM. A live native menu gutter would require each platform’s highlight lifecycle and a readable WebView while the native menu is active. Document the boundary, including that Export Audio originates in File, and keep native controls on their existing platform paths. [Tauri MenuEvent](https://docs.rs/tauri/latest/tauri/menu/struct.MenuEvent.html) · [Project dialog calls](../../src/lib/project/api.ts) · [Export native integration](../../src-tauri/src/export_audio.rs)

## Control inventory

Audit each group on the branch during implementation, including rarely rendered states. Record coverage by control family and state. Each app-owned interactive WebView element should get help through a primitive, a custom binding, or its owning composite.

| Surface | Controls and state cases |
| --- | --- |
| [Welcome](../../src/components/Welcome.svelte), [songs](../../src/components/SongList.svelte), [creation](../../src/components/NewSong.svelte), [conflict](../../src/components/ConflictBar.svelte) | Recent projects, create/open, file name entry and keyboard submit/cancel, song row, instruments opener, keep editor or disk copy, recreate deleted file, close deleted file. |
| [Toolbar](../../src/components/Toolbar.svelte), [status bar](../../src/components/StatusBar.svelte), [master meter](../../src/components/ui/MasterMeter.svelte) | Evaluate/play versus stop, roll toggle, note audition, format on save, Vim, restore suspended audio, dismiss notices, external links, limiter toggle and clip reset. Scopes and readouts display information. |
| [Timeline](../../src/components/Timeline.svelte), [section block](../../src/components/SectionBlock.svelte) | Section triggers, start-from, loop section and range, range end/back, stop looping, ending and metadata edits. Distinguish quarter-note BPM, time signature, section bars and effect on next evaluation. |
| [Piano roll](../../src/components/PianoRoll.svelte), [grid](../../src/components/PianoRollGrid.svelte) | Expression versus In song, meter anchor, paging, definition/return/use links, preview and occurrence actions, snap, insert/select, audition keys, grid placement, note movement/resize/delete and read-only states. Notes, pitch keys and cells share behavior with scope-specific copy where needed. |
| [Recording](../../src/components/RecordControls.svelte), [device picker](../../src/components/DevicePicker.svelte), [memo list](../../src/components/MemoList.svelte) | Record/stop, song backing, metronome and count-in, device/channel/refresh, memo playback and variants, rename entry, delete/cancel and state-dependent unavailable actions. |
| [New Project](../../src/components/NewProject.svelte) | Name, cancel, create, and missing-name prerequisite. |
| [Instrument Browser](../../src/components/InstrumentBrowser.svelte) | Source switch, search, keyboard row navigation, refresh, preview/stop, pin/update/apply, close, loading and selection prerequisites. |
| [Export Audio](../../src/components/ExportAudio.svelte) | Preset radios, mix/stem grouping, format, range/section/length, filename/destination/default path controls, preview refresh, submit and close, progress/quit choices, final dismissal, and reason submit is unavailable. |
| [CodeMirror](../../src/components/Editor.svelte) | Editor surface, generated completion choices, authored sliders and any interactive elements in widgets. Existing function documentation remains separately discoverable through CodeMirror behavior. |

## Dependencies, delivery and validation

1. Settle placement and interaction with the prototype, then build the presentation slot and target binding. Exercise the two piano-roll actions, a stateful preference, the timeline BPM field, a menu item, the custom clip-reset or grid control, and a keyboard-readable unavailable reason before annotating the rest of the app.
2. Apply the mechanism surface by surface using the inventory. Keep examples in contributor guidance that show a primitive, derived help, and a custom control. Document native limitations and the additional work needed for native menu highlighting.
3. Test the pure ownership rules and the actual bindings through the app. Cover fast transitions, field labels and icons, nested controls, disabled fieldsets, state changes with retained focus, menu/dialog open and close, CodeMirror options and widgets, song swaps, collapse, scroll, window blur, and detachment. Compare accessible names, roles, descriptions and action results before and after. Exercise focus order and Vim, editor, transport, grid and menu shortcuts. Confirm enabled and disabled controls retain submit, click, focus and keyboard semantics.
4. Compare editor/roll bounds across idle, short and long help, and verify wrapping at the default and minimum sizes, short-wide and tall-narrow windows, zoom and increased text spacing. Open all three dialogs and verify both the help and the action footer remain visible. Review captures of the real desktop app via the repository’s screenshot command and check logs. Verify descriptions with VoiceOver in WKWebView, including the first two roll controls, the toggle, the menu item, the unavailable reason and the editor widget. Run equivalent keyboard and native hit-testing smoke checks on WebView2 and WebKitGTK where available and record actual coverage. A browser accessibility snapshot verifies markup and descriptions. It cannot confirm speech behavior.
5. Run `bun run build`, `bun test`, `bun run check` and `bun run lint` for the completed frontend. Add targeted native validation if native code changes. [Contributor verification](../../CONTRIBUTING.md#verify-your-changes)

## Risks and open questions for refinement

- **Width, height and prominence.** The proposed short-window sidebar slot gives up 112 px of list space. Review it with long song and memo lists, a dense timeline and modal bottom actions. The prototype’s horizontal strip works at regular height. Pick the final breakpoint and text budget based on rendered content. Place essential warnings in the existing inline surface as well when they must remain visible regardless of focus.
- **Pointer access to the distant strip.** The acceptance request favors clearing stale hover help when leaving a control. W3C guidance also calls for moving the pointer over added hover content without losing it. Test moving directly onto the gutter and across blank space at magnified sizes. Refine whether a short cancellable pointer-exit grace period is enough, or whether the user needs a way to hold a message while moving to it. Preserve immediate clearing on view change or whenever information becomes invalid. The prototype establishes geometry and basic interaction and makes no conformance determination. [WCAG hover or focus guidance](https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html)
- **Focus after the shared-definition action.** The two buttons replace one another. Chromium moved focus to the document body after keyboard activation. Confirm that result in the native WebView. The proposal is to focus the replacement when the removed button held focus, so the user can continue from the same place. Settle that behavior in refinement and preserve the surrounding tab order.
- **Reading unusual copy and AT announcements.** Test real device labels, long project data, and worst-case help with larger text. Keep secondary details in their existing visible locations and check that all essential explanations fit without changing work-area dimensions. Check state changes under VoiceOver with an existing field error, a toggle that keeps focus and a menu that restores focus.
