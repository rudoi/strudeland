# Help interaction

The [proposal](proposal.md) specifies the precedence and lifecycle. Keep one controller for the WebView and a Help slot for each view or modal. Each control has its own stable accessible description. The visual help is not a live region and can follow pointer intent without changing unrelated descriptions.

The shared primitives take a reactive plain-text `help` prop. Custom rendered elements use the same `helpTarget` Svelte action with their current text. Imperative widgets can use the corresponding registration with an update and cleanup handle. Attach to the actual control, resolving a native label or child content to its control for hover. Keep existing description IDs and native disabled semantics.

The policy distinguishes focus and hover candidates. A new intentional hover takes precedence over focus, while actual focus or keyboard navigation wins over a stationary pointer. Moving onto background restores current focus help. Scope invalidation clears stale candidates on switch, unmount, modal transitions and window focus loss. During pointer capture, allow the captured control to retain its help and refresh on release. Use the active modal scope and clear the workspace slot while it owns focus.

Use scoped delegated listeners and narrowly scoped DOM observation where necessary for detachment, state changes, and pre-existing accessibility attributes. Cover transitions with pure policy cases and browser validation of actual bindings. Use the same mechanism for the two piano-roll actions, native-disabled hover, a label and its input, and an imperative editor widget. [Control inventory](coverage.md)

## Binding interfaces

`src/lib/help/targets.ts` exports `helpTarget` for Svelte elements and `registerHelpTarget(element, text)` for editor code. The controller exposes the current target, its scope and message through a readable store, and installs document listeners for the lifetime of App. `src/components/ui/Help.svelte` infers its modal scope from its containing dialog. The `compact` prop selects the workspace arrangement. A workspace layout transition may call `resetHelp()` after changing its view key.
