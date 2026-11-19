import { drywet, onceEnded, releaseAudioNode } from './helpers.mjs';
import { getAudioContext } from './audioContext.mjs';
import { makeRandom, onAudioSeedChange, randomFromSeed } from './rng.mjs';

let noiseCache = {};
onAudioSeedChange(() => {
  noiseCache = {};
});

// lazy generates noise buffers and keeps them forever
export function getNoiseBuffer(type, density, voiceSeed) {
  const ac = getAudioContext();
  if (noiseCache[type]) {
    return noiseCache[type];
  }
  const bufferSize = 2 * ac.sampleRate;
  const noiseBuffer = ac.createBuffer(1, bufferSize, ac.sampleRate);
  const output = noiseBuffer.getChannelData(0);
  let lastOut = 0;
  let b0, b1, b2, b3, b4, b5, b6;
  b0 = b1 = b2 = b3 = b4 = b5 = b6 = 0.0;
  // crackle is regenerated per voice, the cached types depend only on seed, type and sample rate
  const random = type === 'crackle' ? randomFromSeed(voiceSeed) : makeRandom('noise', type, ac.sampleRate);

  for (let i = 0; i < bufferSize; i++) {
    if (type === 'white') {
      output[i] = random() * 2 - 1;
    } else if (type === 'brown') {
      let white = random() * 2 - 1;
      output[i] = (lastOut + 0.02 * white) / 1.02;
      lastOut = output[i];
    } else if (type === 'pink') {
      let white = random() * 2 - 1;
      b0 = 0.99886 * b0 + white * 0.0555179;
      b1 = 0.99332 * b1 + white * 0.0750759;
      b2 = 0.969 * b2 + white * 0.153852;
      b3 = 0.8665 * b3 + white * 0.3104856;
      b4 = 0.55 * b4 + white * 0.5329522;
      b5 = -0.7616 * b5 - white * 0.016898;
      output[i] = b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362;
      output[i] *= 0.11;
      b6 = white * 0.115926;
    } else if (type === 'crackle') {
      const probability = density * 0.01;
      if (random() < probability) {
        output[i] = random() * 2 - 1;
      } else {
        output[i] = 0;
      }
    }
  }

  // Prevent caching to randomize crackles
  if (type !== 'crackle') noiseCache[type] = noiseBuffer;
  return noiseBuffer;
}

// expects one of noises as type
export function getNoiseOscillator(type = 'white', t, density = 0.02, voiceSeed) {
  const ac = getAudioContext();
  const o = ac.createBufferSource();
  o.buffer = getNoiseBuffer(type, density, voiceSeed);
  o.loop = true;
  o.start(t);
  return {
    node: o,
    stop: (time) => o.stop(time),
  };
}

export function getNoiseMix(inputNode, wet, t) {
  const noiseOscillator = getNoiseOscillator('pink', t);
  const noiseMix = drywet(inputNode, noiseOscillator.node, wet);
  onceEnded(noiseOscillator.node, () => {
    releaseAudioNode(noiseOscillator.node);
  });
  return {
    node: noiseMix.node,
    stop: (time) => noiseOscillator?.stop(time),
    teardown: noiseMix.teardown,
  };
}
