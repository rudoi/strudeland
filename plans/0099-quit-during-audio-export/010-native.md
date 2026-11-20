# Native application Quit

## Goal

Intercept AppKit termination and keep the process alive while the main window decides.

## Prerequisites

Read the native lifecycle in the design and inspect the locked Tao delegate.

## Implementation

Register one guarded selector, channel subscription and request/reply flow. Invalidate pending
requests on navigation, keep a single pending request, and hold the existing export permit
during approved shutdown.

## Validation

Test stale replies, cancellation, a delivery failure and approval while an export owns its
permit. Compile the macOS app and smoke the AppKit entry point with stage 020.

## Commit message

`feat(export): defer native quit until the app decides`
