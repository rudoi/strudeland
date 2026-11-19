import { Annotation, RangeSetBuilder, StateField } from '@codemirror/state';
import type { EditorState } from '@codemirror/state';
import { Decoration, EditorView } from '@codemirror/view';
import type { DecorationSet } from '@codemirror/view';

/** Annotate a transaction with `true` to mark the lines it changes as a Claude edit. */
export const claudeEdit = Annotation.define<boolean>();

const lineMark = Decoration.line({ class: 'cm-claude-edit' });

/** Line highlights for an annotated edit; cleared by the next ordinary doc change. */
export const claudeMarkField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    if (!tr.docChanged) return value;
    if (!tr.annotation(claudeEdit)) return Decoration.none;
    const doc = tr.state.doc;
    const builder = new RangeSetBuilder<Decoration>();
    let last = 0; // highest line number already marked
    tr.changes.iterChanges((fromA, toA, fromB, toB, inserted) => {
      // An insertion starting with a line break at the end of a line leaves that line untouched.
      const startLine = doc.lineAt(fromB);
      const keepsStart =
        fromA === toA && fromB > startLine.from && inserted.sliceString(0, 1) === '\n';
      const start = keepsStart ? startLine.number + 1 : startLine.number;
      // An insertion ending in a line break ends at the start of the next, untouched line.
      const endLine = doc.lineAt(toB);
      const end = toB > fromB && endLine.from === toB ? endLine.number - 1 : endLine.number;
      for (let n = Math.max(start, last + 1); n <= end; n++) {
        builder.add(doc.line(n).from, doc.line(n).from, lineMark);
        last = n;
      }
    });
    return builder.finish();
  },
});

export const claudeMark = [claudeMarkField, EditorView.decorations.from(claudeMarkField)];

export function hasClaudeMark(state: EditorState): boolean {
  return (state.field(claudeMarkField, false)?.size ?? 0) > 0;
}
