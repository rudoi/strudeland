# 0116 loss check (light)

## Preserved

- **The research's decisions 1-6.** Decision 3 was corrected.
- **Refine, must fix 1-6:**
  1. the exit rule is narrowed to bad-request plan errors, and a bad `--loops` value becomes a
     bad request;
  2. `strudeland-render`'s shared `exit_class` is untouched;
  3. every plan-request JSON literal is stripped;
  4. the `host-entry` error rows;
  5. the unused imports;
  6. every Rust literal is named.
- **Refine, nice to have 7-9:** the docs, the wording, and the "invalid request:" prefix tests.

## Modified (justified)

- **Decision 3's rule changed from "every MetaFlag → 2" to "bad request → 2".** Song-dependent
  flagged errors must stay host errors.

## Unintentionally dropped

- None.
