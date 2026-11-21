/** Fixed pitched synths and their aliases in Superdough's registerSynthSounds. */
export const auditionSounds: readonly string[] = [
  'sine',
  'sin',
  'triangle',
  'tri',
  'square',
  'sqr',
  'sawtooth',
  'saw',
  'supersaw',
  'pulse',
];

export function isAuditionSound(sound: string): boolean {
  return auditionSounds.includes(sound.toLowerCase());
}
