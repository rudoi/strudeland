/** Half-open UTF-16 range in the inspected editor snapshot. */
export interface SourceSpan {
  from: number;
  to: number;
}

export interface RollNote {
  id: string;
  /** Exact cycle edges, in Fraction.toFraction() format. */
  start: string;
  end: string;
  pitch: number;
  visibleStart?: string;
  visibleEnd?: string;
  origin?: SourceSpan;
  contributors?: readonly SourceSpan[];
  sound?: string;
  sourceMatch?: boolean;
  pitchEditable?: boolean;
  timeEditable?: boolean;
}

export interface RollPhrase {
  notes: readonly RollNote[];
  bars: number;
  sound?: string;
  /** True when the authoring grammar can insert notes into this phrase. */
  insertable: boolean;
}

export interface RollUse {
  span: SourceSpan;
  sections: readonly string[];
}

export interface RollDefinition {
  name: string;
  expression: SourceSpan;
  phrase?: RollPhrase;
  uses: readonly RollUse[];
  incomplete: boolean;
}

export interface RollTarget {
  expression: SourceSpan;
  label: string;
  phrase?: RollPhrase;
  definition?: RollDefinition;
  sections: readonly string[];
}

export type RollEdit =
  | { kind: 'pitch'; id: string; pitch: number }
  | { kind: 'move'; id: string; start: string }
  | { kind: 'resize'; id: string; end: string }
  | { kind: 'delete'; id: string }
  | { kind: 'insert'; start: string; end: string; pitch: number };

export interface RollPatch extends SourceSpan {
  insert: string;
}

export interface RollBar {
  cycle: number;
  label: string;
  numerator: number;
  denominator: number;
}

export interface InspectionRequest {
  code: string;
  /** The native boundary resolves pins for this open project. */
  root: string;
  view: 'expression' | 'song';
  expression?: SourceSpan;
  from: number;
  to: number;
  anchorBar?: number;
}

export interface InspectionResult {
  notes: readonly RollNote[];
  truncated: boolean;
}
