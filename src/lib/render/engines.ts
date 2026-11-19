/**
 * The engines that can render for the harness and what differs between them, in one place.
 * Pure: no imports.
 */

/** Which engine renders: superdough in Chromium's `OfflineAudioContext`, or the Rust engine
 *  (the `strudeland-engine` binary). */
export type EngineName = 'superdough' | 'rust';

export interface EngineTraits {
  /** Renders are deterministic: twins are compared bit for bit whatever the fixture. */
  deterministic: boolean;
  /** Can refuse haps it does not implement; corpus.json's `rust` field is checked against what
   *  it detects. */
  refusesHaps: boolean;
  /** Its renders may write goldens (`--update-goldens`). */
  writesGoldens: boolean;
  /** The `PROFILES` entry its renders are judged with against the superdough goldens. */
  parityProfile: 'self' | 'engine';
}

export const ENGINE_TRAITS: Readonly<Record<EngineName, EngineTraits>> = {
  superdough: {
    deterministic: false,
    refusesHaps: false,
    writesGoldens: true,
    parityProfile: 'self',
  },
  rust: {
    deterministic: true,
    refusesHaps: true,
    writesGoldens: false,
    parityProfile: 'engine',
  },
};
