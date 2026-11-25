const SHARP = ['c', 'c#', 'd', 'd#', 'e', 'f', 'f#', 'g', 'g#', 'a', 'a#', 'b'];
const FLAT = ['c', 'db', 'd', 'eb', 'e', 'f', 'gb', 'g', 'ab', 'a', 'bb', 'b'];

/** Spell an integral MIDI pitch, preserving the source's case and accidental convention. */
export function spellNote(pitch: number, old = ''): string {
  const octave = Math.floor(pitch / 12) - 1;
  const style = /^[a-g][bf]/i.test(old) ? FLAT : SHARP;
  let name = style[((pitch % 12) + 12) % 12];
  if (old[1] === 's') name = name.replace('#', 's');
  if (old[1]?.toLowerCase() === 'f' && name.length > 1) name = name[0] + 'f';
  if (/^[A-G]/.test(old)) name = name[0].toUpperCase() + name.slice(1);
  return name + octave;
}
