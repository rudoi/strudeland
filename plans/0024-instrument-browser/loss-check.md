# Design comparison

The [proposal](proposal.md) and its refinement are covered by three contracts.

| Requirement | Where it is preserved |
|---|---|
| Browse global definitions and project pins, show kit details, and explicitly pin or update | [Storage and preview](storage-and-preview.md) and [browser and editor](browser-and-editor.md) |
| Audition the chosen version with defaults and keep preview separate from transport | [Storage and preview](storage-and-preview.md) and [browser and editor](browser-and-editor.md) |
| Apply a pin in one conditional, undoable editor transaction | [Browser and editor](browser-and-editor.md) and [automation](automation.md) |
| List, create, pin, update and apply through CLI and MCP | [Automation](automation.md) |

Refinement keeps the general MCP catalog deferred and leaves parameter editing to the
song. The documented source range is zero-based UTF-16 with an exclusive end. The
visible editor selection and automation both use the same operation.
