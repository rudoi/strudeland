# Help coverage

The [proposal inventory](proposal.md#control-inventory) lists each authored WebView surface and states to cover. Mark every control family in that inventory through its primitive, native label association, shared custom action, or keyboard composite. Describe an enabled action in terms of outcome and scope and derive a disabled reason or alternate state from the same condition that controls behavior. Follow the [writing guide](../../docs/writing-contextual-help.md).

Piano roll coverage includes both definition actions, usage links, paging, meters, occurrences, snap, pitch keys, placement, and editable and read-only notes. Give the replacement action focus if activation removes the previously focused action. Do not move focus for an unrelated source or view update. Preserve the selected occurrence and edit semantics.

The CodeMirror content, selected suggestion and authored slider use editor-owned registration. Preserve Strudel function documentation, slider value and source updates, and completion controls. Verify keyboard selection, name and description. Native menu highlighting, system dialogs and OS select popups require distinct platform work. The Settings menu command itself belongs to [workspace integration](workspace-settings.md).

Use DOM and keyboard tests for shared controls, decision logic and fragile behavior. Include pointer and focus arbitration, disabled boundaries, native select labels, state updates and cleanup. Screen-reader speech and desktop layouts require actual desktop validation in addition to browser checks.

## Review entry points

Inspect state guards in `Toolbar.svelte`, `Timeline.svelte`, `ExportAudio.svelte`, `RecordControls.svelte` and `InstrumentBrowser.svelte` before writing disabled copy. CodeMirror uses `createExtensions` in `src/lib/editor/setup.ts`, `EditorView.contentAttributes`, completion-state APIs and the vendored slider class. Preserve autocomplete's documented behavior and treat the slider's parent accessibility attributes as part of the widget integration.
