# 0123 loss check

The single stage covers the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| `src/lib/shell.ts`: state store, `quitting` getter, one modal slot, the gate table (`allowed`, plus the private `can`), every verb, `onQuit` in today's order, `canCloseWindow`, `start`/`destroy` | 001 |
| `src/lib/shell-ports.ts` (wraps the existing adapters only), `tests/helpers/fake-shell-ports.ts`, the call-through port test | 001 |
| Export Audio sync batched per microtask; instrument browser dropped on root change | 001 |
| App.svelte renders from `shellStore` and keeps focus, `onGesture`, shortcut parsing and help | 001 |
| 17 shell test cases; existing controller and session tests unchanged | 001 |
| AGENTS.md layout line; GLOSSARY "App shell" | 001 |
| No visible change (menu enablement, shortcuts, quit and window-close outcomes, focus, copy) | 001 (tests, plus the user's manual check in the PR) |

Follow-up filed: #152 (whether window close should behave like Quit during an export).
