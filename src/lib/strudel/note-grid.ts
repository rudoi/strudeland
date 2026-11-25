import { note, noteToMidi, silence, stepcat } from '@strudel/core';

/** UTF-16 offsets are relative to the row's string value. */
export interface GridCell {
  text: string;
  offset: number;
}

export interface GridRun {
  start: number;
  end: number;
  tone?: { spelling: string; midi: number; bend: number; attack: number };
}

export interface GridScore {
  width: number;
  bars: number;
  cells: GridCell[];
  runs: GridRun[];
}

/** A row is one logical cycle. Scan the flattened cells so a hold can cross a row boundary. */
export function decodeNoteGrid(rows: readonly string[]): GridScore {
  if (!Array.isArray(rows) || !rows.length) throw new Error('noteGrid needs at least one row');
  const cells: GridCell[] = [];
  let width = 0;
  for (const [index, row] of rows.entries()) {
    if (typeof row !== 'string') throw new Error(`noteGrid row ${index + 1} must be a string`);
    const slots = Array.from(row.matchAll(/\S+/gu), (match) => ({
      text: match[0],
      offset: match.index!,
    }));
    if (!slots.length) throw new Error(`noteGrid row ${index + 1} is empty`);
    if (!index) width = slots.length;
    if (slots.length !== width) throw new Error(`noteGrid row ${index + 1} needs ${width} slots`);
    cells.push(...slots);
  }

  const runs: GridRun[] = [];
  let previous: number | undefined;
  for (let start = 0; start < cells.length;) {
    let end = start + 1;
    while (end < cells.length && cells[end].text === '_') end++;
    const token = cells[start].text;
    const run: GridRun = { start, end };
    if (token !== '_' && token !== '~') {
      const match = /^([a-gA-G][#bsf]*-?\d+)(?::(\d+(?:\.\d+)?|\.\d+))?$/.exec(token);
      const midi = match ? noteToMidi(match[1]) : NaN;
      const attack = match?.[2] === undefined ? 0.09 : Number(match[2]);
      if (!match || !Number.isInteger(midi) || midi < 0 || midi > 127 || !Number.isFinite(attack)) {
        throw new Error(
          `Invalid noteGrid cell at row ${Math.floor(start / width) + 1}, slot ${(start % width) + 1}: ${token}`,
        );
      }
      const bend = match[2] !== undefined && previous !== undefined ? midi - previous : 0.55;
      run.tone = { spelling: match[1], midi, bend, attack };
      previous = midi;
    }
    runs.push(run);
    start = end;
  }
  return { bars: rows.length, width, cells, runs };
}

/** Construct the same weighted, gliding pattern as the original Luoyuan row consumer. */
export function noteGrid(rows: readonly string[]) {
  const score = decodeNoteGrid(rows);
  return stepcat(
    ...score.runs.map(({ start, end, tone }) => [
      end - start,
      tone ? note(tone.spelling).penv(tone.bend).pattack(tone.attack) : silence,
    ]),
  ).slow(score.bars);
}
