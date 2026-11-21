# Scope preservation

- Preserved from the proposal are one-call-per-line chains, two-space continuation indentation,
  common arrangement layout, raw Strudel spelling, comment attachment, existing mini rules, and
  one localized save transaction.
- Refined the order of the passes. Establish mini line breaks before resolving JavaScript indentation,
  then finish mini indentation against the JavaScript result. This handles a receiver that moves
  when an earlier mini closes on a different line.
- The contract defines what counts as a chain and how the formatter handles optional access,
  computed access, grouping, comment gaps, invalid headers, and mixed line endings.
- General JavaScript style, a separate command, and per-rule preferences stay outside this release
  as proposed. No approved requirement was dropped.
